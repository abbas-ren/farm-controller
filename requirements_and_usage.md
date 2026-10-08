# FarmController, EdgeController, and EdgeAgent Requirements and Usage

This document describes the hosts, services, files, paths, hardware, network access, configuration, startup, and routine operations required for a complete FarmController deployment with one or more EdgeControllers and target EdgeAgents.

## 1. Deployment model

Use four trust zones:

1. **FarmController host**: runs the central Rust service, PostgreSQL-backed workflows, authentication integration, API, WebSocket/Socket.IO endpoints, workers, reports, and the administrator control plane.
2. **EdgeController host(s)**: one Linux host beside each hardware setup. EdgeController owns relay, GPIO, CPLD, UART, USB discovery, and firmware operations.
3. **Target device(s)**: each board runs the Rust EdgeAgent. EdgeAgent owns target registration, inventory heartbeat, NFS boot configuration, testcase execution, result reporting, and target-local logs.
4. **Browser/frontend**: connects only to FarmController. FarmController resolves approved EdgeController and EdgeAgent addresses and proxies administrator log and control requests. The browser must not connect directly to either edge service.

FarmController is the only component permitted to initiate SSH sessions to targets or controller hosts. Browser messages never supply SSH usernames, passwords, or arbitrary target addresses.

No workflow depends on `hardware.json`. Hardware generation and capabilities come from the EdgeController configuration, startup flags, persisted controller/device records, and verified hardware discovery.

## 2. Baseline requirements

### FarmController host

- Linux with systemd for the packaged production service, or Docker Engine with Compose for the container deployment.
- The repository's pinned Rust toolchain, Cargo, a C toolchain, and CMake when building from source.
- PostgreSQL 17 is used by the supplied Compose deployment. A compatible reachable PostgreSQL service is required whenever `device`, `reports`, or `workers` is enabled.
- Keycloak 24.0.1 is used by the supplied Compose deployment. A configured Keycloak realm/client is required when `auth` is enabled.
- Read/write access to every path enabled by the selected workflows.
- Network access to each approved EdgeController on TCP 8888.
- SSH access to approved targets/controllers on TCP 22 when terminal or IPL transfer workflows are used.

### EdgeController host

- Linux with access to `/sys`, `/dev/ttyUSB*` or `/dev/ttyACM*`, and the selected hardware interfaces.
- systemd for the supplied `dev-con.service` deployment.
- USB access to the relay board for Gen3/Gen4.
- GPIO character-device access for Gen3/Gen4 IPL mode control.
- USB UART access for every supported generation.
- Gen5 CPLD power/download-control TTY access for Gen5. Gen5 does not use relay or GPIO control.
- A stable network interface with an IPv4 address and connectivity to FarmController.

### EdgeAgent target

- Linux with systemd, the board's valid `/etc/fw_env.config`, and `fw_printenv`/`fw_setenv` from `u-boot-fw-utils`.
- A stable configured network interface with a MAC address, IPv4 address, and HTTP(S)/WebSocket connectivity to FarmController.
- A writable persistent eMMC mount at `/mnt/emmc` and writable NFS testcase root at `/usr/src/tests`.
- Root service privileges for U-Boot updates, reboot, kernel logs, sysfs inventory, and testcase artifacts.
- Rust EdgeAgent installed as `/usr/bin/edgeagent-rs`; do not run legacy `DeviceAgent` concurrently.

### Frontend/operator workstation

- A modern browser that can reach FarmController over HTTP(S), native WebSocket, and Socket.IO through the same reverse proxy.
- For a source build: Node.js and npm versions compatible with the frontend repository's `package.json`.

## 3. Required and conditional services

