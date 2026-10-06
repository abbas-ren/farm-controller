use super::*;

fn policy(table_name: &str, timestamp_column: &str) -> RetentionPolicy {
    RetentionPolicy {
        table_name: table_name.to_owned(),
        timestamp_column: timestamp_column.to_owned(),
        retention_days: 30,
        batch_size: 500,
        archive_before_delete: false,
    }
}

#[test]
fn retention_targets_reject_unreviewed_identifiers() {
    let error = RetentionTarget::from_policy(&policy("devices; DROP TABLE devices", "updatedAt"))
        .unwrap_err();
    assert!(matches!(error, RetentionError::UnsupportedTarget(_, _)));
}

#[test]
fn queue_cleanup_only_targets_terminal_jobs() {
    let target = RetentionTarget::from_policy(&policy("device_action_queue", "updatedAt")).unwrap();
    let sql = target.delete_sql();
    assert!(sql.contains("FOR UPDATE SKIP LOCKED"));
    assert!(sql.contains("'completed', 'cancelled'"));
    assert!(!sql.contains("'failed'"));
    assert!(sql.contains("LIMIT $2"));
}

#[test]
fn alert_cleanup_uses_legacy_snake_case_timestamp() {
    let target = RetentionTarget::from_policy(&policy("alerts", "created_at")).unwrap();
    let sql = target.delete_sql();
    assert!(sql.contains("created_at"));
    assert!(!sql.contains("\"createdAt\""));
}

#[test]
fn embedded_migrations_are_discovered() {
    assert_eq!(MIGRATOR.iter().count(), 5);
}
