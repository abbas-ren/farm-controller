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

/// PURPOSE: Assert queue retention maps onto the device_action_queue target.
/// USE: Guards allow-list wiring after SeaORM `delete_batch` replaced raw SQL.
#[test]
fn queue_cleanup_resolves_device_action_queue_target() {
    let target = RetentionTarget::from_policy(&policy("device_action_queue", "updatedAt")).unwrap();
    assert_eq!(target, RetentionTarget::DeviceActionQueue);
}

/// PURPOSE: Confirm alerts retention still keys off legacy snake_case `created_at`.
/// USE: Prevents regressions that would require camelCase `"createdAt"` for alerts.
#[test]
fn alert_cleanup_resolves_legacy_snake_case_timestamp() {
    let target = RetentionTarget::from_policy(&policy("alerts", "created_at")).unwrap();
    assert_eq!(target, RetentionTarget::Alerts);
    let rejected = RetentionTarget::from_policy(&policy("alerts", "createdAt")).unwrap_err();
    assert!(matches!(rejected, RetentionError::UnsupportedTarget(_, _)));
}

/// PURPOSE: Ensure the embedded sqlx migrator still finds expected migrations.
/// USE: Fails the build if migrations are accidentally removed from the tree.
#[test]
fn embedded_migrations_are_discovered() {
    assert_eq!(MIGRATOR.iter().count(), 7);
}
