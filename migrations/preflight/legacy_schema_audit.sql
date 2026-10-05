-- Read-only preflight for the Sequelize-managed PostgreSQL schema.
-- Run with psql -v ON_ERROR_STOP=1 -X -f migrations/preflight/legacy_schema_audit.sql.
-- Store the output with the deployment record before enabling a domain migration.

BEGIN TRANSACTION READ ONLY;

SELECT current_database() AS database_name,
       current_user AS database_user,
       current_setting('server_version') AS server_version,
       now() AS audited_at;

-- Tables expected by the active model registry, plus the inactive heartbeats candidate.
SELECT expected.table_name,
       to_regclass('public.' || expected.table_name) IS NOT NULL AS exists
FROM unnest(ARRAY[
    'devices',
    'device_interfaces',
    'device_controllers',
    'device_controller_heartbeats',
    'device_controller_metrics',
    'device_action_queue',
    'gen5_mapping_queue',
    'test_executions',
    'execution_reports',
    'relays',
    'relay_channels',
    'heartbeats'
]) AS expected(table_name)
ORDER BY expected.table_name;

-- Duplicate natural keys. Every result set must be empty before adding constraints.
SELECT 'devices.deviceId' AS check_name, "deviceId" AS key, count(*) AS duplicate_count
FROM devices
GROUP BY "deviceId"
HAVING count(*) > 1
ORDER BY duplicate_count DESC, key
LIMIT 100;

SELECT 'device_interfaces.deviceId+interfaceId' AS check_name,
       "deviceId" || ':' || "interfaceId" AS key,
       count(*) AS duplicate_count
FROM device_interfaces
GROUP BY "deviceId", "interfaceId"
HAVING count(*) > 1
ORDER BY duplicate_count DESC, key
LIMIT 100;

SELECT 'relays.serialNumber' AS check_name, "serialNumber" AS key,
       count(*) AS duplicate_count
FROM relays
WHERE "deletedAt" IS NULL
GROUP BY "serialNumber"
HAVING count(*) > 1
ORDER BY duplicate_count DESC, key
LIMIT 100;

SELECT 'execution_reports.testExecutionId' AS check_name, "testExecutionId" AS key,
       count(*) AS duplicate_count
FROM execution_reports
GROUP BY "testExecutionId"
HAVING count(*) > 1
ORDER BY duplicate_count DESC, key
LIMIT 100;

-- Orphans and loose references. Counts must be explained before adding foreign keys.
SELECT 'devices.controllerId' AS check_name, count(*) AS orphan_count
FROM devices child
LEFT JOIN device_controllers parent
  ON parent."deviceControllerId" = child."controllerId"
WHERE child."controllerId" IS NOT NULL AND parent."deviceControllerId" IS NULL
UNION ALL
SELECT 'device_interfaces.deviceId', count(*)
FROM device_interfaces child
LEFT JOIN devices parent ON parent."deviceId" = child."deviceId"
WHERE parent."deviceId" IS NULL
UNION ALL
SELECT 'relay_channels.relayId', count(*)
FROM relay_channels child
LEFT JOIN relays parent ON parent.id = child."relayId"
WHERE parent.id IS NULL
UNION ALL
SELECT 'relay_channels.deviceId', count(*)
FROM relay_channels child
LEFT JOIN devices parent ON parent."deviceId" = child."deviceId"
WHERE child."deviceId" IS NOT NULL AND parent."deviceId" IS NULL
UNION ALL
SELECT 'device_action_queue.testId', count(*)
FROM device_action_queue child
LEFT JOIN test_executions parent ON parent."testId" = child."testId"
WHERE child."testId" IS NOT NULL AND parent."testId" IS NULL
UNION ALL
SELECT 'device_action_queue.targetDeviceId', count(*)
FROM device_action_queue child
LEFT JOIN devices parent ON parent."deviceId" = child."targetDeviceId"
WHERE child."targetDeviceId" IS NOT NULL AND parent."deviceId" IS NULL
UNION ALL
SELECT 'execution_reports.testExecutionId', count(*)
FROM execution_reports child
LEFT JOIN test_executions parent ON parent."testId" = child."testExecutionId"
WHERE parent."testId" IS NULL
ORDER BY check_name;

-- Retention sizing and oldest/newest bounds for high-growth tables.
SELECT 'device_controller_heartbeats' AS table_name, count(*) AS row_count,
       min(timestamp) AS oldest, max(timestamp) AS newest
FROM device_controller_heartbeats
UNION ALL
SELECT 'device_controller_metrics', count(*), min("collectedAt"), max("collectedAt")
FROM device_controller_metrics
UNION ALL
SELECT 'log_entries', count(*), min(timestamp), max(timestamp)
FROM log_entries
UNION ALL
SELECT 'device_action_queue', count(*), min("updatedAt"), max("updatedAt")
FROM device_action_queue
UNION ALL
SELECT 'gen5_mapping_queue', count(*), min("updatedAt"), max("updatedAt")
FROM gen5_mapping_queue
UNION ALL
SELECT 'alerts', count(*), min("createdAt"), max("createdAt")
FROM alerts
ORDER BY table_name;

-- Capture actual production constraints and indexes; model files are not authoritative
-- because the legacy process used sync({ alter: true }).
SELECT table_name, constraint_name, constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public'
ORDER BY table_name, constraint_type, constraint_name;

SELECT tablename, indexname, indexdef
FROM pg_indexes
WHERE schemaname = 'public'
ORDER BY tablename, indexname;

ROLLBACK;