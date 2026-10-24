//! Unit tests for retention allow-listing and migration discovery.
//!
//! These tests guard the SQL-injection-resistant retention target map and
//! verify embedded migrations remain discoverable. They do not hit a live DB.

use super::*;

/// PURPOSE: Build a minimal policy DTO for target-resolution tests.
/// USE: Fixtures only; not used in production code paths.
fn policy(table_name: &str, timestamp_column: &str) -> RetentionPolicy {
    RetentionPolicy {
        table_name: table_name.to_owned(),
        timestamp_column: timestamp_column.to_owned(),
        retention_days: 30,
        batch_size: 500,
        archive_before_delete: false,
    }
}

/// PURPOSE: Ensure malicious / unknown table names are rejected.
/// USE: Guards `RetentionTarget::from_policy` allow-list behavior.
#[test]
fn retention_targets_reject_unreviewed_identifiers() {
    let error = RetentionTarget::from_policy(&policy("devices; DROP TABLE devices", "updatedAt"))
        .unwrap_err();
    assert!(matches!(error, RetentionError::UnsupportedTarget(_, _)));
}

/// PURPOSE: Assert queue retention SQL only deletes terminal jobs with SKIP LOCKED.
/// USE: Locks the delete_sql shape for device_action_queue cleanup.
#[test]
fn queue_cleanup_only_targets_terminal_jobs() {
    let target = RetentionTarget::from_policy(&policy("device_action_queue", "updatedAt")).unwrap();
    let sql = target.delete_sql();
    assert!(sql.contains("FOR UPDATE SKIP LOCKED"));
    assert!(sql.contains("'completed', 'cancelled'"));
    assert!(!sql.contains("'failed'"));
    assert!(sql.contains("LIMIT $2"));
}

/// PURPOSE: Confirm alerts retention SQL uses snake_case `created_at`.
/// USE: Prevents regressions that would emit camelCase `"createdAt"` for alerts.
#[test]
fn alert_cleanup_uses_legacy_snake_case_timestamp() {
    let target = RetentionTarget::from_policy(&policy("alerts", "created_at")).unwrap();
    let sql = target.delete_sql();
    assert!(sql.contains("created_at"));
    assert!(!sql.contains("\"createdAt\""));
}

/// PURPOSE: Ensure the embedded sqlx migrator still finds expected migrations.
/// USE: Fails the build if migrations are accidentally removed from the tree.
#[test]
fn embedded_migrations_are_discovered() {
    assert_eq!(MIGRATOR.iter().count(), 6);
}