| Service | Where | Required when | Default endpoint |
| --- | --- | --- | --- |
| `farmcontroller.service` | Farm host | Always | TCP `0.0.0.0:3000` |
| `dev-con.service` | Every Edge host | Always for hardware operations | TCP `0.0.0.0:8888` |
| `edgeagent-rs.service` | Every target device | Always for target registration and tests | TCP `0.0.0.0:8888` |
| PostgreSQL | Farm network | `device`, `reports`, or `workers` is enabled | TCP 5432 |
| Keycloak | Farm network | `auth` is enabled | Compose exposes TCP 9000 |
| TFTP server | Farm/device network | Network boot artifacts are used | UDP 69 |
| NFS server/mount | Farm/device network | Test preparation uses NFS staging | Deployment-specific |
| SSH server | Target/controller host | Terminal or SFTP IPL distribution is used | TCP 22 |
| Reverse proxy/frontend server | Browser-facing host | Production browser deployment | Usually TCP 443 |
| GitLab/TestRail/Qmetry/Jira/Confluence | External | Corresponding integration is configured | Deployment-specific HTTPS |

FarmController also exposes `/metrics` on its main listener. EdgeController exposes Prometheus metrics on TCP 8081 by default.

Do not run legacy dispatch workers and the Rust workers against the same database concurrently.

## 4. Network flows and ports

Allow only the flows needed by enabled features:

| Source | Destination | Port/protocol | Purpose |
| --- | --- | --- | --- |
| Browser/reverse proxy | FarmController | 3000/TCP | REST, Swagger, health, native WebSocket, Socket.IO |
| FarmController | PostgreSQL | 5432/TCP | Persistence and migrations |
| FarmController/browser | Keycloak | Configured HTTP(S) port | Login, token, and role validation |
| FarmController | EdgeController | 8888/TCP | Hardware API, logs, and proxied control |
| FarmController | approved EdgeAgent | 8888/TCP | Proxied target logs and runtime log-level control |
| EdgeAgent | FarmController | 3000/TCP or production HTTPS | Registration, `/ws` heartbeat/results, and callbacks |
| FarmController | approved SSH hosts | 22/TCP | Administrator terminal and SFTP IPL transfer |
| Devices | TFTP server | 69/UDP | Network-boot artifact retrieval |
| Prometheus | EdgeController | 8081/TCP | Edge metrics |
| Controller/device clients | FarmController | 3000/TCP | `/ws` and callbacks |

At a reverse proxy, route `/api`, `/health`, `/ready`, `/metrics`, `/openapi.json`, `/swagger*`, `/ws`, and `/socket.io` to FarmController. Preserve WebSocket upgrade headers. Native clients use `/ws`; browser Socket.IO uses `/socket.io` and must not be rewritten to `/ws`.

## 5. FarmController files and paths

### Administrator-provided files

| Path | Purpose | Recommended owner/mode |
| --- | --- | --- |
| `/usr/local/bin/farmcontroller` | Release binary | `root:root`, `0755` |
| `/etc/farmcontroller/config.toml` | Non-secret production configuration | `root:farmcontroller`, `0640` |
| `/etc/farmcontroller/farmcontroller.env` | Database URL, client secrets, SSH/SFTP passwords, and tokens | `root:farmcontroller`, `0640` or stricter |
| `/etc/farmcontroller/ssh_known_hosts` | Trusted SSH host keys | `root:farmcontroller`, `0640` |

The known-hosts file is required when `device.terminal_accept_unknown_host_keys = false`, which is the recommended production setting.

### Automatically managed paths

The supplied systemd unit creates these paths:

| Path | Purpose |
| --- | --- |
| `/var/lib/farmcontroller/` | Working and state directory |
| `/var/lib/farmcontroller/admin-control.json` | Mode-`0600` staged administrator settings and Edge tokens |
| `/var/log/farmcontroller/` | Log directory |

Do not hand-edit `admin-control.json` while the service is running. Use the administrator API/UI, then restart when the control response reports that a restart is pending.

### Workflow-specific writable paths

| Default path | Required when |
| --- | --- |
| `/nfs_share` | NFS test staging is used |
| `/tftp_data` | TFTP/network-boot output is used |
| `/artifacts` | Persistent build artifacts are used |
| `/test-results` | Test execution/report collection is enabled |
| `/Racer_IPL_flash` | Gen4/Gen5 IPL payloads are stored |
| `/Racer_IPL_flash/gen5_ipl.sh` | The configured Gen5 IPL script is used |
| `/var/lib/farmcontroller/uploads` | Build uploads are enabled |
| `/var/lib/farmcontroller/uploads/faulty-reports` | Faulty-report uploads are enabled |

