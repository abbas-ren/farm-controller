-- The legacy service evolved its schema through Sequelize sync({ alter: true }).
-- This control plane is additive and establishes versioned ownership before
-- any domain table is transformed.

CREATE SCHEMA IF NOT EXISTS farmcontroller;

CREATE TABLE IF NOT EXISTS farmcontroller.schema_inventory (
    object_name text PRIMARY KEY,
    object_kind text NOT NULL CHECK (object_kind IN ('table', 'column', 'index', 'constraint')),
    domain_owner text NOT NULL,
    migration_status text NOT NULL CHECK (
        migration_status IN ('inventory', 'keep', 'merge', 'remove', 'rename', 'migrated')
    ),
    evidence text NOT NULL,
    reviewed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS farmcontroller.retention_policies (
    table_name text PRIMARY KEY,
    timestamp_column text NOT NULL,
    retention_days integer NOT NULL CHECK (retention_days > 0),
    batch_size integer NOT NULL DEFAULT 1000 CHECK (batch_size BETWEEN 1 AND 10000),
    enabled boolean NOT NULL DEFAULT false,
    archive_before_delete boolean NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (table_name ~ '^[a-z][a-z0-9_]*$'),
    CHECK (timestamp_column ~ '^[A-Za-z][A-Za-z0-9_]*$')
);

COMMENT ON TABLE farmcontroller.schema_inventory IS
    'Evidence ledger for legacy-to-unified schema decisions; not application-domain data.';
COMMENT ON TABLE farmcontroller.retention_policies IS
    'Operator-controlled cleanup policies. Policies are disabled until explicitly enabled.';

INSERT INTO farmcontroller.retention_policies (
    table_name,
    timestamp_column,
    retention_days,
    batch_size,
    enabled,
    archive_before_delete
) VALUES
    ('device_controller_heartbeats', 'timestamp', 30, 1000, false, false),
    ('device_controller_metrics', 'collectedAt', 90, 1000, false, false),
    ('device_state_change', 'changedAt', 365, 1000, false, true),
    ('log_entries', 'timestamp', 90, 1000, false, false),
    ('device_action_queue', 'updatedAt', 30, 500, false, false),
    ('gen5_mapping_queue', 'updatedAt', 30, 500, false, false),
    ('alerts', 'createdAt', 180, 500, false, false)
ON CONFLICT (table_name) DO NOTHING;

INSERT INTO farmcontroller.schema_inventory (
    object_name,
    object_kind,
    domain_owner,
    migration_status,
    evidence
) VALUES
    (
        'heartbeats',
        'table',
        'devices',
        'remove',
        'Defined by HeartbeatModel.ts but absent from dbInit.ts and models/index.ts; no active model or query usage.'
    ),
    (
        'device_controller_heartbeats',
        'table',
        'devices',
        'keep',
        'Initialized by dbInit.ts and read/written by ControllerHeartbeatQueueService.ts.'
    )
ON CONFLICT (object_name) DO UPDATE SET
    object_kind = EXCLUDED.object_kind,
    domain_owner = EXCLUDED.domain_owner,
    migration_status = EXCLUDED.migration_status,
    evidence = EXCLUDED.evidence,
    reviewed_at = now();