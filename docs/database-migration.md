# Database Migration Plan

## Safety boundary

The Sequelize models are evidence, not the production schema authority. The legacy service calls `sequelize.sync({ alter: true })`, has no versioned migration history, and may have drifted between installations. Before changing a domain table, capture a schema-only dump and run `migrations/preflight/legacy_schema_audit.sql` against a production clone. No deduplication, foreign key, type conversion, rename, or drop is permitted until that evidence is reviewed.

Migration `0001_persistence_control_plane.sql` is safe to deploy independently. It creates only the `farmcontroller` namespace, an evidence ledger, and disabled retention policies. It does not mutate legacy domain data.

## Decisions and evidence

| Existing object | Decision | Proposed representation | Evidence and prerequisite |
|---|---|---|---|
| Active Sequelize tables | KEEP initially | Same public names and columns behind typed SQLx repositories | All 21 are initialized by `dbInit.ts`; preserve API and stored-data compatibility until each domain is ported |
| `heartbeats` | KEEP | Per-device heartbeat history | Although absent from `dbInit.ts`, `DeviceService.ts` dynamically initializes `HeartbeatModel(sequelize)` and actively queries it. This table is distinct from controller-level heartbeats |
| `device_controller_heartbeats` | KEEP, then partition | Time-range partitioned heartbeat history | Active queue reads/writes; critical unbounded growth. Benchmark partitioning on a production-sized clone first |
| `device_controller_metrics` | KEEP, normalize later | Numeric measurements plus controller/time index | Active aggregate history; current numeric values are strings. Conversion requires invalid-value inventory and dual-read compatibility |
| `devices.deviceId` | KEEP contract key | Unique text key initially | Frontend, controller, queues, and tests use this identifier. Do not convert to UUID without proving the value domain |
| `device_interfaces` | KEEP, constrain | Unique `(deviceId, interfaceId)` | This is the interface identity in service queries. Preflight must show no duplicates before adding a unique constraint |
| `relays` and `relay_channels` | KEEP | Relay identity plus unique `(relayId, channelNumber)` | Active topology. Audit duplicate relay serials and soft-delete semantics before strengthening constraints |
| `alerts.status` and `alerts.isRead` | MERGE candidate | One read state plus `readAt` | Redundant state exists. Measure conflicting rows, backfill deterministically, then dual-write before column removal |
| JSON/JSONB payloads | KEEP initially | Typed Rust DTO at repository boundary | Heartbeat payloads, selections, mappings, and artifacts have active compatibility value. Normalize only fields with proven relational query needs |
| Queue tables | KEEP | Durable queue repositories | Required for restart safety. Retention may delete only terminal jobs in bounded batches |
| Execution/report tables | KEEP | Explicit one-to-one report relationship where data proves it | `execution_reports.testExecutionId` is intended as one report per execution, but only an index exists; preflight duplicate check gates uniqueness |

## Integrity and performance

Apply improvements in separate migrations after preflight, using `NOT VALID` foreign keys followed by `VALIDATE CONSTRAINT` where PostgreSQL permits it. Candidate changes are:

- Unique index on `device_interfaces ("deviceId", "interfaceId")` after deterministic deduplication.
- Unique partial index on active relay serial numbers after defining whether a serial may be reused following soft deletion.
- Unique index on `execution_reports ("testExecutionId")` only if the product contract confirms one report per execution.
- Foreign keys for device-to-controller and loose queue/report references only after orphan classification and repair.
- Polling indexes with terminal/active partial predicates, justified with `EXPLAIN (ANALYZE, BUFFERS)` from production-shaped data.
- Controller/time composite indexes for heartbeat and metric range reads; consider time partitioning before adding overlapping indexes.

Do not add indexes solely from model declarations. Capture `pg_stat_user_indexes`, query plans, table sizes, and the actual production index catalog first to avoid duplicate write cost.

## Deduplication

Deduplication is a data migration, not a delete script. For each natural key:

1. Export duplicate groups and all inbound references to an immutable audit artifact.
2. Select a survivor with a domain-specific deterministic rule, generally active over soft-deleted and newest complete record over incomplete record.
3. Repoint children in one transaction and record loser-to-survivor identifiers in a reconciliation table.
4. Quarantine ambiguous groups for operator review; never choose by row ID alone.
5. Add the unique constraint only after a second zero-duplicate check.

## Retention

Policies are disabled by default. Enabling one requires approved retention duration, backup/restore verification, a measured initial backlog, and an alert threshold. Cleanup uses bounded `ctid` batches with `FOR UPDATE SKIP LOCKED`; queue cleanup is limited to `completed` and `cancelled`, and read alerts only. `device_state_change` requires archival and is rejected by the deletion executor until an archive destination exists.

Run cleanup from the workers module at a fixed low-concurrency cadence. Export deleted-row counts, duration, failures, and pool health through Prometheus. Pause automatically on repeated errors or replica lag. Vacuum behavior and table bloat must be monitored during backlog removal.

## Rollout and rollback

1. Restore the latest production backup into an isolated PostgreSQL 17 database and run preflight.
2. Apply `0001`, rerun it to prove idempotent seed behavior, and run the Rust database integration tests.
3. Deploy SQLx in read-only shadow mode for each repository and compare results with the legacy service.
4. Move writes one domain at a time, using dual-read or dual-write only where an online shape change requires it.
5. Enable retention one table at a time with a small batch and explicit operator approval.
6. Remove legacy models or columns only after compatibility tests pass and the rollback window expires.

Additive migrations roll back by disabling new code and leaving the new objects in place. Destructive migrations require restore or an explicit reverse data migration; therefore they are scheduled only after backup restore tests and a release-specific rollback runbook.

## Test matrix

- Migration tests: empty database, legacy-shaped database, repeat application, existing-row preservation, and upgrade from every supported release snapshot.
- Repository tests: CRUD, transactions, uniqueness conflicts, foreign key failures, enum/JSON decoding, pagination, and concurrent updates.
- Retention tests: disabled-by-default, allowlisted targets, age boundary, batch cap, terminal-state protection, archival requirement, concurrent workers, and retry metrics.
- Performance tests: representative row counts, query plans, pool saturation, migration lock duration, and cleanup impact.
- Recovery tests: failed migration, interrupted backfill, point-in-time restore, and old-binary compatibility during the rollout window.

The live PostgreSQL harness remains to be implemented. It must use
`TEST_DATABASE_URL`, refuse database names without an `_test` suffix, and stay
outside the default unit-test path so local checks cannot touch a developer or
production database accidentally.