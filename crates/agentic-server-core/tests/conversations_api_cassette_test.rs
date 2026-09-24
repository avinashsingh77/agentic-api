//! Compare recorder-generated `OpenAI` and gateway item API histories.
//! Set `REQUIRE_CONVERSATIONS_CASSETTES=1` in CI to require both recordings.

use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Cassette {
    turns: Vec<Turn>,
}

#[derive(Debug, Deserialize)]
struct Turn {
    filename: String,
    request: Request,
    response: RecordedResponse,
}

#[derive(Debug, Deserialize)]
struct Request {
    method: String,
    path: String,
    #[serde(default)]
    query_params: Value,
    #[serde(default)]
    body: Value,
}

#[derive(Debug, Deserialize)]
struct RecordedResponse {
    status_code: u16,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    sse: Option<Vec<String>>,
}

fn cassette(name: &str, provider: &str) -> Option<Cassette> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/cassettes/conversations")
        .join(format!("conversations-{name}-{provider}.yaml"));
    if !path.exists() {
        return None;
    }
    Some(
        serde_yaml::from_str(&std::fs::read_to_string(&path).expect("read cassette"))
            .expect("parse flat recorder-generated cassette"),
    )
}

fn pair(name: &str) -> Option<(Cassette, Cassette)> {
    let openai = cassette(name, "openai");
    let gateway = cassette(name, "gateway");
    if openai.is_none() && gateway.is_none() && std::env::var_os("REQUIRE_CONVERSATIONS_CASSETTES").is_none() {
        eprintln!("{name}: recordings unavailable; run record_conversations_api_cassettes.sh");
        return None;
    }
    Some((
        openai.expect("missing OpenAI cassette"),
        gateway.expect("missing gateway cassette"),
    ))
}

#[derive(Default)]
struct IdPairs {
    forward: HashMap<String, String>,
    reverse: HashMap<String, String>,
}

impl IdPairs {
    fn compare(&mut self, a: &str, b: &str, location: &str) {
        if let Some(previous) = self.forward.insert(a.to_owned(), b.to_owned()) {
            assert_eq!(previous, b, "{location}: one OpenAI ID mapped to two gateway IDs");
        }
        if let Some(previous) = self.reverse.insert(b.to_owned(), a.to_owned()) {
            assert_eq!(previous, a, "{location}: two OpenAI IDs mapped to one gateway ID");
        }
    }
}

fn is_id_field(key: &str) -> bool {
    matches!(
        key,
        "id" | "conversation"
            | "conversation_id"
            | "previous_response_id"
            | "first_id"
            | "last_id"
            | "after"
            | "call_id"
    )
}

fn compare_value(a: &Value, b: &Value, path: &str, ids: &mut IdPairs, generated_text: bool) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len(), "{path}: field count differs");
            for (key, av) in a {
                let bv = b.get(key).unwrap_or_else(|| panic!("{path}: missing field {key}"));
                if key == "model" || key == "created_at" || key == "updated_at" || key == "usage" {
                    assert_eq!(
                        std::mem::discriminant(av),
                        std::mem::discriminant(bv),
                        "{path}.{key}: type differs"
                    );
                } else if is_id_field(key) && av.is_string() && bv.is_string() {
                    ids.compare(av.as_str().unwrap(), bv.as_str().unwrap(), &format!("{path}.{key}"));
                } else if generated_text && key == "text" && av.is_string() && bv.is_string() {
                    // Model wording varies. History visibility is asserted separately.
                } else {
                    compare_value(av, bv, &format!("{path}.{key}"), ids, generated_text);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}: item count differs");
            for (i, (av, bv)) in a.iter().zip(b).enumerate() {
                compare_value(av, bv, &format!("{path}[{i}]"), ids, generated_text);
            }
        }
        _ => assert_eq!(a, b, "{path}: values differ"),
    }
}

