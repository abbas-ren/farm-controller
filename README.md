# FarmController

FarmController is the unified Rust control plane for farm devices, controllers,
build artifacts, test execution, reporting, and browser/device event streams. It
runs as one Tokio process with one Axum listener while keeping compilation and
ownership boundaries explicit through a Cargo workspace.

## Workspace architecture

```text
crates/
├── app/           process composition, signals, listener, worker lifecycle
├── core/          configuration, CLI schema, shared errors, HTTP client policy
├── integrations/  Jira, TestRail/GitLab, and Qmetry ports and HTTP adapters
└── service/       API, auth, devices, persistence, events, reports, and workers
```

The dependency direction is:

```text
app -> service -> integrations -> core
               -> core
```

`service` is the compatibility facade and re-exports the public core and
integration modules. This preserves existing Rust paths while preventing lower
layers from depending on application state. Crates are architectural boundaries,
not separate deployed services.

See [docs/architecture.md](docs/architecture.md),
[docs/architecture-decisions.md](docs/architecture-decisions.md), and
[docs/rearchitecture.md](docs/rearchitecture.md) for the runtime model and design
decisions.

## Prerequisites

- Rust 1.98.1 with Cargo
- A C toolchain and CMake for native dependencies
- PostgreSQL for database-backed modules and repository tests
- Keycloak only when the authentication module is enabled
- NFS/TFTP/SFTP and external systems only for workflows that use them

## Configuration

Configuration precedence is built-in defaults, TOML file, environment, then CLI.
Start with [config/default.toml](config/default.toml); production deployments can
derive a protected file from
[config/production.toml.example](config/production.toml.example).

Nested environment keys use `FARMCONTROLLER__`, for example:

```bash
export FARMCONTROLLER__DATABASE__URL='postgresql://localhost/farmcontroller'
export FARMCONTROLLER__AUTH__CLIENT_SECRET='...'
export FARMCONTROLLER__LOGGING__LEVEL='info'
```

Secrets must come from protected files or environment injection and must not be
committed. The main runtime controls are `--config`, `--bind`, `--enable`, and
`--disable`; module lists may be repeated or comma-separated. Full settings and
validation rules are in [docs/configuration.md](docs/configuration.md).

## Build and run

```bash
cargo build --workspace
cargo run -p farmcontroller-app --bin farmcontroller -- \
  --config config/default.toml \
  --bind 127.0.0.1:3000
```

Enable infrastructure-backed modules explicitly when their configuration is
available:

```bash
cargo run -p farmcontroller-app --bin farmcontroller -- \
  --config config/default.toml \
  --enable auth,device,reports,events,workers
```

Build the production binary with:

```bash
cargo build --locked --release -p farmcontroller-app
./target/release/farmcontroller --config config/production.toml
```

The Rust-only Compose definition is available as
[docker-compose.rust.yml](docker-compose.rust.yml). Deployment and rollback
requirements are documented in [docs/deployment.md](docs/deployment.md) and
[docs/migration.md](docs/migration.md).

## Database

PostgreSQL is required when any enabled module needs persistence. SQLx migrations
are additive and live in [migrations/](migrations/). The existing public legacy
schema remains a compatibility boundary; do not apply destructive changes without
the preflight process in
[docs/database-migration.md](docs/database-migration.md).

Live persistence tests require `TEST_DATABASE_URL` to name a disposable database
ending in `_test`. Never point that variable at production or shared data.

## API and protocols

One listener serves:

- `/api/*` for REST APIs
- `/ws` for native controller, terminal, test-result, and heartbeat protocols
- `/socket.io` for browser Socket.IO clients
- `/health` and `/ready` for health checks
- `/metrics` for Prometheus exposition
- `/openapi.json` and `/swagger-ui/` for generated API documentation

Route availability depends on enabled modules. Authentication uses Keycloak-backed
bearer tokens and compatibility cookies where required by existing clients. See
[docs/api.md](docs/api.md) and
[docs/api-compatibility.md](docs/api-compatibility.md) for endpoint and wire
contract details.

## Development and testing

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Mock-backed tests cover Keycloak, TestRail, Qmetry, Jira, and Confluence without
live credentials. Database tests are environment-gated. Keep events after durable
commits, SQL parameterized, workers bounded and cancellable, and filesystem paths
root-contained. See [docs/development.md](docs/development.md).

## Observability

The process emits structured `tracing` logs in text or JSON, supports nonblocking
file output and optional UDP forwarding, and exposes bounded-cardinality HTTP,
worker, database, pool, memory, and uptime metrics. Tokens, credentials, and
sensitive payloads must never be logged. Operational details are in
[docs/observability.md](docs/observability.md) and diagnostics in
[docs/troubleshooting.md](docs/troubleshooting.md).

## Documentation

The documentation index is [docs/README.md](docs/README.md). Migration history,
compatibility evidence, deployment guidance, current limitations, and future work
remain under [docs/](docs/); they are retained as operational history rather than
as alternate setup instructions.