-- Reconcile additive fields and control-plane metadata with legacy schemas
-- observed in production dumps.

DO $$
BEGIN
    IF to_regclass('public.device_controllers') IS NOT NULL THEN
        ALTER TABLE public.device_controllers
            ADD COLUMN IF NOT EXISTS "deletedAt" timestamptz;

        CREATE INDEX IF NOT EXISTS device_controllers_active_idx
            ON public.device_controllers ("deviceControllerId")
            WHERE "deletedAt" IS NULL;
    END IF;
END
$$;

UPDATE farmcontroller.retention_policies
SET timestamp_column = 'created_at', updated_at = now()
WHERE table_name = 'alerts' AND timestamp_column = 'createdAt';