The actual paths are configurable. If they change, update both `config.toml` and the systemd unit's `ReadWritePaths` allowlist. Parent directories must be writable by the `farmcontroller` account.

## 6. EdgeController files and paths

### Administrator-provided files

| Path | Purpose | Recommended owner/mode |
| --- | --- | --- |
| `/usr/bin/edgecontroller` | Release binary | `root:root`, `0755` |
| `/etc/config/login.cfg` | Strict legacy `key=value` configuration | `root:root`, `0600` |
| `/etc/default/edgecontroller` | Optional systemd environment and `EDGE_CONTROLLER_ARGS` override | `root:root`, `0600` |

`login.cfg` is not TOML. It permits blank lines and whole-line `#` or `;` comments, but rejects unknown keys, duplicate keys, quotes, inline comments, interpolation, and files over 16 KiB.

### Automatically managed files and directories

| Path | Purpose |
| --- | --- |
| `/var/lib/dev-controller/` | systemd state directory |
| `/var/lib/dev-controller/admin-control.json` | Mode-`0600` staged control settings |
| `/var/lib/dev-controller/firmware/` | Administrator-controlled firmware package root |
| `/var/lib/dev-controller/ipl/` | IPL operation logs/state |
| `/var/lib/dev-controller/captures/` | RTOS/serial captures |
| `/var/log/uid` | Persisted controller UUID |
| `/var/log/usb_mapping.csv` | Gen3/Gen4 relay-to-UART mappings |
| `/var/log/gen5_mapping.csv` | Gen5 UART/power mappings |
| `/var/log/gen5_uart.csv` | Legacy Gen5 UART state |
| `/var/log/gen5_power.csv` | Legacy Gen5 power state |
| `/etc/log/uart_mappings.csv` | Physically verified UART topology for all generations |

The supplied unit creates `/var/lib/dev-controller` and `/etc/log`. Mapping files are optional at first boot and are atomically created or replaced as mappings are verified. Their parent directories must be writable by the service account.

The administrator control plane may move mapping files only below `/var/log`, `/var/lib/dev-controller`, or `/etc/log`.

## 7. EdgeAgent files and paths

| Path | Purpose | Recommended owner/mode |
| --- | --- | --- |
| `/usr/bin/edgeagent-rs` | Rust target agent binary | `root:root`, `0755` |
| `/etc/edgeagent/config.toml` | Strict non-secret service configuration | `root:root`, `0600` |
| `/etc/edgeagent/environment` | Optional `EDGEAGENT_API_TOKEN` override | `root:root`, `0600` |
| `/mnt/emmc/UID.txt` | Atomically persisted approved MAC-derived device ID | `root:root`, `0600` |
| `/mnt/emmc/timeout` | Atomically persisted live heartbeat interval | `root:root`, `0600` |
| `/usr/src/currentTest.txt` | Reboot-spanning active test ID | `root:root`, `0600` |
| `/usr/src/tests/<testId>/<caseId>.sh` | NFS testcase executable | deployment-specific |
| `/usr/src/tests/<testId>/<caseId>.output` | Combined testcase output and result footer | created by agent |
| `/usr/src/tests/<testId>/<caseId>.txt` | Captured kernel log | created by agent |

The EdgeAgent source tree includes `config/default.toml`, a hardened systemd unit under `packaging/systemd`, a Yocto recipe template under `packaging/yocto`, and its complete migration/API guide in `README.md`. The configured `security.api_token` must match FarmController's `EDGE_AGENT_TOKEN`. If no token is configured, EdgeAgent accepts only loopback or the literal FarmController IPv4 source.

## 8. Hardware requirements by generation

### Gen3 and Gen4

- Start EdgeController with the matching `--enable-gen3` and/or `--enable-gen4` flag.
- Configure a relay USB VID:PID using `--vid-pid VVVV:PPPP`.
- If multiple matching relays exist, set `--relay-serial-number SERIAL`; otherwise startup discovery can select the unique device serial.
- Relay channels are numbered 0 through 7.
- GPIO values are Linux gpiochip line offsets, not physical Raspberry Pi header pin numbers.
- FarmController keeps UART configuration disabled until relay/GPIO configuration has been confirmed for the target.

