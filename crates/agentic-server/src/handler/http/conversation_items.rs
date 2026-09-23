//! HTTP transport for conversation item CRUD.

use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

#[cfg(feature = "openapi")]
use agentic_core::types::ConversationResponse;
use agentic_core::types::{CreateItemRequest, ItemResponse, ListItemsResponse};

use super::super::common::{error_response, executor_error_response, extract_json, read_bytes};
use super::conversations::{conversation_response, extract_tenant_id};
use crate::app::AppState;

/// Query parameters for listing conversation items.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListItemsQuery {
    /// Maximum number of items to return (default: 20, max: 100).
    #[serde(default = "default_limit")]
    pub limit: i64,

    /// Cursor for pagination (item ID to start after).
    pub after: Option<String>,

    /// Sort order for items (default: desc). Ascending order is oldest-first.
    #[serde(default = "default_order")]
    pub order: String,

    /// Additional data to include in the response. Currently unsupported - will return error if specified.
    pub include: Option<Vec<String>>,
}

fn default_limit() -> i64 {
    20
}

fn default_order() -> String {
    "desc".to_string()
}

#[cfg_attr(feature = "openapi", utoipa::path(
    post,
    path = "/v1/conversations/{conversation_id}/items",
    params(
        ("conversation_id" = String, Path, description = "Conversation ID")
    ),
    request_body = CreateItemRequest,
    responses(
        (status = 200, description = "Items created", body = ListItemsResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Conversation not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "conversations",
))]
pub async fn create_item(State(state): State<AppState>, Path(conversation_id): Path<String>, req: Request) -> Response {
    let tenant_id = match extract_tenant_id(&req) {
        Ok(id) => id,
        Err(error) => return error,
    };

    let (_, body) = req.into_parts();
    let bytes = match read_bytes(body, state.max_request_body_size).await {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };

    let request: CreateItemRequest = match extract_json(&bytes) {
        Ok(request) => request,
        Err(error) => return error,
    };

    match state
        .exec_ctx
        .conv_handler
        .create_items(&tenant_id, &conversation_id, request.items)
        .await
    {
        Ok(item_responses) => {
            // Return list response (matches OpenAI format)
            let response = agentic_core::types::ListItemsResponse::new(item_responses, false);
            axum::Json(response).into_response()
        }
        Err(e) => executor_error_response(e),
    }
}

#[cfg_attr(feature = "openapi", utoipa::path(
    get,
    path = "/v1/conversations/{conversation_id}/items",
    params(
        ("conversation_id" = String, Path, description = "Conversation ID"),
        ListItemsQuery
    ),
    responses(
        (status = 200, description = "Items retrieved", body = ListItemsResponse),
        (status = 404, description = "Conversation not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "conversations",
))]
pub async fn list_items(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ListItemsQuery>,
    req: Request,
) -> Response {
    let tenant_id = match extract_tenant_id(&req) {
        Ok(id) => id,
        Err(error) => return error,
    };

    // Validate limit parameter
    if query.limit < 1 || query.limit > 100 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "limit must be between 1 and 100",
        );
    }

    // Validate order parameter
    if query.order != "asc" && query.order != "desc" {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "order must be 'asc' or 'desc'",
        );
    }

    // Reject unsupported include values
    if let Some(ref include) = query.include {
        if !include.is_empty() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "include parameter is not currently supported",
            );
        }
    }

    match state
        .exec_ctx
        .conv_handler
        .list_items(
            &tenant_id,
            &conversation_id,
            query.limit,
            query.after.as_deref(),
            &query.order,
        )
        .await
    {
        Ok(response) => axum::Json(response).into_response(),
        Err(e) => executor_error_response(e),
    }
}

#[cfg_attr(feature = "openapi", utoipa::path(
    get,
    path = "/v1/conversations/{conversation_id}/items/{item_id}",
    params(
        ("conversation_id" = String, Path, description = "Conversation ID"),
        ("item_id" = String, Path, description = "Item ID")
    ),
    responses(
        (status = 200, description = "Item retrieved", body = ItemResponse),
        (status = 404, description = "Item or conversation not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "conversations",
))]
pub async fn retrieve_item(
    State(state): State<AppState>,
    Path((conversation_id, item_id)): Path<(String, String)>,
    req: Request,
) -> Response {
    let tenant_id = match extract_tenant_id(&req) {
        Ok(id) => id,
        Err(error) => return error,
    };

    match state
        .exec_ctx
        .conv_handler
        .retrieve_item(&tenant_id, &conversation_id, &item_id)
        .await
    {
        Ok(response) => axum::Json(response).into_response(),
        Err(e) => executor_error_response(e),
    }
}

#[cfg_attr(feature = "openapi", utoipa::path(
    delete,
    path = "/v1/conversations/{conversation_id}/items/{item_id}",
    params(
        ("conversation_id" = String, Path, description = "Conversation ID"),
        ("item_id" = String, Path, description = "Item ID")
    ),
    responses(
        (status = 200, description = "Item deleted", body = ConversationResponse),
        (status = 404, description = "Item or conversation not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "conversations",
))]
pub async fn delete_item(
    State(state): State<AppState>,
    Path((conversation_id, item_id)): Path<(String, String)>,
    req: Request,
) -> Response {
    let tenant_id = match extract_tenant_id(&req) {
        Ok(id) => id,
        Err(error) => return error,
    };

    match state
        .exec_ctx
        .conv_handler
        .delete_item(&tenant_id, &conversation_id, &item_id)
        .await
    {
        Ok(conversation) => conversation_response(conversation),
        Err(e) => executor_error_response(e),
    }
}
