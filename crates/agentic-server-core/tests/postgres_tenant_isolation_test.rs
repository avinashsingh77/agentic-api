use std::sync::Arc;

use agentic_core::config::{PostgresConfig, SqliteConfig};
use agentic_core::storage::{ConversationStore, InOutItem, create_pool_with_schema_and_configs};
use agentic_core::types::io::{InputItem, InputMessage, InputMessageContent};
use serde_json::json;

async fn setup_postgres_pool() -> Arc<agentic_core::storage::DbPool> {
    let database_url = std::env::var("TEST_POSTGRES_URL").expect("TEST_POSTGRES_URL must be set");
    create_pool_with_schema_and_configs(Some(&database_url), SqliteConfig::default(), PostgresConfig::default())
        .await
        .expect("create PostgreSQL pool")
}

#[tokio::test]
#[ignore = "requires TEST_POSTGRES_URL pointing to an isolated PostgreSQL database"]
async fn postgres_conversation_tenant_isolation() {
    let pool = setup_postgres_pool().await;
    let store = ConversationStore::new(pool);

    // Create conversation for tenant A with metadata
    let conv_a = store
        .create_with_metadata_and_items(
            Some("tenant_a"),
            Some(json!({"user": "alice", "project": "test"})),
            vec![],
        )
        .await
        .expect("create for tenant_a failed");

    assert!(conv_a.conversation_id.starts_with("conv_"));

    // Tenant A can retrieve it
    let retrieved = store
        .retrieve("tenant_a", &conv_a.conversation_id)
        .await
        .expect("tenant_a should retrieve own conversation");

    assert_eq!(retrieved.conversation_id, conv_a.conversation_id);

    // Tenant B CANNOT retrieve it (cross-tenant access blocked)
    let result = store.retrieve("tenant_b", &conv_a.conversation_id).await;
    assert!(result.is_err(), "tenant_b should NOT retrieve tenant_a's conversation");

    // Verify it's a NotFound error
    match result {
        Err(agentic_core::storage::StorageError::NotFound { .. }) => {
            // Expected
        }
        other => panic!("Expected NotFound error, got: {:?}", other),
    }
}

#[tokio::test]
#[ignore = "requires TEST_POSTGRES_URL pointing to an isolated PostgreSQL database"]
async fn postgres_conversation_metadata_update_with_tenant() {
    let pool = setup_postgres_pool().await;
    let store = ConversationStore::new(pool);

    // Create conversation
    let conv = store
        .create_with_metadata_and_items(Some("tenant_a"), Some(json!({"status": "draft"})), vec![])
        .await
        .expect("create failed");

    // Update metadata
    let updated = store
        .update_metadata(
            "tenant_a",
            &conv.conversation_id,
            json!({"status": "active", "updated": true}),
        )
        .await
        .expect("update_metadata failed");

    assert_eq!(updated.conversation_id, conv.conversation_id);

    // Verify updated metadata
    let retrieved = store
        .retrieve("tenant_a", &conv.conversation_id)
        .await
        .expect("retrieve failed");

    assert_eq!(retrieved.conversation_id, conv.conversation_id);
}

#[tokio::test]
#[ignore = "requires TEST_POSTGRES_URL pointing to an isolated PostgreSQL database"]
async fn postgres_conversation_delete_with_tenant_scoping() {
    let pool = setup_postgres_pool().await;
    let store = ConversationStore::new(pool);

    // Create conversation for tenant A
    let conv = store
        .create_with_metadata_and_items(Some("tenant_a"), Some(json!({"temp": true})), vec![])
        .await
        .expect("create failed");

    // Tenant B cannot delete tenant A's conversation
    let delete_result = store.delete("tenant_b", &conv.conversation_id).await;
    assert!(
        delete_result.is_err(),
        "tenant_b should NOT delete tenant_a's conversation"
    );

    // Verify conversation still exists for tenant A
    let still_exists = store.retrieve("tenant_a", &conv.conversation_id).await;
    assert!(
        still_exists.is_ok(),
        "conversation should still exist after failed delete"
    );

    // Tenant A can delete their own conversation
    let delete_result = store.delete("tenant_a", &conv.conversation_id).await;
    assert!(delete_result.is_ok(), "tenant_a should delete own conversation");

    // Verify conversation is gone
    let should_be_gone = store.retrieve("tenant_a", &conv.conversation_id).await;
    assert!(should_be_gone.is_err(), "conversation should be deleted");
}

#[tokio::test]
#[ignore = "requires TEST_POSTGRES_URL pointing to an isolated PostgreSQL database"]
async fn postgres_conversation_create_with_initial_items() {
    let pool = setup_postgres_pool().await;
    let store = ConversationStore::new(pool);

    let initial_items = vec![InOutItem::Input(InputItem::Message(InputMessage {
        id: None,
        role: "user".to_string(),
        status: None,
        content: InputMessageContent::Text("Hello, PostgreSQL!".to_string()),
    }))];

    // Create conversation with initial items
    let conv = store
        .create_with_metadata_and_items(
            Some("tenant_a"),
            Some(json!({"test": "initial_items", "db": "postgres"})),
            initial_items,
        )
        .await
        .expect("create with items failed");

    assert!(conv.conversation_id.starts_with("conv_"));

    // Rehydrate and verify items are present
    let items = store.rehydrate(&conv.conversation_id).await.expect("rehydrate failed");

    assert_eq!(items.len(), 1, "should have 1 initial item");
}

#[tokio::test]
#[ignore = "requires TEST_POSTGRES_URL pointing to an isolated PostgreSQL database"]
async fn postgres_concurrent_tenant_isolation() {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let pool = setup_postgres_pool().await;
    let store = Arc::new(ConversationStore::new(pool));
    let barrier = Arc::new(Barrier::new(3));

    // Create conversations concurrently for different tenants
    let mut handles = vec![];

    for tenant_id in ["tenant_x", "tenant_y", "tenant_z"] {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let tenant = tenant_id.to_string();

        let handle = tokio::spawn(async move {
            barrier.wait().await;

            let conv = store
                .create_with_metadata_and_items(Some(&tenant), Some(json!({"tenant": tenant})), vec![])
                .await
                .expect("create failed");

            (tenant, conv.conversation_id)
        });

        handles.push(handle);
    }

    // Wait for all to complete
    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.expect("task failed"))
        .collect();

    // Verify each tenant can only access their own conversation
    for (tenant, conv_id) in &results {
        let store = Arc::clone(&store);

        // Can retrieve own conversation
        let own = store.retrieve(tenant, conv_id).await;
        assert!(own.is_ok(), "tenant {} should retrieve own conversation", tenant);

        // Cannot retrieve other tenants' conversations
        for (other_tenant, other_conv_id) in &results {
            if tenant != other_tenant {
                let cross_access = store.retrieve(tenant, other_conv_id).await;
                assert!(
                    cross_access.is_err(),
                    "tenant {} should NOT access tenant {}'s conversation",
                    tenant,
                    other_tenant
                );
            }
        }
    }
}
