//! Item queries authorized through the owning conversation.

use super::{DbPool, DbResult, DbTransaction, Item};
use crate::types::conversations::ItemOrder;

/// List a page using keyset pagination with (seq, id) ordering.
///
/// Resolves the cursor to get its (seq, id) tuple, then paginates using compound predicates
/// that match the ordering keys to prevent skipping or repeating items.
///
/// # Errors
/// Returns an error if the query fails or cursor not found in conversation.
#[allow(clippy::too_many_lines)]
pub async fn list_for_conversation(
    pool: &DbPool,
    tenant_id: &str,
    conversation_id: &str,
    limit: i64,
    after_id: Option<&str>,
    order: ItemOrder,
) -> DbResult<Vec<Item>> {
    let order_clause = match order {
        ItemOrder::Desc => "ORDER BY items.seq DESC, items.id DESC",
        ItemOrder::Asc => "ORDER BY items.seq ASC, items.id ASC",
    };

    if let Some(cursor_id) = after_id {
        // Resolve cursor to get its (seq, id) tuple within this conversation and tenant
        let cursor: Option<(Option<i64>, String)> = sqlx::query_as(
            "SELECT items.seq, items.id FROM items \
             JOIN conversations ON conversations.id = items.conversation_id \
             WHERE conversations.id = $1 AND conversations.tenant_id = $2 AND items.id = $3",
        )
        .bind(conversation_id)
        .bind(tenant_id)
        .bind(cursor_id)
        .fetch_optional(pool)
        .await?;

        let Some((cursor_seq, cursor_id_value)) = cursor else {
            // Cursor not found in this conversation - return empty
            return Ok(Vec::new());
        };

        // Paginate using (seq, id) keyset - handles NULL seq correctly
        if order == ItemOrder::Desc {
            // Descending: (seq < cursor_seq) OR (seq = cursor_seq AND id < cursor_id)
            if let Some(seq) = cursor_seq {
                sqlx::query_as(&format!(
                    "SELECT items.* FROM items \
                     JOIN conversations ON conversations.id = items.conversation_id \
                     WHERE conversations.id = $1 AND conversations.tenant_id = $2 \
                     AND ((items.seq < $3) OR (items.seq = $3 AND items.id < $4)) \
                     {order_clause} \
                     LIMIT $5"
                ))
                .bind(conversation_id)
                .bind(tenant_id)
                .bind(seq)
                .bind(&cursor_id_value)
                .bind(limit)
                .fetch_all(pool)
                .await
            } else {
                // cursor has NULL seq - only items with NULL seq and id < cursor_id
                sqlx::query_as(&format!(
                    "SELECT items.* FROM items \
                     JOIN conversations ON conversations.id = items.conversation_id \
                     WHERE conversations.id = $1 AND conversations.tenant_id = $2 \
                     AND items.seq IS NULL AND items.id < $3 \
                     {order_clause} \
                     LIMIT $4"
                ))
                .bind(conversation_id)
                .bind(tenant_id)
                .bind(&cursor_id_value)
                .bind(limit)
                .fetch_all(pool)
                .await
            }
        } else {
            // Ascending: (seq > cursor_seq) OR (seq = cursor_seq AND id > cursor_id)
            if let Some(seq) = cursor_seq {
                sqlx::query_as(&format!(
                    "SELECT items.* FROM items \
                     JOIN conversations ON conversations.id = items.conversation_id \
                     WHERE conversations.id = $1 AND conversations.tenant_id = $2 \
                     AND ((items.seq > $3) OR (items.seq = $3 AND items.id > $4)) \
                     {order_clause} \
                     LIMIT $5"
                ))
                .bind(conversation_id)
                .bind(tenant_id)
                .bind(seq)
                .bind(&cursor_id_value)
                .bind(limit)
                .fetch_all(pool)
                .await
            } else {
                // cursor has NULL seq - return items with non-NULL seq, or NULL seq with id > cursor_id
                sqlx::query_as(&format!(
                    "SELECT items.* FROM items \
                     JOIN conversations ON conversations.id = items.conversation_id \
                     WHERE conversations.id = $1 AND conversations.tenant_id = $2 \
                     AND (items.seq IS NOT NULL OR (items.seq IS NULL AND items.id > $3)) \
                     {order_clause} \
                     LIMIT $4"
                ))
                .bind(conversation_id)
                .bind(tenant_id)
                .bind(&cursor_id_value)
                .bind(limit)
                .fetch_all(pool)
                .await
            }
        }
    } else {
        // No cursor - start from beginning
        sqlx::query_as(&format!(
            "SELECT items.* FROM items \
             JOIN conversations ON conversations.id = items.conversation_id \
             WHERE conversations.id = $1 AND conversations.tenant_id = $2 \
             {order_clause} \
             LIMIT $3"
        ))
        .bind(conversation_id)
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

/// Get an item through its conversation, including items written by Responses persistence.
///
/// # Errors
/// Returns an error if the query fails.
pub async fn get_for_conversation(
    pool: &DbPool,
    tenant_id: &str,
    conversation_id: &str,
    item_id: &str,
) -> DbResult<Option<Item>> {
    sqlx::query_as(
        "SELECT items.* FROM items JOIN conversations ON conversations.id = items.conversation_id \
         WHERE conversations.id = $1 AND conversations.tenant_id = $2 AND items.id = $3",
    )
    .bind(conversation_id)
    .bind(tenant_id)
    .bind(item_id)
    .fetch_optional(pool)
    .await
}

/// Remove one or all items from a locked conversation while preserving stored response history.
///
/// # Errors
/// Returns an error if the update fails.
pub async fn detach_from_conversation_in_tx(
    tx: &mut DbTransaction<'_>,
    conversation_id: &str,
    item_id: Option<&str>,
) -> DbResult<u64> {
    let result = sqlx::query(
        "UPDATE items SET conversation_id = NULL, seq = NULL \
         WHERE conversation_id = $1 AND (CAST($2 AS TEXT) IS NULL OR id = $2)",
    )
    .bind(conversation_id)
    .bind(item_id)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}
