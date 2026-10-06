# Architecture Decisions

This document records decisions already reflected in the current Rust migration. It is descriptive, not proof of completion. `prompt.md` remains authoritative, and implementation status is tracked in `docs/rust-migration-progress.md` and `docs/api-compatibility.md`.

Last reconciled against source: 2026-10-06

## Status vocabulary

- **Accepted**: reflected in current source and intended to remain.
- **Provisional**: implemented for parity but requires validation or later replacement.
- **Open**: required by the migration plan but not yet decided or implemented.

## Unified Rust service architecture

**Decision: Accepted.** FarmController is one Cargo workspace with four responsibility-driven packages, one `farmcontroller` binary, one Tokio runtime, one `AppState`, and one Axum listener. The listener conditionally mounts auth, device, reports, events, metrics, Swagger, health, and readiness routes. Internal Rust domains call providers/repositories directly rather than recreating legacy service-to-service HTTP hops.

Evidence: `Cargo.toml`, `crates/app/src/main.rs`, `crates/service/src/api/mod.rs`, `crates/service/src/state.rs`, `crates/service/src/lib.rs`.

Consequences:

- The old Node/Python services remain migration references and rollback assets, not desired runtime dependencies.
- Module boundaries are logical, not separate processes or servers.
- Module activation errors are startup configuration errors.

## Module and package organization

**Decision: Accepted.** `farmcontroller-core` owns configuration, CLI types, shared startup errors, and outbound HTTP policy. `farmcontroller-integrations` owns TestRail/GitLab, Qmetry, and Jira adapters. `farmcontroller` owns the stateful API, auth, devices, reports, events, workers, persistence, and observability. `farmcontroller-app` owns process composition.

Within the service crate, `auth`, `reports`, and `devices` use thin compatibility facades over responsibility-focused modules. Auth separates transport, session policy, provider contracts, DTOs, errors, constants, and Keycloak operations. Reports separates HTTP transport, orchestration, data access, artifact writing, graphing, and Confluence publication. Devices separates DTO families, handler families, validation, stores, repository capabilities, PostgreSQL implementations, and focused tests. The facades preserve established Rust paths without retaining duplicate implementations.

## Dependency injection and external boundaries

**Decision: Accepted.** `AppState` owns optional trait-backed providers for identity, device persistence, TestRail, Qmetry, reports, database, metrics, and configuration. Traits are used at genuine external boundaries rather than for every helper.

Evidence: `crates/service/src/state.rs`, `crates/service/src/auth/provider.rs`, `crates/service/src/devices/repository.rs`, `crates/service/src/devices/postgres/`, `crates/integrations/src/test_catalog/mod.rs`, `crates/integrations/src/qmetry_catalog/mod.rs`, `crates/service/src/reports/data_source.rs`.

**Decision: Accepted.** `DeviceRepository` remains the object-safe dependency stored by `AppState`, but it composes feature-sized capability traits for execution, build, inventory, user-device, controller, relay, analytics, alerts, and related persistence. PostgreSQL and test implementations satisfy those capabilities directly, avoiding a forwarding implementation while preserving `Arc<dyn DeviceRepository>` compatibility.

## Database and schema decisions

**Decision: Accepted.** PostgreSQL remains the persistence engine. SQLx uses the existing public legacy table/column names initially; the migration does not recreate or casually redesign the Sequelize domain schema. Static parameterized SQL and explicit transactions are preferred.

Five additive migrations exist:

1. `0001_persistence_control_plane.sql`: `farmcontroller` schema, evidence inventory, disabled retention policies.
2. `0002_schema_evidence_and_indexes.sql`: corrected heartbeat evidence and conditional indexes.
3. `0003_pending_relay_configuration.sql`: durable pending relay configuration state.
4. `0004_gen5_reboot_attempts.sql`: durable Gen5 reboot-attempt and grace-period state.
5. `0005_legacy_schema_compatibility.sql`: conditional controller soft-delete support and corrected alerts retention metadata.

`migrations/preflight/legacy_schema_audit.sql` and `docs/database-migration.md` define the safety boundary. Domain DDL is intentionally assumed from an upgraded legacy deployment until production-catalog evidence is captured. No destructive schema modernization is approved without preflight, backup/restore, duplicate/orphan analysis, query plans, and rollback artifacts.

**Blocked:** live migration/repository tests require `TEST_DATABASE_URL` pointing to a disposable database whose name ends in `_test`. No PostgreSQL/container tooling or test URL was available at handoff.

## API compatibility decisions

**Decision: Accepted.** Existing legacy paths, methods, payload names, status codes, pagination, file headers, and selected historical quirks are compatibility contracts unless current implementation/tests/consumers justify a change. Static Axum paths are used to avoid Express dynamic-route shadowing. Observed legacy `500` not-found behavior is retained in a few testcase/build operations and documented rather than silently normalized.