### Gen5

- Start EdgeController with `--enable-gen5`.
- Do not supply relay or GPIO information for Gen5 UART configuration.
- Provide the Gen5 UART and CPLD power/download-control USB TTY devices.
- A Gen5 firmware package must be a real directory below `/var/lib/dev-controller/firmware`; symlink escapes are rejected.
- Gen5 UART mapping rows contain `-` for relay serial and relay channel.

### UART physical verification

UART setup is a physical verification workflow, not a static filename assignment. EdgeController inventories USB topology, requires the selected UART to disconnect and reconnect, and confirms that the reconnected device retains the expected USB identity and hub topology. Keep USB hubs, ports, and cabling unchanged during verification.

## 9. Configuration examples

### Minimal EdgeController `/etc/config/login.cfg`

```ini
# FarmController address without scheme or port
server_ip=192.0.2.10
http_port=3000
ws_port=3000

# Exactly 3, 4, or 5
gen=4

# Optional; defaults shown
iface_name=eth0
bind_port=8888
```

`server_ip` must be a usable IPv4 address, not `0.0.0.0`, multicast, or broadcast. All ports must be in `1..65535`; interface names are limited to valid Linux-style names of at most 15 characters.

### EdgeController systemd environment

Gen4 example:

```bash
EDGE_CONTROLLER_ARGS="--config-path /etc/config/login.cfg --enable-gen4 --enable-rtos --vid-pid 0403:6001 --relay-serial-number RELAY_SERIAL"
DEV_CONTROLLER_LOG_LEVEL=info
DEV_CONTROLLER_BIND=0.0.0.0
DEV_CONTROLLER_INTERFACE=eth0
```

Gen5 example:

```bash
EDGE_CONTROLLER_ARGS="--config-path /etc/config/login.cfg --enable-gen5 --enable-rtos"
DEV_CONTROLLER_LOG_LEVEL=info
DEV_CONTROLLER_BIND=0.0.0.0
DEV_CONTROLLER_INTERFACE=eth0
```

Optional Edge variables are `DEV_CONTROLLER_LOG_FILE`, `DEV_CONTROLLER_LOG_NETWORK`, and `DEV_CONTROLLER_LOG_STREAM`. API authentication is disabled by default. To enable it at startup, set `DEV_CONTROLLER_TOKEN` to 32 through 256 visible ASCII characters and configure the same per-controller token through FarmController's administrator control plane.

### FarmController configuration

Start from `config/production.toml.example`, install it as `/etc/farmcontroller/config.toml`, and select only the modules the deployment uses. A full deployment commonly enables:

```toml
[server]
bind = "0.0.0.0:3000"
request_body_limit_bytes = 10485760
cors_allowed_origins = ["https://farm.example"]

[modules]
enable = ["api", "auth", "device", "reports", "workers", "events", "metrics", "swagger", "health"]
disable = []

[auth]
keycloak_url = "https://identity.example"
realm = "dev-realm"
client_id = "dev-auth"
user_role = "user"
secure_cookies = true

[device]
server_ip = "192.0.2.10"
terminal_username = "root"
terminal_known_hosts_file = "/etc/farmcontroller/ssh_known_hosts"
terminal_accept_unknown_host_keys = false
nfs_host_path = "/nfs_share"
tftp_output_dir = "/tftp_data"
build_artifacts_dir = "/artifacts"
ipl_artifacts_dir = "/Racer_IPL_flash"
gen5_ipl_script = "/Racer_IPL_flash/gen5_ipl.sh"

[reports]
test_results_dir = "/test-results"

[logging]
level = "info"
json = true
stream = "stdout"
```

`device.server_ip` is retained for compatibility/default routing; normal multi-controller operations use approved controller addresses from persistence.

### FarmController protected environment

