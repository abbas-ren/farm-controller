-- Align pending_relay_configs.status with Sequelize:
-- enum_pending_relay_configs_status ('pending', 'confirmed', 'failed').
-- Fresh installs from 0003 created a text column with a CHECK constraint;
-- SeaORM ActiveEnum casts require the real PG enum type.

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_type WHERE typname = 'enum_pending_relay_configs_status'
    ) THEN
        CREATE TYPE enum_pending_relay_configs_status AS ENUM (
            'pending',
            'confirmed',
            'failed'
        );
    END IF;
END
$$;

ALTER TABLE pending_relay_configs
    DROP CONSTRAINT IF EXISTS pending_relay_configs_status_check;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'pending_relay_configs'
          AND column_name = 'status'
          AND udt_name = 'text'
    ) THEN
        ALTER TABLE pending_relay_configs
            ALTER COLUMN status DROP DEFAULT;
        ALTER TABLE pending_relay_configs
            ALTER COLUMN status TYPE enum_pending_relay_configs_status
            USING status::enum_pending_relay_configs_status;
        ALTER TABLE pending_relay_configs
            ALTER COLUMN status SET DEFAULT 'pending'::enum_pending_relay_configs_status;
    END IF;
END
$$;