**Decision: Accepted.** EdgeController bootstrap/callback routes remain public where deployed controllers do not send bearer tokens. Device registration accepts both collection forms needed by clients.

**Decision: Accepted.** TUS uses durable sidecar metadata and payload files, append locking, offset validation, relative public locations, retry-safe rollback, explicit limits, and traversal-safe ZIP staging/extraction.

**Decision: Accepted.** Browser Socket.IO, native controller/device/test-result WebSockets, authenticated frontend SSH/RTOS, and WebCLI protocols share the listener with protocol isolation. Inventoried HTTP route groups and event producers are implemented subject to live consumer validation.

## Authentication and authorization

**Decision: Accepted.** Keycloak remains the external identity provider during parity migration. Rust supports password/client credentials, introspection, refresh, realm/client roles, user administration, action email, and the observed refresh headers/cookie. Authorization is centralized through `auth::authorize_request`.

**Decision: Accepted.** All Reqwest clients are built through `external_http`, which caps connect time, preserves each integration's request timeout, disables redirects, and rejects configured non-HTTP(S) or credential-bearing URLs at startup. The client policy performs no implicit retries; the two retained device command paths share one bounded three-attempt policy.

**Compatibility note:** legacy authorization is inconsistent. Public callbacks and historical status/error behavior are preserved where required; hardening must be a documented compatibility change.

**Provisional:** `IdentityProvider` has `501` default methods for test/substitute providers, while `KeycloakIdentityProvider` overrides the production operations. These defaults are not evidence that the Keycloak implementation is a stub.

## Worker architecture and lifecycle

**Decision: Accepted.** Owned Tokio tasks must be cancellable through `CancellationToken`, joined during shutdown, bounded, and instrumented. `WorkerManager` owns retention, reports, bounded relay dispatch, relay timeout, controller/device heartbeat timeout, test preparation/action dispatch/watchdog, Gen5 mapping/reboot, and fallback-flash handles; SIGINT/SIGTERM cancel the server and workers.

**Decision: Accepted.** Artifact scanning, preparation, official workflow dispatch, IPL distribution/backfill/deletion, frontend SSH/RTOS, and interactive WebCLI run in-process under owned worker or connection lifecycles.

**Decision: Accepted.** Relay hardware synchronization uses an AppState-owned bounded queue and cancellable worker. Handler queue saturation/unavailability is explicit; durable retries remain future work.

## WebSocket and event architecture

**Decision: Accepted for native clients.** `/ws` selects protocol-isolated controller (`deviceControllerId`), testcase-result (`testId` plus `deviceId`), or regular-device (`deviceId`) sessions in that precedence order. Controller sessions preserve `web-cli-protocol`, binary/text JSON, identity validation, metrics persistence, and timeout/recovery. Test sessions persist results and terminalize complete executions. Device sessions reconcile heartbeats, interfaces, and post-boot transitions.

**Decision: Accepted.** Browser Socket.IO is a separate protocol on `/socket.io` through the shared Axum listener. The AppState-owned event hub installs a lifecycle-owned Socketioxide layer, supports polling/WebSocket upgrade, handshake `userId`, and the legacy build/test/dashboard/device/controller rooms. The native EdgeController `/ws` remains protocol-isolated.

**Decision: Accepted subject to live validation.** Upload, report, test creation/cancellation/watchdog/completion, heartbeat, controller, relay, power/flashing, registration/faulty-report, and build flag/delete producers use the event hub after durable transitions. WebCLI, SSH, and RTOS frontend terminal behavior is implemented separately from Socket.IO events.

## Reports

**Decision: Accepted with temporary state.** Reports run in-process through a bounded Tokio queue and cancellable worker. HTTP routes `/report`, `/report/{job_id}`, and `/upload-confluence` are mounted when the Reports module is enabled. Execution-report compatibility routes bridge to the same service.

**Temporary implementation:** native job snapshots are held in memory and lost on restart. Legacy `execution_reports` rows are created before enqueue and terminal status is synchronized before user/test-room event publication. Confluence behavior has mock/path tests but no live validation in this environment.

## Observability and logging

**Decision: Accepted.** `tracing` provides configurable text/JSON console output and optional nonblocking file output. Request IDs, route-template HTTP counters/durations/active gauge, database operation metrics, worker metrics, and process uptime are exposed through Prometheus on the shared listener.

**Decision: Accepted.** Selectable stdout/stderr/off streams, JSON/text formatting, nonblocking file output, optional UDP network logging, bounded domain/external/pool/runtime metrics, Prometheus/Grafana documentation, and dashboard JSON are implemented. Request IDs remain HTTP-span context rather than explicit repository parameters.

