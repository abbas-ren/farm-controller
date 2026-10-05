# Architecture

FarmController runs as one Rust executable, one Tokio runtime, and one Axum listener. Runtime modules are logical boundaries selected through configuration or CLI flags; they are not separate services.

```text
Frontend / EdgeController
          |
   Axum + Socket.IO (/api, /ws, /socket.io, /metrics, /health)
          |
       AppState
   +------+------+--------+---------+
   | auth | devices | reports | events/workers |
   +------+------+--------+---------+
          | PostgreSQL / Keycloak / TestRail / Qmetry / Confluence
```

`AppState` owns configuration, providers, PostgreSQL, reports, metrics, the event hub, and worker-facing services. HTTP handlers authorize and validate input, domain stores own transactions, and events are published only after commit. `WorkerManager` owns cancellable task handles and joins them during shutdown.

Native `/ws` protocols are dispatched in this order: controller identity, frontend SSH/RTOS, WebCLI, testcase result identity, then device heartbeat. Browser Socket.IO remains isolated at `/socket.io`.

The legacy public PostgreSQL schema remains the compatibility boundary. Rust migrations are additive and live under `migrations/`; destructive modernization requires the preflight process in [database-migration.md](database-migration.md).

Open design history and temporary debt are recorded in [architecture-decisions.md](architecture-decisions.md).