```bash
FARMCONTROLLER__DATABASE__URL=postgresql://farm_user:REPLACE_ME@db.example:5432/farmcontroller
FARMCONTROLLER__AUTH__CLIENT_SECRET=REPLACE_ME
FARMCONTROLLER__DEVICE__TERMINAL_PASSWORD=REPLACE_ME
FARMCONTROLLER__DEVICE__RTOS_PASSWORD=REPLACE_ME
FARMCONTROLLER__DEVICE__GEN4_IPL_USERNAME=
FARMCONTROLLER__DEVICE__GEN4_IPL_PASSWORD=
FARMCONTROLLER__DEVICE__GEN5_IPL_USERNAME=
FARMCONTROLLER__DEVICE__GEN5_IPL_PASSWORD=

# Startup fallback used only when Edge API authentication is enabled.
EDGE_CONTROLLER_TOKEN=REPLACE_WITH_THE_SAME_EDGE_TOKEN

# Must match EdgeAgent security.api_token or EDGEAGENT_API_TOKEN.
EDGE_AGENT_TOKEN=REPLACE_WITH_AT_LEAST_32_RANDOM_CHARACTERS
```

Configuration precedence is built-in defaults, TOML, environment, then CLI. Nested environment variables use `FARMCONTROLLER__SECTION__FIELD`. CLI aliases include `FARMCONTROLLER_CONFIG`, `FARMCONTROLLER_BIND`, `FARMCONTROLLER_LOG_LEVEL`, `FARMCONTROLLER_LOG_FILE`, `FARMCONTROLLER_LOG_JSON`, `FARMCONTROLLER_LOG_STREAM`, and `FARMCONTROLLER_LOG_NETWORK`.

Keep credentials out of TOML, shell history, source control, and API examples.

### Frontend build-time origins

The frontend requires all three values at build time:

```bash
VITE_API_BASE_URL=https://farm.example/api/v1
VITE_WEBSOCKET_BASE_URL=wss://farm.example/ws
VITE_SOCKET_IO_BASE_URL=https://farm.example
```

For local development against FarmController on port 3000, use `http://localhost:3000/api/v1`, `ws://localhost:3000/ws`, and `http://localhost:3000` respectively.

## 10. Installation and startup

### Build from source

FarmController:

```bash
cd /path/to/farm-controller
cargo build --release --workspace
sudo install -o root -g root -m 0755 target/release/farmcontroller /usr/local/bin/farmcontroller
```

EdgeController:

```bash
cd /path/to/edgecontroller
cargo build --release
sudo install -o root -g root -m 0755 target/release/edgecontroller /usr/bin/edgecontroller
```

EdgeAgent on each target:

```bash
cd /path/to/edgeagent-rs
cargo build --release --locked
sudo install -o root -g root -m 0755 target/release/edgeagent-rs /usr/bin/edgeagent-rs
sudo install -d -o root -g root -m 0700 /etc/edgeagent
sudo install -o root -g root -m 0600 config/default.toml /etc/edgeagent/config.toml
sudo install -o root -g root -m 0644 packaging/systemd/edgeagent-rs.service /usr/lib/systemd/system/
```

### Install and start systemd services

Create the FarmController account and configuration directories, install the repository's supplied units, then start in dependency order:

```bash
sudo useradd --system --home /var/lib/farmcontroller --shell /usr/sbin/nologin farmcontroller
sudo install -d -o root -g farmcontroller -m 0750 /etc/farmcontroller
sudo systemctl daemon-reload

# Start infrastructure first.
sudo systemctl start postgresql
sudo systemctl start keycloak

# Validate and start each edge before FarmController workflows use it.
sudo /usr/bin/edgecontroller --config-path /etc/config/login.cfg --check
sudo systemctl enable --now dev-con.service

# On every target, after editing config.toml and mounting /mnt/emmc:
sudo /usr/bin/edgeagent-rs --config /etc/edgeagent/config.toml --check
sudo systemctl enable --now edgeagent-rs.service

sudo systemctl enable --now farmcontroller.service
```

Adapt service names for the local PostgreSQL and Keycloak packages. `farmcontroller.service` uses `Restart=always`, which is required for its API-triggered self-restart. `dev-con.service` uses `Restart=on-failure`.