fn compare_path(a: &str, b: &str, ids: &mut IdPairs) {
    let left: Vec<_> = a.split('/').collect();
    let right: Vec<_> = b.split('/').collect();
    assert_eq!(left.len(), right.len(), "request path depth differs");
    for (index, (l, r)) in left.iter().zip(right).enumerate() {
        if index == 3 || index == 5 {
            ids.compare(l, r, "request path");
        } else {
            assert_eq!(*l, r, "request path differs");
        }
    }
}

fn sse_events(response: &RecordedResponse) -> Vec<Value> {
    let mut events = Vec::new();
    for line in response.sse.as_ref().expect("streaming response has no SSE capture") {
        if let Some(data) = line.trim().strip_prefix("data: ") {
            if data != "[DONE]" {
                events.push(serde_json::from_str(data).expect("valid SSE JSON"));
            }
        }
    }
    assert!(!events.is_empty(), "empty SSE recording");
    events
}

fn result_body(response: &RecordedResponse) -> Value {
    if response.sse.is_some() {
        let events = sse_events(response);
        let terminal = events
            .iter()
            .rev()
            .find(|event| {
                matches!(
                    event["type"].as_str(),
                    Some("response.completed" | "response.failed" | "response.incomplete")
                )
            })
            .expect("SSE has no terminal event");
        terminal["response"].clone()
    } else {
        response.body.clone().expect("response body missing")
    }
}

fn lifecycle(response: &RecordedResponse) -> Vec<String> {
    sse_events(response)
        .iter()
        .filter(|event| {
            let kind = event["type"].as_str().unwrap_or_default();
            let is_delta = kind.split('.').next_back() == Some("delta");
            let is_reasoning_item = kind.starts_with("response.output_item.") && event["item"]["type"] == "reasoning";
            !(is_delta || kind.starts_with("response.reasoning_") || is_reasoning_item)
        })
        .filter_map(|event| event["type"].as_str().map(str::to_owned))
        .collect()
}

fn item_shape(item: &Value) -> Value {
    let content = item["content"]
        .as_array()
        .expect("message content array")
        .iter()
        .map(|part| {
            let text = if part["type"] == "output_text" {
                Value::String("<generated>".to_owned())
            } else {
                part["text"].clone()
            };
            json!({"type": part["type"], "text": text})
        })
        .collect::<Vec<_>>();
    json!({
        "id": item["id"],
        "type": item["type"],
        "role": item["role"],
        "status": item["status"],
        "content": content,
    })
}

fn list_shape(body: &Value) -> Value {
    let data = body["data"]
        .as_array()
        .expect("item list data")
        .iter()
        .filter(|item| item["type"] == "message")
        .map(item_shape)
        .collect::<Vec<_>>();
    let first_id = data.first().map(|item| item["id"].clone());
    let last_id = data.last().map(|item| item["id"].clone());
    json!({
        "object": body["object"],
        "data": data,
        "first_id": first_id,
        "last_id": last_id,
        "has_more": body["has_more"],
    })
}

fn response_shape(body: &Value, has_conversation: bool) -> Value {
    let outputs = body["output"]
        .as_array()
        .expect("response output")
        .iter()
        .filter(|item| item["type"] == "message")
        .map(item_shape)
        .collect::<Vec<_>>();
    let mut result = json!({
        "id": body["id"],
        "object": body["object"],
        "status": body["status"],
        "previous_response_id": body["previous_response_id"],
        "output": outputs,
    });
    if has_conversation {
        result["conversation_id"] = if body["conversation"].is_object() {
            body["conversation"]["id"].clone()
        } else {
            body["conversation_id"].clone()
        };
    }
    result
}

fn comparable_body(turn: &Turn) -> Value {
    let body = result_body(&turn.response);
    if turn.request.path == "/v1/responses" && turn.response.status_code == 200 {
        return response_shape(&body, turn.request.body.get("conversation").is_some());
    }
    if turn.request.path.ends_with("/items") && turn.response.status_code == 200 {
        return list_shape(&body);
    }
    if turn.request.path.contains("/items/") && turn.request.method == "GET" && turn.response.status_code == 200 {
        return item_shape(&body);
    }
    body
}

