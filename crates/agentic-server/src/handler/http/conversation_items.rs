use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use agentic_core::storage::models::item;
use agentic_core::types::{CreateItemRequest, DeletedResponse, ItemResponse, ListItemsResponse};
use agentic_core::utils::common::uuid7_str;

use super::super::common::{error_response, extract_json, read_bytes};
use crate::app::AppState;

/// Extract tenant ID from authenticated principal in request extensions.
fn extract_tenant_id(_req: &Request) -> Result<String, Response> {
    // For now, return a placeholder until we wire up authentication
    // In production, this would extract from the AuthenticatedPrincipal extension
    Ok("default_tenant".to_string())
}

/// Query parameters for listing items.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListItemsQuery {
    /// Maximum number of items to return (default: 20, max: 100).
    #[serde(default = "default_limit")]
    pub limit: i64,

    /// Cursor for pagination (item ID to start after).
    pub after: Option<String>,
}

fn default_limit() -> i64 {
    20
}

/// Create a new item in a conversation.
#[cfg_attr(feature = "openapi", utoipa::path(
    post,
    path = "/v1/conversations/{conversation_id}/items",
    params(
        ("conversation_id" = String, Path, description = "Conversation ID")
    ),
    request_body = CreateItemRequest,
    responses(
        (status = 200, description = "Item created", body = ItemResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Conversation not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "conversations",
))]
pub async fn create_item(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
    req: Request,
) -> Response {
    let tenant_id = match extract_tenant_id(&req) {
        Ok(id) => id,
        Err(err) => return err,
    };

    let (_, body) = req.into_parts();
    let bytes = match read_bytes(body, state.max_request_body_size).await {
        Ok(b) => b,
        Err(e) => return e,
    };

    let request: CreateItemRequest = match extract_json(&bytes) {
        Ok(r) => r,
        Err(e) => return e,
    };

    // Verify conversation exists and belongs to tenant
    if let Err(e) = state.exec_ctx.conv_handler.store().retrieve(&tenant_id, &conversation_id).await {
        return match e {
            agentic_core::storage::StorageError::NotFound { .. } => {
                error_response(StatusCode::NOT_FOUND, "not_found", "Conversation not found")
            }
            _ => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Failed to verify conversation: {e}"),
            ),
        };
    }

    let item_id = uuid7_str("item_");
    let item_data = match &request.item {
        agentic_core::types::ConversationItem::Input(input) => {
            String::try_from(&agentic_core::storage::InOutItem::Input(input.clone()))
        }
        agentic_core::types::ConversationItem::Output(output) => {
            String::try_from(&agentic_core::storage::InOutItem::Output(output.clone()))
        }
    };

    let item_data_str = match item_data {
        Ok(s) => s,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "serialization_error",
                &format!("Failed to serialize item: {e}"),
            )
        }
    };

    let pool = match state.exec_ctx.conv_handler.store().pool() {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Storage not configured: {e}"),
            )
        }
    };

    match item::create_items_for_conversation(pool, &tenant_id, &conversation_id, vec![(item_id.clone(), item_data_str)])
        .await
    {
        Ok(mut items) => {
            if let Some(created_item) = items.pop() {
                let response = ItemResponse::new(created_item.id, created_item.created_at, request.item);
                axum::Json(response).into_response()
            } else {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage_error", "Failed to create item")
            }
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            &format!("Failed to create item: {e}"),
        ),
    }
}

/// List items in a conversation with pagination.
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
        Err(err) => return err,
    };

    // Verify conversation exists and belongs to tenant
    if let Err(e) = state.exec_ctx.conv_handler.store().retrieve(&tenant_id, &conversation_id).await {
        return match e {
            agentic_core::storage::StorageError::NotFound { .. } => {
                error_response(StatusCode::NOT_FOUND, "not_found", "Conversation not found")
            }
            _ => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Failed to verify conversation: {e}"),
            ),
        };
    }

    let limit = query.limit.clamp(1, 100);

    let pool = match state.exec_ctx.conv_handler.store().pool() {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Storage not configured: {e}"),
            )
        }
    };

    // Fetch limit + 1 to determine if there are more items
    match item::list_items(pool, &tenant_id, &conversation_id, limit + 1, query.after.as_deref()).await {
        Ok(mut items) => {
            let has_more = items.len() as i64 > limit;
            if has_more {
                items.pop(); // Remove the extra item
            }

            let item_responses: Vec<ItemResponse> = items
                .into_iter()
                .filter_map(|db_item| {
                    let conversation_item = db_item.as_inout().and_then(|inout| match inout {
                        agentic_core::storage::InOutItem::Input(input) => {
                            Some(agentic_core::types::ConversationItem::Input(input))
                        }
                        agentic_core::storage::InOutItem::Output(output) => {
                            Some(agentic_core::types::ConversationItem::Output(output))
                        }
                    })?;

                    Some(ItemResponse::new(db_item.id, db_item.created_at, conversation_item))
                })
                .collect();

            let response = ListItemsResponse::new(item_responses, has_more);
            axum::Json(response).into_response()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            &format!("Failed to list items: {e}"),
        ),
    }
}

