# Rust Migration Progress

This is the persistent execution checklist for the single-process Rust migration. `prompt.md` is authoritative. This document records verified implementation evidence; it does not prove completion by itself.

Last reconciled: 2026-10-06

## Status legend

- `[x]` implemented, validated, and documented for the current stage
- `[-]` partially implemented or validation/documentation incomplete
- `[ ]` not started
- `[B]` blocked only where required infrastructure is unavailable

## Handoff baseline

This is the authoritative implementation ledger. The FarmController root is a Git worktree; preserve unrelated user changes when reconciling future work. Reconciliation used the current Rust source, migration and compatibility documents, diagnostics, and executable checks.

`docs/migration-matrix.md` remains useful as the original component/configuration inventory, but many row statuses predate the current implementation. Use this document and `docs/api-compatibility.md` for current status, and verify both against source before changing code.

Current stage: **Local implementation parity and the planned service modularization are complete.** Gateway, auth, device, report, worker, and event behavior has been source-audited against the active legacy paths. Remaining work requires live infrastructure, deployment/consumer validation, or is explicitly tracked operational and database-modernization debt.

Current checkout health (2026-10-05):

- **PASS** `cargo fmt --all -- --check`.
- **PASS** `cargo check --workspace --all-targets --all-features`.
- **PASS** `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- **PASS** full workspace tests: 148 passed, 0 failed. Live PostgreSQL coverage still requires `TEST_DATABASE_URL`.
- Editor diagnostics additionally report legacy TypeScript setup issues: missing Jest types and deprecated `baseUrl`/`moduleResolution` options in `device-service/tsconfig.json`. These are legacy-tooling issues, not Rust errors.

## Current implementation map

Migration-created/current Rust surfaces include:

- Workspace/runtime: `Cargo.toml`, `crates/app`, `crates/core`, `crates/integrations`, and `crates/service`.
- Auth/reports/stateful features: `crates/service/src/{auth,reports,api,state,persistence,observability,workers,events}`. Auth and reports expose compatibility facades over focused handler, provider, orchestration, data-source, artifact, and publication modules.
- External adapters: `crates/integrations/src/{test_catalog,qmetry_catalog,jira.rs}`.
- Device root and routing: `crates/service/src/devices/{mod,routes,repository,repository_types,error,constants,validation}.rs`, with DTOs under `types/`, capability traits under `repository/`, and PostgreSQL implementations under `postgres/`.
- Device registration/controller/callback/heartbeat: `device_registration_*`, `registration_store.rs`, `controller_store.rs`, `callback_store.rs`, `heartbeat_store.rs`, `flashing_*`, `test_completion_*`.
- Builds/uploads/artifacts: `build_handlers.rs`, `build_store.rs`, `build_ingestion.rs`, `tus_handlers.rs`, `tus_store.rs`, `artifact_handlers.rs`, `artifacts.rs`, `csv_export.rs`, `device_export_*`.
- Tests/reports/catalog: `test_execution_handlers/`, `test_export.rs`, `test_export_handlers.rs`, and `test_catalog_handlers.rs`.
- Relays: `relay_handlers/`, `relay_store.rs`, `relay_configuration_store.rs`, and `legacy_relay_store.rs`.
- Analytics, notifications, faulty reports, and logs: dedicated `*_handlers.rs`/`*_store.rs` modules with repository methods and focused route tests.
- Database: `migrations/0001_*`, `0002_*`, `0003_*`, `0004_*`, and `migrations/preflight/legacy_schema_audit.sql`; a live migration/repository test harness remains pending.

Rust deployment deliverables are `Dockerfile.rust`, `docker-compose.rust.yml`, `deploy/systemd`, production configuration examples, and `dashboards/farmcontroller.json`. Root legacy deploy/Docker/Nginx assets remain available for rollback.

## Non-negotiable constraints

- [x] One Rust executable and one runtime process.
- [x] One Axum listener serves APIs, health/readiness, metrics, OpenAPI, Swagger, and WebSocket upgrades.
- [x] Runtime modules are selected through CLI/configuration.
- [-] Preserve all frontend and EdgeController contracts. Inventoried HTTP and event protocols have focused local coverage; live consumer verification remains unavailable.
- [x] Existing functionality is prioritized before extensions.
- [x] Generated OpenAPI registers every mounted HTTP/WebSocket operation with a summary, useful behavioral description, and success response. Stable response bodies, bounded errors, multipart/binary/CSV media, TUS headers, representative examples, and compatibility quirks are documented and regression-tested.
- [x] Prometheus HTTP/worker/database/runtime metrics and structured text/JSON tracing with stream, file, and optional UDP sinks exist.
- [x] Linux is the target and Rust 1.98/edition 2024 currently builds successfully.

## Original migration stages

Stages remain in the exact order defined by `prompt.md`. Work found in a later stage does not close an earlier stage gate.

### Step 1 - Workspace audit

Status: `[x]` initial audit complete; evidence must continue to be checked when each contract is ported.

Objectives:

- [x] Inspect `api-gateway`, `auth-service`, `device-service`, `reports-service`, deployment, tests, requirements, and operational documentation.
- [x] Inspect sibling `edgecontroller` and `frontend` consumers.
- [x] Inventory services, routes, models, PostgreSQL, integrations, workers, configuration, auth, tests, filesystem/network behavior, startup, and shutdown.
- [x] Identify cross-service HTTP, Keycloak, PostgreSQL, filesystem, WebSocket/Socket.IO, queue, and external integration flows.

Evidence:

- `docs/migration-matrix.md`
- `docs/api-compatibility.md`
- `docs/database-migration.md`
- Legacy route files under `auth-service/src/routes`, `device-service/src/routes`, and `reports-service/app/main.py`
- EdgeController registration, HTTP callback, relay, and binary heartbeat contracts were reviewed against Rust implementations.
- Frontend service modules were reviewed to identify active APIs and Socket.IO usage.

Limitations:

- This repository is not a Git worktree, so change reconciliation uses current file contents, diagnostics, and executable validation rather than `git status` or history.
- Detailed external integration behavior is revalidated when its owning feature is implemented.

### Step 2 - API and behavior contract

Status: `[x]` source-reconciled and locally contract-tested. Live consumer validation remains tracked in Step 7.

Completed:

- [x] Auth endpoint inventory and token/refresh compatibility notes.
- [x] Core device/controller endpoint inventory.
- [x] Build and test route inventory.
- [x] Reports HTTP endpoint inventory.
- [x] Unified operational endpoint inventory.
- [x] EdgeController registration, callback, relay, and heartbeat wire contracts used by implemented routes.
- [x] Remaining mounted device route paths and authorization are enumerated from current Express routers.
- [x] Device registration request/default/validation/identity/response/duplicate/transaction/side-effect contract and compatibility tests.
- [x] Device CSV export auth/query/header/column/date/error contract and compatibility tests.
- [x] Device flashing callback state/queue/version/response/side-effect contract and compatibility tests.
- [x] Test-completed callback ID/status/response/RTOS-log side-effect contract and compatibility tests.
- [x] Build-list authentication/filter/pagination/sort/response/error contract and compatibility test.
- [x] Build detail and distinct filter authentication/response/not-found/routing contracts and compatibility tests.
- [x] Build upload initialization auth/body/range/session/response/error contract and compatibility tests.
- [x] Retained relay CRUD/fresh/remap/conflict request, auth, response, soft-delete, cleanup, and hardware-resynchronization contracts with focused route tests.
- [x] Analytics authentication, query defaults/bounds, user scoping, historical unscoped exceptions, date windows, aggregation shapes, not-found behavior, and focused route tests.
- [x] Notification list visibility, pagination/defaults, read-state mutations, legacy missing-ID differences, response envelopes, and focused route tests.
- [x] Faulty-report multipart fields/limits, release/test fallback, file/log storage and downloads, detail enrichment, status/release mutation, hard delete, and focused route tests.

Incomplete completion criteria:

- [x] Record request fields, query/path/header parameters, success/error status codes, response schemas, side effects, and dependencies for every build endpoint.
- [x] Record the same contract detail for every test execution endpoint.
- [x] Record server-side `/ws/send/message` payload, response, emitted event name/envelope, and public access contract.
- [x] Record multipart/TUS size, offset, metadata, and error semantics in generated OpenAPI and compatibility documentation.
- [x] Record Socket.IO event names, rooms, payloads, and reconnect behavior. The shared client, auth, room joins, public `user` event, polling/WebSocket transport, and active domain payloads are verified.
- [x] Map active frontend and EdgeController hardcoded expectations to compatibility fixtures. Coverage includes power toggle, relay-device picker, heartbeat timeout, execution creation, report upload status, relay confirmation, build status/performance fan-out, system metrics, and pending-user registration.

Evidence: `docs/api-compatibility.md`.

Exact next action: add configurable production CORS allowlisting and baseline HTTP security headers while preserving the documented legacy permissive default for compatibility.

TUS trace checkpoint: wire metadata, 5 MiB frontend chunking/retries, 500 MiB default limit, file storage, relative locations, creation/completion events, and build-ingestion failure behavior are documented. Implementation must first add upload directory/max-size configuration, then protocol state/storage handlers, then finalization integration.

TUS implementation checkpoint: authenticated create/head/patch/options, durable offsets, append locking, limits, metadata, retry-safe rollback, safe ZIP staging/extraction, and transactional release/session finalization are implemented and tested without infrastructure. Live PostgreSQL finalization and Step 6.5 artifact/event/official-workflow side effects remain.

### Step 3 - Unified Rust architecture design

Status: `[x]` implemented, modularized, documented, and locally validated.

Implemented:

- [x] Central `AppState` dependency container.
- [x] Domain modules for auth, devices, reports, events, workers, persistence, and observability.
- [x] Repository/provider traits at PostgreSQL, identity, and report data boundaries.
- [x] Direct in-process routing replaces gateway-to-service HTTP hops.
- [x] Cancellation token controls worker and server shutdown.
- [x] Device domain progressively split into DTO, validation, repository, handler, and focused store modules.
- [x] Auth split into transport, session policy, provider, Keycloak operation, DTO, error, constant, and focused test modules.
- [x] Reports split into HTTP, orchestration, data-source, artifact, combined-report, graphing, Confluence, and focused test modules.
- [x] `DeviceRepository` split into feature capability traits with a composed object-safe facade; PostgreSQL and test adapters implement capabilities directly.

Remaining:

- [x] Record implemented architecture and open decisions in `docs/architecture-decisions.md`.
- [x] Add the final `docs/architecture.md` developer-facing dependency/ownership guide required by `prompt.md`.
- [x] Replace the oversized auth, reports, device, and test-execution modules with thin compatibility facades and responsibility-focused submodules while preserving focused contract tests.
- [x] Define an AppState-owned event transport boundary with bounded observation and lifecycle-owned Socket.IO delivery.
- [x] Avoid detached tasks without lifecycle ownership; task spawns are owned by startup signal handling, reports, or `WorkerManager`, including bounded relay synchronization.

### Step 4 - Bootstrap

Status: `[-]` implemented and validated, but stage documentation remains incomplete.

- [x] Cargo workspace, one binary, and one listener.
- [x] Clap CLI with repeated/comma-separated `--enable` and `--disable`.
- [x] TOML, environment, and CLI configuration loading.
- [x] Structured error responses and request IDs.
- [x] Console/JSON/file tracing.
- [x] Health, readiness, metrics, OpenAPI, and Swagger on the same listener.
- [x] SIGINT/SIGTERM graceful server and worker cancellation.
- [x] Optional nonblocking UDP network logging sink.
- [x] Selectable stdout/stderr/off stream logging sink.
- [x] Bootstrap/configuration/development documentation.

Evidence: `crates/app/src/main.rs`, `crates/core/src/cli.rs`, `crates/core/src/config/mod.rs`, `crates/service/src/api/mod.rs`, `crates/service/src/observability.rs`.

### Step 5 - Core infrastructure

Status: `[-]` partially complete.

Database and persistence:

- [x] SQLx PostgreSQL pool, startup migrations, readiness ping, parameterized static SQL, and transactions.
- [x] Additive control-plane migration and disabled retention policy (`0001`).
- [x] Evidence-backed heartbeat/metric/interface indexes (`0002`).
- [x] Durable pending relay configuration table and indexes (`0003`).
- [x] Durable Gen5 reboot attempts and indexes (`0004`).
- [x] Legacy schema preflight audit and database migration strategy documented.
- [ ] Add a live migration/repository harness guarded by a disposable PostgreSQL database in `TEST_DATABASE_URL`; no test database was available locally.
- [ ] Execute preflight against a production-shaped clone and capture catalog/query-plan evidence.
- [ ] Validate duplicate/orphan data before proposed constraints, partitioning, or normalization.
- [ ] Add repository tests for concurrency, rollback, constraints, timeouts, and partial failures.

Authentication/common infrastructure:

- [x] Keycloak password/client-credential flows, introspection, refresh, user administration, roles, action email, and webhook route.
- [x] Central authorization helper and frontend-compatible refresh headers/cookies.
- [x] Common body limit, CORS, panic handling, request ID propagation, and safe JSON errors.
- [x] Configurable production CORS allowlist with explicit legacy permissive default and baseline response security headers.
- [x] Central outbound HTTP policy for all Reqwest clients: bounded connect/request timeouts, redirects disabled, configured HTTP(S) URL validation without embedded credentials, and no implicit retries outside two shared bounded device-command loops.

### Step 6 - Port services in required order

Status: `[x]` locally complete. Ordering was gateway, auth, devices, reports, then workers/events; live integration verification remains tracked separately.

#### 6.1 API gateway behavior

Status: `[x]` locally implemented and documented; live edge deployment validation remains in Step 10.

- [x] `/api/v1/auth` and `/api/v1/device` mount in one router without internal proxy hops.
- [x] Request streaming remains possible through Axum body handling; global body limit is configurable.
- [x] CORS and correlation headers exist.
- [x] Legacy forwarded-prefix headers are unnecessary for direct in-process handlers.
- [x] Frontend static assets are served by edge Nginx/CDN without a Node.js runtime; the Rust process remains the only backend application server.
- [x] Multipart and TUS paths reach the unified listener without prefix rewriting or internal proxy hops; the edge example disables request buffering and retains the 500 MiB deployment limit.
- [x] Explicit gateway-prefix/TUS preflight compatibility test and unified Nginx deployment notes/example.

#### 6.2 Authentication service

Status: `[-]` active route set is implemented, modularized, and locally contract-tested; live realm deployment remains.

Implemented routes:

- [x] Sign-in; registration request/action; pending/all/active users; user lookup/delete; verify/reset/forgot password; admin registration; user event webhook; validate/user/admin.
- [x] Keycloak-backed behavior tested for sign-in, validation, registration, acceptance, pending users, forgot password, and admin creation.

Remaining:

- [x] Mounted handler matrices cover every auth route success plus every protected route's missing-auth and every request body's validation path; provider tests cover Keycloak success/upstream protocol behavior.
- [ ] Verify confirmation-mail webhook/provider deployment against a live Keycloak realm.
- [x] Refactor the oversized module into cohesive DTO/provider/handler/error/session/Keycloak modules with focused tests.

#### 6.3 Device service

Status: `[x]` active legacy behavior is locally implemented; live database, controller, and external-provider verification remains.

Implemented and validated:

- [x] Public device registration with MAC normalization, create/update/restore semantics, transactional interface synchronization, auto-approval, and artifact-folder creation.
- [x] Admin device CSV export with legacy filters, dynamic interface columns, date formatting, and download headers.
- [x] Public flashing callback with busy-state gating and queue-version upgrade/downgrade decisions.
- [x] Public test-completed callback with test-ID cleanup, terminal no-op behavior, Gen4/Gen5 RTOS-end transport, traversal-safe log storage, persisted `rtosLogPath`, and committed owner event.
- [x] Authenticated build inventory with legacy filters, pagination, allowlisted sorting, and frontend response shape.
- [x] Authenticated build detail and deterministic family/type filter discovery.
- [x] Authenticated build upload-session initialization with legacy row defaults and file-count validation.
- [-] Authenticated TUS protocol and build ingestion core; external/workflow side effects and live database validation remain.
- [-] Custom/official multipart upload HTTP, streaming, ingestion, events, IPL distribution, NFS/TFTP preparation, scanning, and official workflow dispatch are ported; live database/controller validation remains.
- [x] Build flag/delete HTTP, database/local-artifact behavior, alerts, Socket.IO events, and best-effort family-controller IPL cleanup.
- [x] Authenticated user-scoped test execution CSV export with legacy calculations and download contract.
- [x] Authenticated TestRail plan listing with bounded Basic-auth provider, same-origin pagination, filtering, and mock-backed tests.
- [x] Authenticated TestRail suite listing with required plan validation, same-origin pagination, projection, and mock-backed tests.
- [x] Authenticated TestRail testcase listing with required IDs, pagination/filtering, normalized scripts/preconditions, labels, and mock-backed tests.
- [x] Authenticated Qmetry plan/suite testcase paths with API-key provider, hierarchy/search projections, validation, and mock-backed tests.
- [x] Authenticated test execution detail/latest and `?table` summary with cases, duration/counts, user analytics, and configured logs.
- [x] Static single, public testcase-ID, latest-by-device, and in-progress execution read routes with ownership/log/status contracts.
- [x] Authenticated user-scoped paginated execution listing with device search, allowlisted sorting, computed counts/duration, and legacy table shape.
- [x] Authenticated execution testcase projection, testcase detail, and text-log attachment routes with legacy error semantics.
- [x] Authenticated execution report metadata/HTML and traversal-safe testcase-log download routes.
- [x] Authenticated execution creation for legacy and `ALL`/`PARTIAL` selections with catalog expansion, deduplication, atomic execution/case persistence, and legacy response shape.
- [x] Authenticated TestRail case-result update with legacy `201 {}` acknowledgement and optional failure logging.
- [x] Execution report generation/upload bridge to the native bounded report/Confluence service; live database status synchronization and live Confluence remain unverified.
- [x] Build-scoped execution pagination, results lookup, and phase-aware cancellation persistence/idempotency, queue cleanup, fallback dispatch, owner/build events, TestRail cleanup where legacy artifact cleanup applies, and deferred IPL/boot terminalization.
- [x] EdgeController registration and relay inventory replacement.
- [x] Gen5 mapping and flash confirmations.
- [x] Controller binary heartbeat WebSocket and persistence.
- [x] Device family/type/detail/heartbeat/topology and general/user/active/controller lists.
- [x] Device approve/decline, heartbeat timeout, controller edit/delete, and device delete.
- [x] Build discovery and artifact-folder configuration.
- [x] Default artifact streaming copy to NFS/TFTP with safe archive paths.
- [x] Device reboot, relay toggle/state/inventory/channels/available devices.
- [x] Relay configuration, clear-first move semantics, conflicts, pending confirmations, and EdgeController synchronization.
- [x] Retained relay CRUD, single-channel auto-unmapping configuration, duplicate cleanup/remap, conflict detection/cleanup, soft deletes, OpenAPI, and focused route coverage.
- [x] All fourteen authenticated analytics routes with legacy aggregation/scoping semantics, static parameterized SQL, OpenAPI, and focused route coverage.
- [x] All five notification routes with admin/user authorization, static parameterized SQL, OpenAPI, and focused route coverage.
- [x] All seven faulty-report routes with bounded multipart streaming, safe storage/downloads, static parameterized SQL, OpenAPI, and focused route coverage.

Ported but externally unverified:

- [-] Device callback HTTP/database parity, flashing state events, and RTOS-end log persistence are ported; flashing-triggered RTOS-start remains in Step 6.5.
- [-] Build API HTTP/TUS/multipart/ZIP/database core, alerts/events, uploaded and scanned IPL distribution, controller backfill, remote IPL deletion, NFS/TFTP preparation, scanner, and official workflow dispatch are ported; live database/controller validation remains.
- [-] TestRail/Qmetry and execution HTTP/database core are ported; preparation/queue workers, events, and live database/external validation remain.
- [x] Authenticated log create/list/search routes with defaults, validation, static parameterized PostgreSQL filtering/sorting/pagination, OpenAPI, and focused tests.
- [x] Public server-side user-message dispatch through an AppState-owned bounded event hub with verified Socket.IO 4.x browser delivery.

#### 6.4 Reports service

Status: `[x]` locally validated; live Confluence and legacy-shaped PostgreSQL verification remain external.

- [x] `/report`, `/report/{job_id}`, and `/upload-confluence` routes.
- [x] Bounded in-process queue, cancellable worker, status snapshots, report artifacts, path controls, and Confluence page/attachment operations.
- [x] Durable `execution_reports` row creation before enqueue, terminal status synchronization, and user/test-room `execution_report_update` events.
- [x] Preserve legacy process-local queue/status semantics; the Python `ReportQueue` also used an in-memory `asyncio.Queue` and `_job_status` dictionary, while durable execution status remains in PostgreSQL.
- [x] Verify escaped consolidated text/HTML, fatal sanity analysis, best-effort performance comparison, radar/category PNG, CSV/JSON summaries, combined report links/images, and terminal failure persistence/events with filesystem fixtures.
- [x] Refactor the reports implementation into cohesive HTTP, service, data-source, artifact, graph, and Confluence modules.

#### 6.5 Workers, events, and background functions

Status: `[-]` partial.

- [x] Cancellable retention worker with bounded batches and metrics.
- [x] Cancellable report worker.
- [x] EdgeController heartbeat WebSocket event handling.
- [x] Shared-listener Socket.IO 4.x polling/WebSocket transport, handshake auth, `user:{id}` membership, and build/test/dashboard/device/controller room join/leave compatibility.
- [x] Durable device action processing: preparation leases, TestRail/script/artifact preparation, preconditions, locked assignment, device dispatch retries, committed events, compensation, and stale-job watchdog are owned and bounded.
- [x] Device and controller heartbeat ingestion, persisted timeout/recovery workers, transactional alerts, and frontend ping/interface/state/controller events.
- [x] Gen5 mapping jobs are transactionally enqueued for newly approved Gen5 registrations; an owned bounded worker triggers active controllers, retries unavailable-controller failures, leases failure polls, retriggers on inventory growth, and expires unmapped devices.
- [x] Bounded relay synchronization queue and stale relay-configuration timeout worker with committed progress/failure events.
- [x] Durable Gen5 reboot watchdog with persisted attempts, controller power recovery, grace-period expiry, and atomic compensation.
- [x] Cancellable HTTP artifact scanner with structured traversal, bounded metadata/download requests, durable local required artifacts, checksums, cleanup policy, staging, official dispatch, and committed events.
- [-] Test execution scheduling/workers and TestRail/GitLab/Qmetry integrations; preparation, dispatch, native result completion, fallback dispatch, heartbeat transitions, completion-time TestRail synchronization, and failed-result Jira synchronization are ported. Live external validation remains.
- [x] Socket.IO core and active producers are implemented and contract-tested, including global build-status delivery, dashboard/build-room performance fan-out, five-second system metrics, and pending-user registration notifications. Live frontend validation remains in Step 7.
- [x] Native testcase-result, authenticated frontend SSH/RTOS, and authenticated WebCLI sessions are ported. SSH targets are inventory-authorized, production host keys use configured known hosts, and credentials are server-managed.
- [x] WebCLI device/build/plan/suite/case commands and interactive run-test prompts preserve the frontend prompt/link/log envelopes.
- [-] Current workers have bounded ownership, cancellation, retry/timeout handling, metrics, and focused tests; live external failure and shutdown scenarios remain incomplete.

### Step 7 - Compatibility verification

Status: `[ ]` blocked as a completion stage by Step 6 parity, with incremental checks already present.

- [-] EdgeController registration, callbacks, heartbeat, relay, power, and reboot contracts have focused tests or direct source evidence.
- [-] Frontend auth/device list/detail/filter contracts have focused tests.
- [ ] Run the frontend against the Rust service and capture results.
- [ ] Map and run applicable `racer-e2e-testcases`.
- [ ] Verify all status/error/pagination/file/event contracts.
- [ ] Document justified differences and obsolete legacy components.

### Step 8 - Comprehensive tests

Status: `[-]` incremental suite exists; full matrix is incomplete.

Current automated evidence (2026-10-02):

- [x] `cargo fmt --all -- --check` passes.
- [x] `cargo check --workspace --all-targets --all-features` passes.
- [x] Strict Clippy passes with `-D warnings`.
- [x] Full local tests pass: 148 passed, 0 failed. The pending live PostgreSQL harness is not included in this total.
- [x] Unit tests cover configuration, parsers, retention safety, auth provider flows, device wire contracts, relay validation/mapping, analytics, notification, faulty-report, Socket.IO producer contracts, reports, and worker shutdown.
- [x] Device registration has focused validation and anonymous HTTP contract tests; live create/update/restore transaction coverage is environment-blocked.
- [ ] API tests for every route/method/auth/error/malformed/oversized input.
- [ ] Integration tests for PostgreSQL, filesystem failures, external timeouts/retries, concurrency, rollback, and shutdown during active work.
- [ ] Full frontend, EdgeController, and E2E contract tests.

### Step 9 - Observability stabilization

Status: `[-]` partial.

- [x] Structured tracing, selectable stream/JSON/file/UDP output, request IDs, HTTP count/duration/status/active/size/failure metrics, uptime/memory, worker/report/database metrics, and pool gauges.
- [x] `/metrics` is on the shared listener.
- [x] Bounded HTTP failure, worker, database, runtime, and pool measurements.
- [x] Route templates rather than raw URLs are used for bounded HTTP metric labels.
- [x] Network and stream log destinations.
- [x] Prometheus scrape documentation.
- [x] Grafana dashboard definition under `dashboards/`.
- [x] Every generated operation has a useful description and success response; stable projections, constraints, side effects, protocol headers/media, representative examples, and bounded errors are documented where applicable.

### Step 10 - Linux deployment and documentation

Status: `[-]` artifacts and documentation exist; environment smoke validation remains.

- [x] Production systemd unit, service user, permissions, config/environment example, restart and health behavior.
- [x] Rust-only Dockerfile/Compose path if containers remain supported.
- [ ] Release build and deployment smoke test without Node.js/Python runtime services.
- [x] Required docs: architecture, configuration, API, development, deployment, observability, migration, compatibility, troubleshooting.
- [x] Root README leads with the unified Rust runtime while retaining legacy rollback instructions.
- [x] Rollback and legacy-service cutover runbook.

### Step 11 - Sensible extensions after parity

Status: `[ ]` deliberately deferred until Steps 1-10 are complete.

No speculative extension should displace missing legacy behavior. Any later extension requires purpose, auth analysis, OpenAPI, tests, metrics, frontend considerations, and EdgeController impact analysis.

## Database modernization ledger

| Migration/work item | Status | Evidence/limitation |
|---|---|---|
| `0001_persistence_control_plane.sql` | Applied by SQLx; unit-discovered | Additive namespace, evidence ledger, disabled retention policies |
| `0002_schema_evidence_and_indexes.sql` | Applied by SQLx; unit-discovered | Conditional indexes and corrected heartbeat evidence |
| `0003_pending_relay_configuration.sql` | Applied by SQLx; unit-discovered | Additive pending relay state and indexes |
| `0004_gen5_reboot_attempts.sql` | Applied by SQLx; unit-discovered | Durable bounded Gen5 reboot recovery state and indexes |
| Legacy schema preflight | Script/document ready | Must run against production clone |
| Live empty/legacy migration tests | Not started | Harness implementation and a disposable database ending in `_test` via `TEST_DATABASE_URL` are required |
| High-growth heartbeat/metric partitioning | Not started | Requires production-shaped sizes/plans |
| Constraint/deduplication work | Not started | Requires duplicate/orphan evidence and rollback artifacts |
| Retention execution | Implemented but disabled by default | Enable only after restore/retention approval |

## Known compatibility gaps

- No source-confirmed locally implementable legacy behavior gaps remain. All inventoried device HTTP routes, browser/terminal protocols, and device/build/test side effects are implemented; the items below are external verification risks.
- Device registration now persists the new-device admin alert and emits legacy addition/approval envelopes; best-effort Gen3 controller mapping remains.
- Device flashing now emits the legacy state-update event; best-effort RTOS-start for test jobs remains.
- Test completion performs best-effort Gen4/Gen5 RTOS-end retrieval, stores the bounded path under the configured results root, persists `rtosLogPath`, and publishes the owner event after persistence.
- Completed upload and scanner ingestion store Gen4/Gen5 IPL payloads and use known-host-verified native SFTP when scoped credentials are configured. Controller registration backfills stored family versions and build deletion removes local/remote versions. NFS/TFTP, official flash workflows, and upload events are post-commit; live controller validation remains.
- Browser Socket.IO is served separately at `/socket.io`; native `/ws` now dispatches protocol-isolated controller and regular-device heartbeat sessions by query identity.
- API gateway frontend proxy/static-host behavior has no final Rust deployment replacement.
- OpenAPI covers all mounted operations with summaries, behavioral descriptions, success responses, and stable response/error/media contracts; a generated-document invariant prevents undocumented operations from being mounted silently.
- External TestRail, Jira, GitLab, Qmetry, SMTP/provider, and deployment integrations are not fully validated.
- PostgreSQL migrations and repositories have not been tested against a live legacy-shaped database in this environment.

## Architecture and refactoring debt

- Auth, reports, and devices now use thin compatibility facades over responsibility-focused modules; no known mixed-responsibility module hotspot remains in those domains.
- `DeviceRepository` is a composed object-safe facade over feature capability traits, with matching PostgreSQL and test implementations.
- Relay hardware synchronization is lifecycle-owned by a bounded cancellable worker; durable retry policy remains open.
- Transient report queue/status state remains in memory by legacy design; durable execution report state is synchronized to PostgreSQL.
- The log API is mounted and tested, but live legacy-shaped PostgreSQL enum/table compatibility remains blocked by the unavailable test database.
- Analytics SQL is mounted and route-tested, but runtime compatibility with the legacy PostgreSQL schema remains blocked by the unavailable test database.
- Notification SQL is mounted and route-tested, but runtime compatibility with the legacy PostgreSQL enum/table schema remains blocked by the unavailable test database.
- Faulty-report SQL and filesystem behavior are mounted and route-tested; live legacy PostgreSQL and browser alert-emission validation remain blocked/deferred.

## Definition-of-done checkpoint

The local implementation is **complete**. The overall migration remains operationally incomplete until the following external verification deliverables are available:

- [-] All old functionality inventoried: route inventory exists; detailed contracts remain incomplete.
- [-] Gateway, auth, devices, reports, and workers are substantially ported; live infrastructure and exhaustive compatibility verification remain incomplete.
- [-] Public APIs, EdgeController, frontend, and E2E compatibility verified: partial focused evidence only.
- [x] One process, runtime modules, one listener, shared metrics/OpenAPI/health.
- [x] OpenAPI contracts, metrics/Grafana, logging sinks, and observability documentation are implemented and locally validated.
- [-] Central configuration/auth/database/security controls: core exists; hardening and live validation remain.
- [x] Cancellable workers and graceful shutdown: all currently required worker families are lifecycle-owned and bounded.
- [-] Unit/integration/API/edge-case coverage: meaningful suite exists but full route and infrastructure matrix remains.
- [x] Current format/check/strict-Clippy/full-test gates pass through consumer fixtures, scanner storage, Jira synchronization, observability, build preparation, the IPL lifecycle, system metrics, pending-user events, HTTP security policy, direct gateway/TUS routing, every mounted auth route, and native report artifacts/failure lifecycle; 148 local tests pass. Live PostgreSQL coverage still requires a harness and disposable database.
- [B] Dependency security audit unavailable (`cargo-audit` not installed) and live PostgreSQL validation unavailable.
- [x] Linux deployment artifacts and the required documentation set exist. Docker Compose rendering/build remains environment-blocked because Docker is unavailable locally.

## Update protocol

After each completed slice:

1. Verify implementation in source rather than relying on this document.
2. Run the narrowest behavior test, then format/check/strict Clippy and broader tests appropriate to risk.
3. Update the owning stage, API compatibility row, migration ledger, validation evidence, known gaps, and exact next action here.
4. Resume the earliest unfinished task whose dependencies are satisfied without asking for approval.

## Compatibility handoff

### EdgeController

- Focused source/tests cover public controller registration (including trailing slash), Gen5 mapping callback, Gen4/Gen5 flash confirmation, relay configuration confirmation, relay config/delete dispatch, reboot/power operations, and binary JSON heartbeat WebSocket identity/subprotocol behavior.
- Retained admin relay reads, soft deletes, conflict checks, duplicate cleanup/remap, and single-channel configuration are mounted and route-tested; live legacy-schema SQL and controller resynchronization remain unverified.
- Sibling EdgeController source confirms calls to `/api/v1/device/controller/`, `/mapping-gen5`, `/flash-confirm*`, `/relay/config/confirmation`, and `/ws`.
- Remaining risks: no live controller integration run and no captured full E2E result.

### Frontend

- Focused compatibility tests exist for auth shapes, device list/detail/topology/family/type, build list/upload wire contracts, many test catalog/execution/report routes, and CSV/file headers.
- Frontend test creation, table-summary, analytics, notification, and faulty-report HTTP contracts are focused-tested. Socket.IO rooms/events, WebCLI, SSH, and RTOS sockets are implemented and locally contract-tested but not validated against a live frontend.
- Native `/ws` is EdgeController heartbeat transport and is not a Socket.IO replacement.
- No full frontend-against-Rust run or racer E2E execution has been captured.

## Database handoff

Completed: SQLx pool/readiness/migration bootstrap, additive `0001`-`0004`, preflight audit script, disabled retention policies, conditional evidence-backed indexes, pending relay and Gen5 reboot state, static parameterized repository SQL, and guarded migration tests.

Remaining/blocked: production-clone preflight, live empty/legacy migration tests, repository integration/concurrency/rollback/constraint tests, duplicate/orphan catalog evidence, query plans, partitioning, data normalization, foreign-key/uniqueness rollout, retention approval/archive/restore evidence, and release-specific rollback artifacts. Rust migrations intentionally do not recreate legacy Sequelize domain tables.

## Real unfinished TODOs

- Run live PostgreSQL, Keycloak provider, TestRail/Qmetry, Confluence, EdgeController, frontend, and E2E validation.
- Run deployment/cutover validation with Docker, systemd, NFS, TFTP, and controller infrastructure.

## NEXT AGENT ACTION

Run the live legacy-shaped PostgreSQL migration/repository suite using a disposable `TEST_DATABASE_URL`, then execute the frontend/EdgeController smoke path against the unified listener when those environments are available.
