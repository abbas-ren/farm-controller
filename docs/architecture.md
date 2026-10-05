# Architecture

FarmController is a Cargo workspace that produces one Rust executable, one Tokio
runtime, and one Axum listener. Crate boundaries control compile-time ownership;
runtime modules remain configurable parts of the same process.

## Crates

| Crate | Responsibility | May depend on |
| --- | --- | --- |
| `farmcontroller-core` | CLI schema, validated configuration, safe shared errors, outbound HTTP client policy | external libraries only |
| `farmcontroller-integrations` | TestRail/GitLab, Qmetry, and Jira ports and HTTP adapters | `core` |
| `farmcontroller` (`service`) | API transport, auth, device workflows, persistence, events, reports, workers, shared state | `core`, `integrations` |
| `farmcontroller-app` | process composition, startup, listener, signals, graceful worker shutdown | `service` |

The allowed internal dependency graph is acyclic:

```text
farmcontroller-app
        |
        v
 farmcontroller-service ----> farmcontroller-integrations
        |                              |
        +------------------------------+
                       |
                       v
              farmcontroller-core
```

Lower crates never import `AppState`. The service crate re-exports core and
integration modules to preserve established Rust paths while callers migrate at
their own pace.

## Runtime flow

1. `app` parses CLI arguments and loads validated configuration from `core`.
2. Logging and metrics initialize before infrastructure connections.
3. `AppState::bootstrap` creates configured database and external adapters.
4. `service::build_app` mounts enabled REST, WebSocket, Socket.IO, operations,
   OpenAPI, and Swagger routes.
5. `WorkerManager` starts bounded tasks with a shared cancellation hierarchy.
6. The Axum server accepts traffic on the configured listener.
7. SIGINT or SIGTERM cancels the server and workers; worker handles are joined
   before process exit.

```text
Frontend / EdgeController
          |
 Axum + Socket.IO (/api, /ws, /socket.io, operations)
          |
       AppState
   +------+------+--------+---------+
   | auth | devices | reports | events/workers |
   +------+------+--------+---------+
          |
 PostgreSQL / Keycloak / TestRail / Qmetry / Jira / Confluence
```

## Service boundaries

- `api` owns router composition, transport middleware, operations endpoints, and
  the generated OpenAPI document.
- `auth` owns authorization policy and the Keycloak identity adapter used by the
  service. Stable response strings live in its local constants module.
- `devices` owns device/build/test/relay/upload contracts, handlers, repository
  traits, PostgreSQL implementations, validation, and compatibility behavior.
- `persistence` owns pool lifecycle, migrations, readiness, and retention policy.
- `events` owns native WebSocket protocol dispatch and browser Socket.IO delivery.
- `reports` owns report jobs, artifacts, graphs, and Confluence publication.
- `workers` owns cancellable background task startup and shutdown.
- `state` is the service composition container; feature crates do not depend on it.

HTTP handlers authorize and validate input, stores own transaction boundaries, and
events are published only after durable commits. Native `/ws` protocols dispatch
controller identity, frontend terminal, WebCLI, test-result, and heartbeat flows
without mixing them with browser Socket.IO at `/socket.io`.

## Persistence boundary

PostgreSQL remains the persistence engine. The public legacy schema is a wire and
data compatibility boundary. SQLx migrations in `migrations/` are additive;
destructive modernization requires the preflight and rollback process in
[database-migration.md](database-migration.md).

## Error and observability boundaries

`core::error::AppError` represents startup/infrastructure failures and emits safe
transport responses. Device, report, integration, and retention errors remain
typed in their owning modules. Internal context is logged with `tracing`; secrets
and sensitive payloads are excluded from logs and client errors.

Metrics use bounded labels. Request, worker, database, pool, memory, and uptime
signals are exposed on the same listener. See [observability.md](observability.md).

Open design history and intentional debt are recorded in
[architecture-decisions.md](architecture-decisions.md) and
[rearchitecture.md](rearchitecture.md).