The Edge control API's restart action invokes `systemctl restart dev-con.service`; permit this only in a deployment whose service user and systemd policy intentionally support it.

### Docker Compose alternative for FarmController

The supplied `docker-compose.rust.yml` starts FarmController, PostgreSQL 17, Keycloak 24.0.1, and TFTP. Define all referenced secrets and host paths first, then run:

```bash
docker compose -f docker-compose.rust.yml config
docker compose -f docker-compose.rust.yml up -d --build
docker compose -f docker-compose.rust.yml ps
```

EdgeController remains a host-level hardware service because it requires direct device and sysfs access.

## 11. Validation checklist

### Before first start

```bash
# FarmController configuration and CLI surface
/usr/local/bin/farmcontroller --help

# Edge configuration and persisted mapping syntax without opening hardware
/usr/bin/edgecontroller --config-path /etc/config/login.cfg --check

# Target EdgeAgent configuration and identity
/usr/bin/edgeagent-rs --config /etc/edgeagent/config.toml --check

# Check service configuration
systemd-analyze verify /etc/systemd/system/farmcontroller.service
systemd-analyze verify /etc/systemd/system/dev-con.service
systemd-analyze verify /usr/lib/systemd/system/edgeagent-rs.service
```

Confirm:

- The PostgreSQL database exists and the configured user can create/use the required schema when migrations are enabled.
- Keycloak contains the configured realm, client, client secret, redirect origins, and administrator/user roles.
- Every configured writable path exists or can be created by the service account.
- The Farm host resolves and reaches every approved Edge IP on TCP 8888.
- Edge reaches the configured Farm IP/ports and sees the intended network interface.
- Each target reaches FarmController, exposes TCP 8888 only to FarmController, and uses the same EdgeAgent token on both services.
- Gen3/Gen4 relay VID:PID, serial, channels, GPIO lines, and permissions are correct.
- Gen5 UART and power/download-control TTYs are present and writable.
- Production SSH host keys are present in `ssh_known_hosts`.

### Runtime health

```bash
curl --fail http://farm.example:3000/health
curl --fail http://farm.example:3000/ready
curl --fail http://edge.example:8888/health
curl --fail http://edge.example:8888/ready
curl --fail http://target.example:8888/health
curl --fail http://target.example:8888/ready
curl --fail http://edge.example:8888/status
```

`/health` confirms that the process is alive. Use `/ready` to check dependency readiness before sending work.

FarmController API documentation is available at `/swagger-ui/` with JSON at `/openapi.json`. EdgeController documentation is available at `/docs` with JSON at `/swagger.json`.

## 12. Normal usage

### Frontend

After signing in, administrators use:

- `/home/control-center` for FarmController and EdgeController active/staged settings, feature flags, authentication, mapping maintenance, and restart actions.
- `/home/logs` for searchable FarmController or selected EdgeController runtime logs and live trace-level changes.
- `/home/terminals` for separate FarmController-host and EdgeController-host SSH terminals.

Selecting an EdgeController in these pages sends the controller database ID to FarmController. FarmController resolves the approved IP and performs the request or SSH connection.

### REST health and documentation

```bash
curl https://farm.example/health
curl https://farm.example/ready
curl https://farm.example/openapi.json -o farmcontroller-openapi.json
```

Most application routes require an access token:

```bash
curl -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  https://farm.example/api/v1/device/controller
```

Use Swagger as the executable reference for current request/response schemas.

### Administrator control snapshots

FarmController settings:

```bash
curl -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  'https://farm.example/api/v1/admin/control?source=farmcontroller'
```

A selected EdgeController, proxied by FarmController:

```bash
curl -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  'https://farm.example/api/v1/admin/control?source=edgecontroller&controllerId=<CONTROLLER_DATABASE_ID>'
```

Use `PATCH` on the same resource for typed settings and `POST` for allowlisted actions. Prefer the Control Center or Swagger for payload construction because the response advertises current capabilities and staged state. Runtime tracing and Edge API authentication changes apply immediately; settings marked restart-pending become active after a supervised restart. Returned secrets are always redacted.

Example FarmController patch that changes the trace level immediately and persists it for restart:

