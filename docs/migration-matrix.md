# FarmController Rust Migration Matrix

This matrix is the implementation ledger for the single-process Rust migration. Status values are `inventory`, `in progress`, `ported`, `replaced`, `obsolete`, and `external`.

| Existing component | Existing location | Rust location | Status | Compatibility notes | Verification |
|---|---|---|---|---|---|
| Unified HTTP/operational API | gateway and service entry points | `crates/service/src/api/mod.rs` | ported | One listener serves health, readiness, metrics, OpenAPI, Swagger aliases, APIs, Socket.IO, and native WebSocket upgrades | Router tests and same-listener smoke test |
| API gateway compatibility | `api-gateway/src/index.ts` | `crates/service/src/api/mod.rs`, `crates/service/src/devices/routes.rs`, `deploy/nginx/farmcontroller.conf.example` | ported | Auth/device prefixes and streaming uploads mount directly; prebuilt frontend assets stay at the edge without Node.js runtime | Router/prefix/TUS tests; live Nginx pending |
| Authentication API | `auth-service/src` | `crates/service/src/auth/mod.rs` | ported | Active registration, password, webhook, validation, and user-administration routes are implemented | Keycloak mock-server tests; live realm pending |
| Device API | `device-service/src/routes`, `controllers`, `services` | `crates/service/src/devices` | ported | All inventoried HTTP routes are mounted; external/live database workflow validation remains | Focused API and consumer-shape tests |
| Reports API | `reports-service/app` | `crates/service/src/reports` | ported | Bounded legacy-lifetime jobs, durable terminal status, consolidated/sanity/performance artifacts, native graphs, combined HTML, Confluence operations, and progress events are implemented | Report API, filesystem artifact/failure fixtures, and HTTP mock tests |
| PostgreSQL persistence | `device-service/src/models`, `queries` | `crates/service/src/persistence`, `crates/service/src/devices`, `migrations` | ported | Legacy tables/enums are preserved with additive SQLx migrations, transactions, readiness, and bounded pool metrics | Unit tests plus environment-gated live PostgreSQL migration tests using `TEST_DATABASE_URL` |
| Keycloak integration | `auth-service/src/config`, `services`, `dev-realm-realm.json`, `keycloak-providers` | `crates/service/src/auth/mod.rs` | ported | Password/client grants, userinfo, roles, introspection, refresh, admin CRUD, action mail, and webhook behavior are implemented | Keycloak HTTP mocks; live realm pending |
| Runtime module activation | legacy process topology | `crates/core/src/cli.rs`, `crates/core/src/config/mod.rs` | ported | One binary supports repeated/comma-separated `--enable` and `--disable`; invalid dependencies fail startup | Config precedence/dependency tests |
| Structured telemetry | service-specific loggers | `crates/service/src/observability.rs` | ported | Correlated text/JSON tracing, selectable streams, bounded file and UDP sinks, HTTP/worker/database/runtime metrics, and pool gauges | Metrics/router tests and UDP datagram test |
| EdgeController registration | `edgecontroller/src/runtime.rs`, `state.rs`, `models.rs` | `crates/service/src/devices/device_registration_handlers.rs`, `crates/service/src/devices/registration_store.rs` | ported | Public trailing-slash registration and controller identity contracts are retained | EdgeController fixture contract test |
| EdgeController heartbeat socket | `edgecontroller/src/websocket.rs`, `models.rs` | `crates/service/src/events/edgecontroller.rs`, `crates/service/src/workers/device_heartbeat.rs` | ported | `/ws?deviceControllerId=...`, protocol isolation, binary JSON heartbeat, timeout/recovery state | WebSocket and transition tests |
| Device action queue | `device-service/src/services/DeviceActionQueueService.ts` | `crates/service/src/workers/device_action.rs`, `crates/service/src/workers/device_action_watchdog.rs` | ported | Durable leases, bounded dispatch/retries, compensation, committed events, and stale-job recovery | Worker state and cancellation tests |
| Heartbeat queues | `device-service/src/services/HeartbeatQueueService.ts`, `ControllerHeartbeatQueueService.ts` | `crates/service/src/workers/device_heartbeat.rs`, `crates/service/src/workers/mod.rs` | ported | Persisted timeout/recovery transitions and alerts are lifecycle-owned | Clock-controlled worker tests |
| Gen5 mapping queue | `device-service/src/services/Gen5MappingQueueService.ts` | `crates/service/src/workers/gen5_mapping.rs` | ported | Durable retries, callback handling, inventory retrigger, and expiry are implemented | Callback and worker tests |
| Relay configuration timeout | `device-service/src/services/RelayConfigTimeoutService.ts` | `crates/service/src/workers/mod.rs` | ported | Pending configurations expire through an owned bounded worker | Clock-controlled timeout tests |
| Gen5 reboot watchdog | `device-service/src/services/Gen5RebootWatchdogService.ts` | `crates/service/src/workers/gen5_reboot_watchdog.rs` | ported | Persisted attempts, controller recovery, grace expiry, and atomic compensation are implemented | Watchdog tests |
| Artifact scanning | `device-service/src/workers/ArtifactsScanner`, `ArtifactsWorker` | `crates/service/src/workers/artifact_scanner.rs`, `artifact_preparation.rs`, `build_preparation.rs`, `ipl_distribution.rs` | ported | Structured HTTP discovery, durable root/nested IPL artifacts, cleanup, NFS/TFTP staging, official dispatch, controller backfill, and IPL cleanup are implemented | HTTP mock and temporary-filesystem tests; live controller pending |
| Test execution workers | `device-service/src/workers/TestWorker`, `controllers/TestCronController.ts` | `crates/service/src/workers/test_preparation.rs`, `device_action.rs`, `crates/service/src/test_results.rs` | ported | Preparation, scheduling, dispatch, fallback, completion, TestRail, and Jira paths are implemented | Worker and external-client mocks; live systems pending |
| TUS uploads | `device-service/src/config/tusConfig.ts` | `crates/service/src/devices/tus_handlers.rs`, `tus_store.rs` | ported | Resumable headers, offsets, limits, append locks, rollback, finalization, and OpenAPI are implemented | TUS protocol contract tests |
| Multipart uploads | `device-service/src/middlewares/upload.ts`, `controllers/BuildController.ts` | `crates/service/src/devices/build_handlers.rs`, `build_store.rs`, `build_ingestion.rs` | ported | Streaming official/custom upload, ZIP controls, staging, events, IPL, and official workflow behavior are implemented | Multipart, ZIP, and focused workflow tests |
| Socket.IO frontend events | `device-service/src/config/socketIoConfig.ts`, `custom-events`, `reports-service/app` | `crates/service/src/events` | ported | Socket.IO 4.x transport, auth, rooms, active producers, build fan-out, system metrics, and pending-user delivery are verified | Frontend event-name/payload/room tests; live frontend pending |
| WebCLI and terminal sockets | `device-service/src/config/webSocketConfig.ts`, `cli` | `crates/service/src/events/web_cli.rs`, `crates/service/src/events/terminal.rs` | ported | Authenticated inventory-authorized targets, server-owned credentials, known-host controls, and frontend envelopes are retained | WebSocket and command/parser tests |
| TestRail/JIRA/GitLab/Qmetry | `device-service/src/utils` | `crates/integrations/src/test_catalog`, `crates/integrations/src/jira.rs`, `crates/integrations/src/qmetry_catalog` | external | Native bounded clients and workflow integration are implemented | Mock-server tests; live systems pending |
| Confluence | `reports-service/app/services/confluence_uploader.py` | `crates/service/src/reports` | external | Page lookup/create/update, generated-image macro conversion, and attachment behavior are implemented | Mock-server and report tests; live Confluence pending |
| Nginx routing/static files | `nginx/nginx.conf` | `crates/service/src/api` and deployment config | inventory | API routing moves into the one listener; static frontend hosting decision remains deployment-configurable | Deployment smoke test |
| Linux deployment | `deploy`, `docker-compose*.yml` | `deploy/systemd`, `Dockerfile.rust`, `docker-compose.rust.yml` | ported | Unified backend runtime requires no Node.js or Python service | Artifact review; Docker/systemd smoke tests environment-blocked |

