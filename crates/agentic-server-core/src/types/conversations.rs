//! Types for the OpenAI-compatible Conversations API.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::io::{InputItem, OutputItem};

/// Request to create a new conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateConversationRequest {
    /// Optional metadata as a JSON object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,

    /// Optional initial items to add to the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<ConversationItem>>,
}

/// Request to update a conversation's metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateConversationRequest {
    /// Metadata as a JSON object.
    pub metadata: Value,
}

/// Response for a conversation resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConversationResponse {
    /// Unique conversation identifier.
    pub id: String,

    /// Object type, always "conversation".
    pub object: String,

    /// Creation timestamp as Unix timestamp in seconds.
    pub created_at: i64,

    /// Metadata as a JSON object.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

/// Request to create a new item in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateItemRequest {
    /// The item to add (input or output).
    #[serde(flatten)]
    pub item: ConversationItem,
}

/// Response for listing items with pagination.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ListItemsResponse {
    /// Object type, always "list".
    pub object: String,

    /// Array of conversation items.
    pub data: Vec<ItemResponse>,

    /// Whether there are more items available.
    pub has_more: bool,

    /// ID of the first item in this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_id: Option<String>,

    /// ID of the last item in this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_id: Option<String>,
}

/// Response for a single conversation item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ItemResponse {
    /// Unique item identifier.
    pub id: String,

    /// Object type, always "conversation.item".
    pub object: String,

    /// Creation timestamp as Unix timestamp in seconds.
    pub created_at: i64,

    /// The item content.
    #[serde(flatten)]
    pub item: ConversationItem,
}

/// A conversation item (input or output).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ConversationItem {
    /// Input item (message, tool call, etc.).
    Input(InputItem),
    /// Output item (message, tool result, etc.).
    Output(OutputItem),
}

/// Response for successful deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeletedResponse {
    /// ID of the deleted resource.
    pub id: String,

    /// Object type.
    pub object: String,

    /// Whether the deletion was successful.
    pub deleted: bool,
}

impl ConversationResponse {
    /// Create a new conversation response.
    #[must_use]
    pub fn new(id: String, created_at: i64, metadata: Option<Value>) -> Self {
        Self {
            id,
            object: "conversation".to_string(),
            created_at,
            metadata: metadata.unwrap_or(Value::Null),
        }
    }
}

impl ItemResponse {
    /// Create a new item response.
    #[must_use]
    pub fn new(id: String, created_at: i64, item: ConversationItem) -> Self {
        Self {
            id,
            object: "conversation.item".to_string(),
            created_at,
            item,
        }
    }
}

impl ListItemsResponse {
    /// Create a new list response.
    #[must_use]
    pub fn new(data: Vec<ItemResponse>, has_more: bool) -> Self {
        let first_id = data.first().map(|item| item.id.clone());
        let last_id = data.last().map(|item| item.id.clone());

        Self {
            object: "list".to_string(),
            data,
            has_more,
            first_id,
            last_id,
        }
    }
}

impl DeletedResponse {
    /// Create a deleted conversation response.
    #[must_use]
    pub fn conversation(id: String) -> Self {
        Self {
            id,
            object: "conversation.deleted".to_string(),
            deleted: true,
        }
    }

    /// Create a deleted item response.
    #[must_use]
    pub fn item(id: String) -> Self {
        Self {
            id,
            object: "conversation.item.deleted".to_string(),
            deleted: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_conversation_response_new() {
        let resp = ConversationResponse::new(
            "conv_123".to_string(),
            1704067200,
            Some(json!({"user": "alice"})),
        );

        assert_eq!(resp.id, "conv_123");
        assert_eq!(resp.object, "conversation");
        assert_eq!(resp.created_at, 1704067200);
        assert_eq!(resp.metadata, json!({"user": "alice"}));
    }

    #[test]
    fn test_conversation_response_null_metadata() {
        let resp = ConversationResponse::new("conv_123".to_string(), 1704067200, None);

        assert_eq!(resp.metadata, Value::Null);
    }

    #[test]
    fn test_list_items_response_ids() {
        let items = vec![
            ItemResponse {
                id: "item_1".to_string(),
                object: "conversation.item".to_string(),
                created_at: 1704067200,
                item: ConversationItem::Input(InputItem::Unknown),
            },
            ItemResponse {
                id: "item_2".to_string(),
                object: "conversation.item".to_string(),
                created_at: 1704067300,
                item: ConversationItem::Input(InputItem::Unknown),
            },
        ];

        let resp = ListItemsResponse::new(items, false);

        assert_eq!(resp.object, "list");
        assert_eq!(resp.first_id, Some("item_1".to_string()));
        assert_eq!(resp.last_id, Some("item_2".to_string()));
        assert!(!resp.has_more);
    }

    #[test]
    fn test_deleted_response_conversation() {
        let resp = DeletedResponse::conversation("conv_123".to_string());

        assert_eq!(resp.id, "conv_123");
        assert_eq!(resp.object, "conversation.deleted");
        assert!(resp.deleted);
    }

    #[test]
    fn test_deleted_response_item() {
        let resp = DeletedResponse::item("item_123".to_string());

        assert_eq!(resp.id, "item_123");
        assert_eq!(resp.object, "conversation.item.deleted");
        assert!(resp.deleted);
    }

    #[test]
    fn test_create_conversation_request_serialization() {
        let req = CreateConversationRequest {
            metadata: Some(json!({"key": "value"})),
            items: None,
        };

        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("metadata"));
        assert!(!json.contains("items")); // Should skip None
    }

    #[test]
    fn test_update_conversation_request() {
        let req = UpdateConversationRequest {
            metadata: json!({"status": "active"}),
        };

        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("status"));
        assert!(json.contains("active"));
    }
}
