# Conversations API Cassettes

This directory contains recorded HTTP request/response cassettes for Conversations API testing.

## Recording Requirements

To record cassettes, you need:

1. **For OpenAI reference recordings:**
   - OpenAI API key with access to the Conversations API
   - Run: `CONVERSATIONS_RECORD_SET=openai OPENAI_API_KEY=sk-xxx bash ../record_conversations_api_cassettes.sh`

2. **For gateway recordings:**
   - Running gateway instance (`cargo run -p agentic-server`)
   - Configured database (SQLite or PostgreSQL)
   - LLM backend (vLLM or OpenAI API via `--llm-api-base`)
   - Run: `CONVERSATIONS_RECORD_SET=gateway GATEWAY_URL=http://localhost:9000 bash ../record_conversations_api_cassettes.sh`

## Expected Cassettes

Each scenario produces two cassettes:

| Scenario | OpenAI Cassette | Gateway Cassette |
|----------|----------------|------------------|
| Basic CRUD | `conversations-basic-crud-openai.yaml` | `conversations-basic-crud-gateway.yaml` |
| Pagination | `conversations-pagination-openai.yaml` | `conversations-pagination-gateway.yaml` |
| Stateful Context | `conversations-stateful-context-openai.yaml` | `conversations-stateful-context-gateway.yaml` |
| Deletion Continuation | `conversations-deletion-continuation-openai.yaml` | `conversations-deletion-continuation-gateway.yaml` |
| Branching | `conversations-branching-openai.yaml` | `conversations-branching-gateway.yaml` |
| Error Cases | `conversations-error-cases-openai.yaml` | `conversations-error-cases-gateway.yaml` |

## Cassette Format

Each cassette is a YAML file with the structure:

```yaml
turns:
- filename: t1
  request:
    method: POST
    path: /v1/conversations
    body:
      metadata: {"test": "value"}
    headers:
      content-type: application/json
    query_params: {}
  response:
    status_code: 200
    headers:
      content-type: application/json
    body:
      id: conv_abc123
      object: conversation
      created_at: 1234567890
      metadata: {"test": "value"}

- filename: t2
  request:
    method: GET
    path: /v1/conversations/conv_abc123
    # ... and so on
```

## Recording Instructions

From the repository root:

```bash
# 1. Start the gateway (in one terminal)
cargo run -p agentic-server -- \
  --llm-api-base http://localhost:8000 \
  --database-url sqlite:///tmp/conversations_test.db

# 2. Record gateway cassettes (in another terminal)
cd crates/agentic-server-core
CONVERSATIONS_RECORD_SET=gateway \
GATEWAY_URL=http://localhost:9000 \
bash tests/cassettes/record_conversations_api_cassettes.sh

# 3. Record OpenAI reference (requires API key)
CONVERSATIONS_RECORD_SET=openai \
OPENAI_API_KEY=sk-xxx \
OPENAI_MODEL=gpt-4o \
bash tests/cassettes/record_conversations_api_cassettes.sh
```

## Testing Without Cassettes

The replay tests (`conversations_api_cassette_test.rs`) will be skipped if cassettes are not present:

```bash
cargo test conversations_api_cassette_test
# Tests will skip with: "cassette not found, skipping..."
```

## Status

- [ ] OpenAI reference cassettes recorded
- [ ] Gateway cassettes recorded
- [x] Recording infrastructure implemented
- [x] Replay tests implemented

**Note:** Cassette recording requires access to either OpenAI's Conversations API or a running gateway instance with LLM backend. The infrastructure is complete and functional, but actual recordings require these dependencies.
