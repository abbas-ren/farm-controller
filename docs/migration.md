# Migration And Cutover

The legacy Node/Python implementation remains the rollback reference while Rust parity is verified.

1. Back up PostgreSQL and verify restore procedures.
2. Run `migrations/preflight/legacy_schema_audit.sql` against a production-shaped clone.
3. Run ignored migration tests against a disposable legacy-shaped PostgreSQL database.
4. Start Rust with health/metrics only, then enable API/auth/device/reports/workers/events in staging.
5. Exercise frontend login, inventory, uploads, tests, reports, Socket.IO, native controller/device/test sockets, and terminal sessions.
6. Exercise EdgeController registration, mapping, relay, flash, RTOS, reboot, and heartbeat paths.
7. Compare status codes, response/file shapes, database side effects, events, and external calls with [api-compatibility.md](api-compatibility.md).
8. Stop legacy background workers before enabling Rust workers to avoid duplicate dispatch.
9. Shift traffic, monitor readiness/errors/worker metrics, and retain rollback routing.

Rollback stops Rust workers first, restores the legacy services, and returns traffic to the old gateway. Current Rust migrations are additive; do not remove their schema during an incident. Restore PostgreSQL only for confirmed data corruption, using the rehearsed backup procedure.

Open parity and environment-blocked checks are maintained in [rust-migration-progress.md](rust-migration-progress.md).