## Configuration inventory

The Rust service will apply `defaults < TOML file < environment < CLI`. Existing database, Keycloak, SMTP, TestRail, Jira, GitLab, Qmetry, Confluence, artifact, NFS, test-result, timeout, and listener settings remain migration inputs. Secrets must be accepted through environment variables or protected files and must never appear in startup output.

Implemented environment nesting uses a double underscore, for example `FARMCONTROLLER__AUTH__CLIENT_SECRET`. CLI-specific aliases include `FARMCONTROLLER_CONFIG`, `FARMCONTROLLER_BIND`, `FARMCONTROLLER_LOG_LEVEL`, `FARMCONTROLLER_LOG_FILE`, and `FARMCONTROLLER_LOG_JSON`.

Runtime modules required by the specification are `api`, `auth`, `device`, `reports`, `workers`, `events`, `metrics`, `swagger`, and `health`. They are modules in one binary and one process. Invalid dependencies, such as `device` without `api` or `workers` without their owning domain modules, must fail during startup validation.

## Migration decisions

- Keep Keycloak as an external identity provider during parity migration. Replacing it would alter password, refresh-token, role, and administrative behavior.
- Remove API-gateway HTTP hops inside the Rust process while retaining its externally visible `/api/v1/{auth,device}` prefixes.
- Serve APIs, WebSocket upgrades, health, readiness, metrics, OpenAPI, and Swagger from one Axum listener.
- Treat the production PostgreSQL catalog and preflight output as authoritative. Preserve domain data initially; apply only explicit, evidence-backed migrations with compatibility notes and tested rollback boundaries.
- Preserve Socket.IO compatibility until frontend usage is either implemented or intentionally migrated with coordinated frontend changes.
- Keep old services in the repository until the corresponding row is `ported` and its listed verification passes.

## Current validation

The current Rust workspace gate passes `cargo fmt --all -- --check`, `cargo test --workspace` (148 passed), `cargo check --workspace --all-targets --all-features`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`. A prior live process smoke test verified `/health`, `/ready`, `/metrics`, `/openapi.json`, and `/swagger-ui/` on one listener. Live infrastructure and consumer validation still prevent declaring the operational migration complete.

## Evidence reviewed

- Service manifests, entry points, routers, controllers, services, models, workers, validations, configuration, tests, and database initialization.
- `racer-e2e-testcases`, requirement/design documents, deployment scripts, Docker Compose, Nginx, README, and operational docs.
- EdgeController runtime registration, models, HTTP callbacks, WebSocket heartbeat transport, configuration, and tests.
- Frontend Axios configuration, service modules, Redux sagas, hooks, socket clients, and TypeScript DTOs.