fn compare_recordings(name: &str, openai: &Cassette, gateway: &Cassette) {
    assert_eq!(openai.turns.len(), gateway.turns.len(), "{name}: comparable step count");
    let mut ids = IdPairs::default();
    for (i, (a, b)) in openai.turns.iter().zip(&gateway.turns).enumerate() {
        // vLLM may record reasoning items that OpenAI does not expose. Page
        // boundaries therefore differ; each provider's pages are checked
        // against its own full ordered list below.
        if name.starts_with("pagination") && i >= 4 {
            continue;
        }
        let location = format!("{name} turn {}", i + 1);
        assert_eq!(a.request.method, b.request.method, "{location}: method");
        compare_path(&a.request.path, &b.request.path, &mut ids);
        compare_value(
            &a.request.query_params,
            &b.request.query_params,
            &location,
            &mut ids,
            false,
        );
        compare_value(&a.request.body, &b.request.body, &location, &mut ids, false);
        assert_eq!(a.response.status_code, b.response.status_code, "{location}: status");
        assert_eq!(
            a.response.sse.is_some(),
            b.response.sse.is_some(),
            "{location}: SSE transport"
        );
        if a.response.sse.is_some() {
            assert_eq!(
                lifecycle(&a.response),
                lifecycle(&b.response),
                "{location}: SSE lifecycle"
            );
        }
        let a_body = comparable_body(a);
        let b_body = comparable_body(b);
        compare_value(&a_body, &b_body, &location, &mut ids, false);
    }
}

fn response_text(turn: &Turn) -> String {
    let body = result_body(&turn.response);
    body["output"]
        .as_array()
        .expect("response.output array")
        .iter()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|part| part["type"] == "output_text")
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

fn response_turns(cassette: &Cassette) -> Vec<&Turn> {
    cassette
        .turns
        .iter()
        .filter(|turn| turn.request.path == "/v1/responses")
        .collect()
}

fn listed_ids(turn: &Turn) -> Vec<&str> {
    turn.response.body.as_ref().expect("list body")["data"]
        .as_array()
        .expect("list data")
        .iter()
        .filter(|item| item["type"] == "message")
        .map(|item| item["id"].as_str().expect("item ID"))
        .collect()
}

fn all_listed_ids(turn: &Turn) -> Vec<&str> {
    turn.response.body.as_ref().expect("list body")["data"]
        .as_array()
        .expect("list data")
        .iter()
        .map(|item| item["id"].as_str().expect("item ID"))
        .collect()
}

fn assert_answer(turn: &Turn, expected: &str) {
    let text = response_text(turn);
    let answer = text.trim().trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    assert_eq!(answer, expected, "unexpected model answer");
}

fn assert_transport(cassette: &Cassette, streaming: bool) {
    for turn in response_turns(cassette) {
        assert_eq!(turn.request.body["stream"], streaming);
        assert_eq!(turn.response.sse.is_some(), streaming);
        if streaming {
            let kinds = lifecycle(&turn.response);
            assert_eq!(kinds.first().map(String::as_str), Some("response.created"));
            assert_eq!(kinds.last().map(String::as_str), Some("response.completed"));
            assert!(kinds.contains(&"response.output_item.added".to_owned()));
            assert!(kinds.contains(&"response.output_item.done".to_owned()));
            assert!(
                !kinds
                    .iter()
                    .any(|kind| kind == "response.failed" || kind == "response.incomplete")
            );
        }
    }
}