```bash
curl -X PATCH \
  -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  -H 'Content-Type: application/json' \
  'https://farm.example/api/v1/admin/control?source=farmcontroller' \
  -d '{"config":{"logging":{"level":"debug"}}}'
```

Example allowlisted Edge mapping reload through FarmController:

```bash
curl -X POST \
  -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  -H 'Content-Type: application/json' \
  'https://farm.example/api/v1/admin/control?source=edgecontroller&controllerId=<CONTROLLER_DATABASE_ID>' \
  -d '{"action":"reloadMappings"}'
```

Do not expose EdgeController directly to browsers. For local break-glass diagnosis on the trusted Edge host, its corresponding aggregate resource is `GET`, `PATCH`, or `POST http://127.0.0.1:8888/admin/control`.

### Runtime logs

FarmController logs:

```bash
curl -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  'https://farm.example/api/v1/operations/logs?source=farmcontroller'
```

Selected EdgeController logs:

```bash
curl -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  'https://farm.example/api/v1/operations/logs?source=edgecontroller&controllerId=<CONTROLLER_DATABASE_ID>'
```

Use `/api/v1/operations/logs/level` or the Logs page to change `trace`, `debug`, `info`, `warn`, or `error` at runtime. Avoid `trace`/`debug` longer than necessary on production systems.

```bash
curl -X PUT \
  -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  -H 'Content-Type: application/json' \
  https://farm.example/api/v1/operations/logs/level \
  -d '{"source":"farmcontroller","level":"debug"}'
```

For an EdgeController, use `{"source":"edgecontroller","controllerId":"<CONTROLLER_DATABASE_ID>","level":"debug"}`; FarmController proxies the change.

### UART verification

Send UART configuration to FarmController, not directly from the browser to EdgeController:

```bash
curl -X POST \
  -H 'Authorization: Bearer <ADMIN_BEARER_TOKEN>' \
  -H 'Content-Type: application/json' \
  https://farm.example/api/v1/device/uart/configure \
  -d '{
    "controllerId":"<CONTROLLER_DATABASE_ID>",
    "deviceId":"<DEVICE_DATABASE_ID>",
    "relayId":"<RELAY_UUID_FOR_GEN3_OR_GEN4>",
    "channelId":"<CHANNEL_UUID_FOR_GEN3_OR_GEN4>",
    "uartVidPid":"0403:6001"
  }'
```

For Gen5, set `relayId` and `channelId` to `null`. A successful response includes MAC, generation, VID:PID, resolved TTY, optional USB serial, interface number, topology, connection type, and `verified: true`. FarmController persists the verified result.

### Administrator terminals

The frontend opens native WebSocket mode `/ws?client=frontend&adminTerminal=true`. Only an authenticated administrator can use it. The alias `farmcontroller` resolves to loopback; Edge targets resolve only from approved database controller addresses. SSH credentials and host-key policy come from FarmController configuration.

An empty configured terminal password uses SSH `none` authentication. Configure a password only where required, and use a restricted account where possible.

## 13. Mapping files

Mapping files have no header row. EdgeController validates them at startup/reload and writes replacements atomically.

### `/var/log/usb_mapping.csv`

Gen3/Gen4 format:

```text
tty,mac,relay_serial,channel
```

Example row:

```csv
/dev/ttyUSB0,aabbccddeeff,RELAY123,0
```

### `/var/log/gen5_mapping.csv`

Gen5 format:

```text
uart_tty,power_tty,mac
```

Example row:

```csv
/dev/ttyUSB0,/dev/ttyUSB1,aabbccddeeff
```

### `/etc/log/uart_mappings.csv`

All generations use exactly 11 fields:

```text
mac,generation,tty,vid,pid,usb_serial,interface,topology,connection,relay_serial,channel
```

Examples:

```csv
aabbccddeeff,4,/dev/ttyUSB0,0403,6001,UART123,0,/sys/devices/platform/usb1/1-1,hub,RELAY123,0
112233445566,5,/dev/ttyACM0,1234,5678,UART999,1,/sys/devices/platform/usb2/2-1,standalone,-,-
```

Rules include:

