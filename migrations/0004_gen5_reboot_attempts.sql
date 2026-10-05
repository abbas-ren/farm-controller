CREATE TABLE IF NOT EXISTS farmcontroller.gen5_reboot_attempts (
    device_id text PRIMARY KEY,
    attempted_at timestamptz NOT NULL,
    controller_ip text NOT NULL,
    power_port text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS gen5_reboot_attempts_attempted_at_idx
    ON farmcontroller.gen5_reboot_attempts (attempted_at);

COMMENT ON TABLE farmcontroller.gen5_reboot_attempts IS
    'Durable single-attempt and grace-period state for the Gen5 stuck-device watchdog.';