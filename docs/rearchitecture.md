# Rearchitecture Report

Date: 2026-10-05

## 1. Executive summary

FarmController was converted from one Cargo package into a four-crate workspace
without changing its deployed process topology or external protocols. Process
composition, shared operational policy, external adapters, and stateful service
behavior now have explicit compile-time ownership and an acyclic dependency graph.

The refactor preserved the `farmcontroller` service library name and re-exports
the moved core and integration modules. Existing Rust paths, HTTP routes, payloads,
database behavior, workers, event protocols, and CLI behavior therefore remain
compatible.

## 2. Before and after

Before:

```text
farmcontroller package
└── src/
    ├── main.rs
    ├── configuration and infrastructure policy
    ├── external integrations
    └── all stateful service features
```

After:

```text
workspace
├── Cargo.toml
├── crates/
│   ├── app/
│   ├── core/
│   ├── integrations/
│   └── service/
├── config/
├── migrations/
└── docs/
```

## 3. Crate responsibilities

### `farmcontroller-app`

Owns CLI invocation, startup ordering, listener binding, signal handling,
cancellation, and worker shutdown. It contains no feature implementation.

### `farmcontroller-core`

Owns configuration models and precedence, CLI types, startup/infrastructure
errors, safe client-facing error envelopes, and the common outbound HTTP client
policy. It has no internal workspace dependency and cannot access `AppState`.

### `farmcontroller-integrations`

Owns TestRail/GitLab, Qmetry, and Jira traits, models, HTTP adapters, and their
mock-backed tests. It depends only on `core` inside the workspace.

### `farmcontroller` (`service`)

Owns router composition, authentication, devices, PostgreSQL behavior, events,
reports, background workers, observability, and `AppState`. It re-exports core and
integration modules as a compatibility facade.

## 4. Dependency direction

```text
app -> service -> integrations -> core
               -> core
```

There are no workspace dependency cycles. `AppState` remains in the stateful
service layer and is not imported by lower crates. A separate domain or
persistence crate was intentionally not introduced: current device repository,
event, report, and worker contracts share service-owned types and state. Moving
them now would create artificial interfaces or cycles rather than useful
independence.

## 5. Important module boundaries

- Configuration, CLI, shared startup errors, and HTTP policy moved to `core`.
- Jira, TestRail/GitLab, and Qmetry moved with their tests to `integrations`.
- Binary lifecycle moved to `app`.
- API, persistence, device workflows, reports, events, and workers remain together
  in `service` because they coordinate through `AppState` and durable transitions.
- Stable authentication response messages now live in an auth-local constants
  module instead of the mixed implementation file.

## 6. Major decisions

- Preserve one deployed process and listener; crates are ownership boundaries, not
  microservices.
- Preserve the service crate name and public module paths through re-exports.
- Centralize versions and features in root workspace dependencies.
- Keep root configuration, migrations, and documentation outside crate directories
  because they are deployment-level assets.
- Keep SQLx migrations additive and point the relocated service crate at the root
  migration directory at compile time.
- Avoid speculative traits and empty domain crates. Boundaries were extracted only
  where imports were already one-way.

## 7. Functionality preserved

Completed and covered by the existing workspace tests:

- REST, OpenAPI, Swagger, health, readiness, and metrics routing
- Keycloak authentication and authorization contracts
- device, controller, build, relay, upload, and test workflows
- PostgreSQL repository and migration compilation
- worker startup, cancellation, and shutdown behavior
- native WebSocket and Socket.IO behavior
- report generation and Confluence adapter behavior
- TestRail/GitLab, Qmetry, and Jira adapters
- configuration precedence and CLI parsing

## 8. API changes

No external API, protocol, route, request, response, authentication, CLI flag, or
binary-name change was introduced. Internal source paths changed to the workspace
layout. The service crate's compatibility re-exports preserve existing Rust module
paths for consumers.

## 9. Database and persistence impact

No schema or SQL behavior changed. Root migrations remain in `migrations/` and are
embedded by the service crate through the adjusted relative path. Existing
additive migration and preflight policies remain in force.

Live PostgreSQL tests remain environment-gated by `TEST_DATABASE_URL`; they were
not run because no disposable PostgreSQL URL was supplied.

## 10. Observability and tracing

Existing structured logging, request spans, worker/database metrics, process
metrics, and safe logging constraints were preserved. Startup and shutdown remain
owned by the app crate, while observability implementation remains with service
state. No labels or event payloads changed.

## 11. Testing and validation

Completed during this refactor:

- `cargo fmt --all -- --check`: passed
- `cargo check --workspace --all-targets --all-features`: passed
- `cargo test --workspace --quiet`: passed, 148 tests, 0 failures
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed
- focused `cargo check -p farmcontroller`: passed after auth constants extraction

## 12. Known limitations

- `devices/mod.rs`, `auth/mod.rs`, `reports/mod.rs`, and
  `devices/test_execution_handlers.rs` remain oversized.
- `DeviceRepository` remains a broad compatibility trait over the legacy schema.
- Report job snapshots remain in memory and are lost on restart.
- Live PostgreSQL, Keycloak, controller, frontend, Confluence, TestRail, Qmetry,
  Jira, Docker, and deployment smoke tests require external infrastructure.
- The legacy public database schema remains intentionally unchanged.

## 13. Remaining technical debt

The next low-risk internal slices are:

1. Split `DeviceRepository` into capability traits after adding contract tests for
   each handler group; keep a composed trait for `AppState` during transition.
2. Separate auth transport handlers from the Keycloak adapter and auth DTOs.
3. Separate report HTTP transport, job orchestration, artifact generation, and
   Confluence publication.
4. Persist report job state and make queue recovery explicit.
5. Move request metrics middleware behind a state-independent metrics handle,
   allowing observability and database pool ownership to move below `service`.

## 14. Recommended future improvements

- Add an architecture test or dependency-policy CI check that rejects reverse
  workspace dependencies.
- Add live PostgreSQL repository tests to CI using an isolated disposable database.
- Add consumer contract fixtures for active frontend Socket.IO listeners and
  EdgeController protocols.
- Benchmark worker queues, report generation, and large upload paths with
  production-shaped data before changing limits.
- Revisit a dedicated persistence crate only after service-owned repository traits
  no longer depend on transport/event types.

## 15. Status summary

- **Completed:** workspace conversion, app/core/integration extraction, dependency
  centralization, compatibility facade, moved tests, documentation rewrite.
- **Partially completed:** fine-grained modularization inside the stateful service
  crate; major hotspots are documented and behavior remains tested.
- **Not applicable:** separate deployable services for each crate.
- **Blocked/unverified:** checks requiring live databases, identity providers,
  device/controller consumers, external SaaS systems, Docker, or production
  infrastructure.