CREATE TABLE IF NOT EXISTS pending_relay_configs (
    id uuid PRIMARY KEY,
    "relayChannelId" uuid NOT NULL,
    "relaySerial" text NOT NULL,
    "channelNo" integer NOT NULL CHECK ("channelNo" BETWEEN 0 AND 7),
    "deviceMac" text NOT NULL,
    "controllerAddress" text NOT NULL,
    "deviceGen" text,
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'confirmed', 'failed')),
    "createdAt" timestamptz NOT NULL DEFAULT now(),
    "updatedAt" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS pending_relay_configs_device_mac_idx
    ON pending_relay_configs (lower("deviceMac"));
CREATE INDEX IF NOT EXISTS pending_relay_configs_status_idx
    ON pending_relay_configs (status);