fn check_opening(turns: &[Turn]) -> (String, String, String) {
    assert_eq!(
        (turns[0].request.method.as_str(), turns[0].request.path.as_str()),
        ("POST", "/v1/conversations")
    );
    let conversation_id = turns[0].response.body.as_ref().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        (turns[1].request.method.as_str(), turns[1].request.path.as_str()),
        ("POST", "/v1/responses")
    );
    assert_eq!(turns[1].request.body["conversation"], conversation_id);
    assert!(turns[1].request.body.get("previous_response_id").is_none());
    assert_answer(&turns[1], "SAPPHIRE");

    let items_path = format!("/v1/conversations/{conversation_id}/items");
    assert_eq!(
        (turns[2].request.method.as_str(), turns[2].request.path.as_str()),
        ("POST", items_path.as_str())
    );
    let first_manual_id = turns[2].response.body.as_ref().unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        turns[2].response.body.as_ref().unwrap()["data"][0]["content"][0]["text"],
        "The new secret word is ORCHID."
    );

    (conversation_id, items_path, first_manual_id)
}

fn check_steps(cassette: &Cassette, expected: usize, streaming: bool) {
    assert_eq!(cassette.turns.len(), expected, "API step count");
    for (index, turn) in cassette.turns.iter().enumerate() {
        assert_eq!(turn.filename, format!("t{}", index + 1));
        assert_eq!(turn.response.status_code, 200, "t{} status", index + 1);
    }
    assert_transport(cassette, streaming);
}

fn check_case(cassette: &Cassette, scenario: &str, streaming: bool) {
    let expected = if scenario == "branch" { 6 } else { 5 };
    check_steps(cassette, expected, streaming);

    let turns = &cassette.turns;
    let (conversation_id, items_path, first_manual_id) = check_opening(turns);

    if scenario == "deletion" {
        assert_eq!(turns[3].request.method, "DELETE");
        assert_eq!(turns[3].request.path, format!("{items_path}/{first_manual_id}"));
        let body = turns[3].response.body.as_ref().unwrap();
        assert_eq!(body["object"], "conversation");
        assert_eq!(body["id"], conversation_id);
    } else {
        assert_eq!(
            (turns[3].request.method.as_str(), turns[3].request.path.as_str()),
            ("POST", "/v1/responses")
        );
        if scenario == "branch" {
            let parent_id = result_body(&turns[1].response)["id"].as_str().unwrap().to_owned();
            assert_eq!(turns[3].request.body["previous_response_id"], parent_id);
            assert!(turns[3].request.body.get("conversation").is_none());
            assert_answer(&turns[3], "SAPPHIRE");
            assert_eq!(
                (turns[4].request.method.as_str(), turns[4].request.path.as_str()),
                ("POST", items_path.as_str())
            );
            assert_eq!(
                turns[4].request.body["items"][0]["content"],
                "The branch marker is VIOLET."
            );
        } else {
            assert_eq!(turns[3].request.body["conversation"], conversation_id);
            assert!(turns[3].request.body.get("previous_response_id").is_none());
            assert_answer(&turns[3], "ORCHID");
        }
    }

    let listing = turns.last().unwrap();
    assert_eq!(
        (listing.request.method.as_str(), listing.request.path.as_str()),
        ("GET", items_path.as_str())
    );
    assert_eq!(listing.request.query_params["order"], "asc");
    let listed = listed_ids(listing);
    let first_generated = result_body(&turns[1].response)["output"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "message")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        listed.contains(&first_generated.as_str()),
        "first response item missing from conversation"
    );
    assert_eq!(listed.contains(&first_manual_id.as_str()), scenario != "deletion");
    assert_eq!(
        listed.len(),
        listed.iter().collect::<std::collections::HashSet<_>>().len()
    );
    if scenario != "deletion" {
        let generated_index = listed.iter().position(|id| *id == first_generated).unwrap();
        let manual_index = listed.iter().position(|id| *id == first_manual_id.as_str()).unwrap();
        assert!(generated_index < manual_index, "manual item is out of order");
    }

    if scenario == "branch" {
        let second_manual_id = turns[4].response.body.as_ref().unwrap()["data"][0]["id"]
            .as_str()
            .unwrap();
        assert!(listed.contains(&second_manual_id), "second manual item missing");
        let first_index = listed.iter().position(|id| *id == first_manual_id.as_str()).unwrap();
        let second_index = listed.iter().position(|id| *id == second_manual_id).unwrap();
        assert!(first_index < second_index, "manual items are out of order");
        let branch_output = result_body(&turns[3].response)["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "message")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            !listed.contains(&branch_output.as_str()),
            "response branch leaked into conversation"
        );
    } else if scenario == "continuation" {
        let continuation_output = result_body(&turns[3].response)["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "message")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            listed.contains(&continuation_output.as_str()),
            "conversation continuation output is missing from history"
        );
    }
}

