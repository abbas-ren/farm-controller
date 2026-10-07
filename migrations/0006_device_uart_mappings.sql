CREATE TABLE IF NOT EXISTS device_uart_mappings (
    id uuid PRIMARY KEY,
    "deviceId" text NOT NULL UNIQUE,
    "controllerId" text NOT NULL,
    "relayChannelId" uuid,
    "deviceMac" text NOT NULL,
    generation smallint NOT NULL CHECK (generation IN (3, 4, 5)),
    "requestedVidPid" text NOT NULL CHECK ("requestedVidPid" ~ '^[0-9a-f]{4}:[0-9a-f]{4}$'),
    tty text NOT NULL,
    "usbSerial" text,
    interface smallint NOT NULL CHECK (interface BETWEEN 0 AND 255),
    topology text NOT NULL,
    connection text NOT NULL CHECK (connection IN ('standalone', 'hub')),
    verified boolean NOT NULL DEFAULT true,
    "verifiedAt" timestamptz NOT NULL DEFAULT now(),
    "createdAt" timestamptz NOT NULL DEFAULT now(),
    "updatedAt" timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (generation IN (3, 4) AND "relayChannelId" IS NOT NULL)
        OR (generation = 5 AND "relayChannelId" IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS device_uart_mappings_device_mac_idx
    ON device_uart_mappings (lower("deviceMac"));
CREATE INDEX IF NOT EXISTS device_uart_mappings_controller_idx
    ON device_uart_mappings ("controllerId");