- MAC addresses are canonical 12-digit lowercase hexadecimal values.
- TTYs are canonical `/dev/ttyUSBn` or `/dev/ttyACMn` paths.
- VID and PID are four hexadecimal digits each.
- `connection` is `standalone` or `hub`.
- Gen3/Gen4 require relay serial and channel 0 through 7.
- Gen5 requires `-` in both relay fields.
- Every UART row records generation, MAC, UART identity, interface, topology, and connection type.

Use the control-plane clear/reload actions instead of editing live files. If manual recovery is unavoidable, stop `dev-con.service`, back up the files, preserve ownership/mode, validate with `edgecontroller --check`, and then restart.

## 14. Security requirements

- Put secrets only in protected environment files or a secret manager.
- Use HTTPS/WSS at the browser boundary and exact production CORS origins.
- Keep EdgeController on a trusted management network. Its API authentication is disabled by default; enable a strong token where network isolation is insufficient.
- Do not expose mapping maintenance, restart, logs, or terminal APIs without FarmController administrator authorization.
- In production, set `terminal_accept_unknown_host_keys = false` and provision `/etc/farmcontroller/ssh_known_hosts`.
- Keep firmware and script roots administrator-owned. Do not permit untrusted users to replace files beneath them.
- Do not grant arbitrary command execution or arbitrary filesystem paths through the administrator control plane.
- Back up PostgreSQL, `/var/lib/farmcontroller/admin-control.json`, Edge mapping files, `/var/log/uid`, artifacts, and test results according to the site's recovery policy.

## 15. Troubleshooting

### FarmController is healthy but not ready

Check `/ready`, PostgreSQL reachability, database URL, migrations, Keycloak URL/client secret, enabled module dependencies, and structured logs. `device`, `reports`, or `workers` requires `database.url`; `auth`, `device`, and `reports` require `api`; `reports` requires `workers`.

### Controllers or devices disconnect

Verify `/ws` upgrade handling, do not route Socket.IO traffic to native `/ws`, check Edge-to-Farm IP/ports from `login.cfg`, and confirm the selected interface is up.

### EdgeController fails startup or `--check`

Check strict `login.cfg` syntax, staged mapping paths, mapping CSV field order, duplicate MAC/TTY entries, parent-directory permissions, and enabled generation flags. `--check` validates the currently selected administrator mapping paths.

### UART verification times out

Confirm the correct VID:PID, relay serial/channel for Gen3/Gen4, CPLD power control for Gen5, USB device permissions, and that the physical UART actually disconnects and reconnects on the same topology. Do not move the cable to another hub port during verification.

### Uploads, tests, or reports fail

Check configured size limits, free space, ownership, NFS/TFTP mounts, `/artifacts`, `/test-results`, IPL directories, and external integration URLs. Keep network-call timeouts bounded and do not print credentials while diagnosing.

### API-triggered restart does not return the service

FarmController's action returns HTTP 202 and then sends SIGTERM; its supervisor must restart it. EdgeController asks systemd to restart `dev-con.service`. Check unit names, restart policies, systemd permissions, and `journalctl -u farmcontroller.service` or `journalctl -u dev-con.service`.

## 16. Operational acceptance checklist

A deployment is fully operational when:

- FarmController `/health` and `/ready` succeed.
- Every EdgeController `/health`, `/ready`, and `/status` succeeds from the Farm host.
- PostgreSQL migrations complete and required modules start.
- Login and administrator role enforcement work through Keycloak when auth is enabled.
- The frontend reaches REST, native WebSocket, and Socket.IO endpoints through its configured origins.
- Each controller is registered/approved and its generation/features match attached hardware.
- Gen3/Gen4 relay and GPIO configuration is confirmed before UART controls become available.
- Gen5 power/download control works without relay/GPIO data.
- UART physical disconnect/reconnect verification succeeds and writes a complete 11-field row.
- FarmController can open administrator SSH terminals only to approved hosts.
- Mapping maintenance, runtime log level, Edge authentication, and supervised restart operations work from the administrator pages.
- Required artifacts, NFS/TFTP data, IPL packages, captures, and test results survive service restart.