## Metrics

**Decision: Accepted.** Metrics use bounded labels; raw users, devices, tokens, bodies, and arbitrary URLs must not become labels. `/metrics` is mounted on the same listener, not a second server.

## Swagger and OpenAPI

**Decision: Accepted.** Utoipa generates `/openapi.json`, and vendored Swagger UI is served at `/swagger-ui/` on the same listener. Bearer authentication is documented as a shared security scheme.

**Decision: Accepted and enforced.** Every mounted HTTP/WebSocket operation has a generated summary, behavioral description, and success response. Stable response envelopes, bounded errors, multipart/binary/CSV media, representative examples, compatibility quirks, and TUS protocol headers are represented and regression-tested through a global generated-document invariant.

## CLI and configuration feature gates

**Decision: Accepted.** Runtime modules are `api`, `auth`, `device`, `reports`, `workers`, `events`, `metrics`, `swagger`, and `health`. Repeated/comma-separated `--enable` and `--disable` flags are supported. Precedence is defaults/config deserialization, environment, then CLI overrides; environment nesting uses `FARMCONTROLLER__...`.

Configuration is centralized in `crates/core/src/config/mod.rs` for server, modules, Keycloak, database, device/artifacts/uploads, workers, reports/Confluence, TestRail, Qmetry, test logs, and logging. Startup validates module dependencies, secrets, database requirements, sizes, and timeouts.

## Deployment decisions

**Decision: Accepted and packaged.** Linux is the deployment target and Rust 1.98.1/edition 2024 is the pinned, currently supported toolchain. The release is one Rust process without Node.js/Python runtime services.

Systemd, Rust-only Docker/Compose, production configuration, Grafana, cutover, and rollback artifacts exist. Docker/systemd deployment smoke tests and the static frontend hosting decision remain open because this environment has no Docker daemon or production infrastructure.

## Security decisions

- Parameterized static SQL is required; dynamic sort/filter inputs use allowlists.
- Uploads have explicit limits, structured metadata, safe staging, archive traversal/symlink/expansion controls, and retry-safe TUS offsets.
- Configurable external pagination links must remain same-origin.
- Database/filesystem paths derived from input or stored rows require segment/root containment checks.
- Secrets must come from protected config/environment and must not be logged.
- `cargo-audit` is required when available; it was not installed at handoff.

## Current deviations and temporary replacements

- Native WebSocket, Socket.IO, authenticated WebCLI, and inventory-authorized SSH/RTOS terminal protocols share one listener while retaining protocol isolation.
- Report jobs are in memory.
- Execution creation persists `PREPARE_ARTIFACTS` state atomically; owned preparation/action-queue workers and frontend event emission process the durable state.
- Build/device callbacks, upload events, NFS/TFTP preparation, official dispatch, scanner/upload IPL storage and SFTP distribution, controller backfill, and build-delete IPL cleanup exist. Live controller validation remains blocked.
- Legacy domain tables are assumed rather than created by Rust migrations.
- Log create/list/search is routed through the device repository with static parameterized SQL; live legacy PostgreSQL enum/table validation remains blocked by unavailable infrastructure.
- Retained relay CRUD/fresh/remap/conflict routes use a dedicated static-SQL store and the existing controller-action boundary; live legacy PostgreSQL and controller resynchronization validation remain blocked by unavailable infrastructure.
- Analytics uses authenticated Axum handlers and a dedicated static-SQL store. User scoping and historical unscoped exceptions are preserved per route; UTC is the explicit date boundary. Live legacy PostgreSQL validation remains blocked by unavailable infrastructure.
- Notifications use authenticated Axum handlers and a dedicated static-SQL store. Read mutations intentionally preserve the legacy distinction between canonical not-found behavior and retained admin zero-row success; mark-all remains best effort. Browser alert emission remains an event-transport gap.
- Faulty reports use bounded streaming multipart handling, configurable storage, root-contained downloads, static SQL, Keycloak user lookup, persisted alerts, and post-commit browser event publication.
- Server-side user dispatch publishes named envelopes through the AppState-owned bounded event hub and installed Socket.IO adapter with room/reconnect-compatible transport behavior.

## Decision review trigger

Revisit these decisions only with source/test/consumer evidence. Any change to a legacy public contract, database shape, authentication requirement, event protocol, or deployment topology must update this file, `docs/api-compatibility.md`, tests, OpenAPI, and rollback notes.

## NEXT AGENT ACTION

Reconcile active frontend Socket.IO listeners and room joins against Rust producers, adding focused fixtures for uncovered payload and room contracts without changing established wire behavior.