/// Retrieve a single item by ID.
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
        Err(err) => return err,
    };

    // Verify conversation exists and belongs to tenant
    if let Err(e) = state.exec_ctx.conv_handler.store().retrieve(&tenant_id, &conversation_id).await {
        return match e {
            agentic_core::storage::StorageError::NotFound { .. } => {
                error_response(StatusCode::NOT_FOUND, "not_found", "Conversation not found")
            }
            _ => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Failed to verify conversation: {e}"),
            ),
        };
    }

    let pool = match state.exec_ctx.conv_handler.store().pool() {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Storage not configured: {e}"),
            )
        }
    };

    match item::get_item_by_tenant(pool, &tenant_id, &item_id).await {
        Ok(Some(db_item)) => {
            // Verify item belongs to the specified conversation
            if db_item.conversation_id.as_deref() != Some(&conversation_id) {
                return error_response(StatusCode::NOT_FOUND, "not_found", "Item not found in this conversation");
            }

            if let Some(inout_item) = db_item.as_inout() {
                let conversation_item = match inout_item {
                    agentic_core::storage::InOutItem::Input(input) => {
                        agentic_core::types::ConversationItem::Input(input)
                    }
                    agentic_core::storage::InOutItem::Output(output) => {
                        agentic_core::types::ConversationItem::Output(output)
                    }
                };

                let response = ItemResponse::new(db_item.id, db_item.created_at, conversation_item);
                axum::Json(response).into_response()
            } else {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "serialization_error", "Failed to deserialize item")
            }
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Item not found"),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            &format!("Failed to retrieve item: {e}"),
        ),
    }
}

/// Delete an item by ID.
#[cfg_attr(feature = "openapi", utoipa::path(
    delete,
    path = "/v1/conversations/{conversation_id}/items/{item_id}",
    params(
        ("conversation_id" = String, Path, description = "Conversation ID"),
        ("item_id" = String, Path, description = "Item ID")
    ),
    responses(
        (status = 200, description = "Item deleted", body = DeletedResponse),
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
        Err(err) => return err,
    };

    // Verify conversation exists and belongs to tenant
    if let Err(e) = state.exec_ctx.conv_handler.store().retrieve(&tenant_id, &conversation_id).await {
        return match e {
            agentic_core::storage::StorageError::NotFound { .. } => {
                error_response(StatusCode::NOT_FOUND, "not_found", "Conversation not found")
            }
            _ => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Failed to verify conversation: {e}"),
            ),
        };
    }

    let pool = match state.exec_ctx.conv_handler.store().pool() {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Storage not configured: {e}"),
            )
        }
    };

    // Verify item exists and belongs to this conversation before deleting
    match item::get_item_by_tenant(pool, &tenant_id, &item_id).await {
        Ok(Some(db_item)) => {
            if db_item.conversation_id.as_deref() != Some(&conversation_id) {
                return error_response(StatusCode::NOT_FOUND, "not_found", "Item not found in this conversation");
            }
        }
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "Item not found"),
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                &format!("Failed to verify item: {e}"),
            )
        }
    }

    match item::delete_item(pool, &tenant_id, &item_id).await {
        Ok(rows_affected) => {
            if rows_affected > 0 {
                let response = DeletedResponse::item(item_id);
                axum::Json(response).into_response()
            } else {
                error_response(StatusCode::NOT_FOUND, "not_found", "Item not found")
            }
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            &format!("Failed to delete item: {e}"),
        ),
    }
}
