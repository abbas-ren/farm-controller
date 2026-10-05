-- Correct audit evidence using the active DeviceService constructor path.
-- Index changes are conditional because some installations may not have every
-- legacy table when the unified service first starts.

INSERT INTO farmcontroller.schema_inventory (
    object_name,
    object_kind,
    domain_owner,
    migration_status,
    evidence
) VALUES (
    'heartbeats',
    'table',
    'devices',
    'keep',
    'HeartbeatModel.ts is initialized dynamically by DeviceService.ts and queried by device list/detail heartbeat workflows.'
)
ON CONFLICT (object_name) DO UPDATE SET
    migration_status = EXCLUDED.migration_status,
    evidence = EXCLUDED.evidence,
    reviewed_at = now();

DO $$
BEGIN
    IF to_regclass('public.heartbeats') IS NOT NULL THEN
        CREATE INDEX IF NOT EXISTS heartbeats_device_timestamp_idx
            ON heartbeats ("deviceId", timestamp DESC);
    END IF;
    IF to_regclass('public.device_controller_heartbeats') IS NOT NULL THEN
        CREATE INDEX IF NOT EXISTS device_controller_heartbeats_controller_timestamp_idx
            ON device_controller_heartbeats ("controllerId", timestamp DESC);
    END IF;
    IF to_regclass('public.device_controller_metrics') IS NOT NULL THEN
        CREATE INDEX IF NOT EXISTS device_controller_metrics_controller_collected_idx
            ON device_controller_metrics ("controllerId", "collectedAt" DESC);
    END IF;
    IF to_regclass('public.device_interfaces') IS NOT NULL THEN
        CREATE INDEX IF NOT EXISTS device_interfaces_device_interface_idx
            ON device_interfaces ("deviceId", "interfaceId");
    END IF;
END
$$;