fn check_page_pair(turns: &[Turn], full_index: usize, page_index: usize, order: &str) {
    let full = all_listed_ids(&turns[full_index]);
    let first = all_listed_ids(&turns[page_index]);
    let second = all_listed_ids(&turns[page_index + 1]);
    assert_eq!(turns[full_index].request.query_params, json!({"order": order}));
    assert_eq!(
        turns[page_index].request.query_params,
        json!({"order": order, "limit": "2"})
    );
    assert_eq!(turns[page_index + 1].request.query_params["order"], order);
    assert_eq!(turns[page_index + 1].request.query_params["limit"], "2");
    assert_eq!(first.len(), 2);
    assert!(!second.is_empty() && second.len() <= 2);
    assert_eq!(turns[page_index + 1].request.query_params["after"], first[1]);
    assert_eq!(
        first.iter().chain(second.iter()).copied().collect::<Vec<_>>(),
        full[..first.len() + second.len()],
        "{order} page order differs from its provider's full list"
    );
    for (turn, ids) in [(&turns[page_index], &first), (&turns[page_index + 1], &second)] {
        let body = turn.response.body.as_ref().unwrap();
        assert_eq!(body["first_id"], ids[0]);
        assert_eq!(body["last_id"], ids[ids.len() - 1]);
    }
    assert_eq!(turns[page_index].response.body.as_ref().unwrap()["has_more"], true);
    assert_eq!(
        turns[page_index + 1].response.body.as_ref().unwrap()["has_more"],
        first.len() + second.len() < full.len()
    );
}

fn check_pagination(cassette: &Cassette, streaming: bool) {
    let turns = &cassette.turns;
    check_steps(cassette, 10, streaming);
    let (conversation_id, items_path, manual_id) = check_opening(turns);
    assert_eq!(turns[1].request.body["conversation"], conversation_id);
    for turn in &turns[3..] {
        assert_eq!(turn.request.method, "GET");
        assert_eq!(turn.request.path, items_path);
    }
    check_page_pair(turns, 3, 4, "asc");
    check_page_pair(turns, 6, 7, "desc");
    let ascending = all_listed_ids(&turns[3]);
    let descending = all_listed_ids(&turns[6]);
    assert_eq!(descending, ascending.iter().rev().copied().collect::<Vec<_>>());
    assert!(ascending.contains(&manual_id.as_str()));

    let included = &turns[9];
    assert_eq!(
        included.request.query_params,
        json!({"order": "asc", "include[]": "message.output_text.logprobs"})
    );
    assert_eq!(all_listed_ids(included), ascending);
    assert_eq!(
        list_shape(included.response.body.as_ref().unwrap()),
        list_shape(turns[3].response.body.as_ref().unwrap())
    );
}

fn compare_case(scenario: &str) {
    for streaming in [false, true] {
        let name = if streaming {
            format!("{scenario}-stream")
        } else {
            scenario.to_owned()
        };
        let Some((openai, gateway)) = pair(&name) else { continue };
        if scenario == "pagination" {
            check_pagination(&openai, streaming);
            check_pagination(&gateway, streaming);
        } else {
            check_case(&openai, scenario, streaming);
            check_case(&gateway, scenario, streaming);
        }
        compare_recordings(&name, &openai, &gateway);
    }
}

#[test]
fn continuation_matches_openai() {
    compare_case("continuation");
}

#[test]
fn deletion_matches_openai() {
    compare_case("deletion");
}

#[test]
fn response_branch_matches_openai() {
    compare_case("branch");
}

#[test]
fn pagination_matches_openai() {
    compare_case("pagination");
}
