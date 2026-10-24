# Raw SQL Inventory (sqlx → SeaORM)

> Generated: 2026-10-24  
> Scope: `crates/service`, `crates/integrations`, `crates/core` (excluding `migrations/`, `target/`)  
> **Status: MIGRATED (2026-10-24)**  
> - Active `sqlx::query*` call sites: **0**  
> - `sqlx` remains only for `PgPool` bootstrap + `sqlx::migrate!` in `persistence/mod.rs`  
> - New entities: `gen5_reboot_attempt`, `device_uart_mapping`, `retention_policy`  
> - Helper: `persistence/orm_util.rs` (`query_all` / `query_one` / `execute` via SeaORM `Statement`)

## Migration outcome

| Layer | Result |
|-------|--------|
| `sqlx::query` / `query_as` / `query_scalar` | **Removed everywhere** |
| SeaORM Entity / ActiveModel / `update_many` | Primary path for CRUD, filters, joins |
| SeaORM `Statement` / `orm_util` | Retained only where SQL is not expressible cleanly as Entity builders: `FOR UPDATE SKIP LOCKED` claims, `LATERAL` / `jsonb_each` matching, analytics CTEs, dynamic retention DELETEs, nested `jsonb` export shapes |

Historical per-file counts below are the **pre-migration** inventory (kept for audit). All listed sites were converted; treat Status column as superseded by this outcome section.

## 1. Summary by file (pre-migration counts)

| Count | File |
|------:|------|
| 23 | `crates/service/src/devices/legacy_relay_store.rs` |
| 16 | `crates/service/src/devices/postgres/controllers.rs` |
| 16 | `crates/service/src/devices/relay_store.rs` |
| 14 | `crates/service/src/test_results.rs` |
| 13 | `crates/service/src/devices/callback_store.rs` |
| 13 | `crates/service/src/devices/device_registration_store.rs` |
| 13 | `crates/service/src/devices/registration_store.rs` |
| 12 | `crates/service/src/workers/gen5_reboot_watchdog.rs` |
| 11 | `crates/service/src/devices/faulty_report_store.rs` |
| 11 | `crates/service/src/devices/postgres/execution_reports.rs` |
| 10 | `crates/service/src/devices/postgres/execution_queries.rs` |
| 10 | `crates/service/src/devices/postgres/inventory.rs` |
| 10 | `crates/service/src/workers/gen5_mapping.rs` |
| 9 | `crates/service/src/workers/artifact_scanner.rs` |
| 8 | `crates/service/src/devices/relay_configuration_store/apply.rs` |
| 7 | `crates/service/src/devices/alert_store.rs` |
| 7 | `crates/service/src/devices/analytics_store/executions.rs` |
| 7 | `crates/service/src/devices/build_store/uploads.rs` |
| 6 | `crates/service/src/devices/build_store/mutations.rs` |
| 6 | `crates/service/src/devices/controller_store.rs` |
| 6 | `crates/service/src/devices/heartbeat_store/controller.rs` |
| 6 | `crates/service/src/devices/relay_configuration_store/confirmation.rs` |
| 6 | `crates/service/src/workers/fallback_flash.rs` |
| 5 | `crates/service/src/devices/build_store/queries.rs` |
| 5 | `crates/service/src/devices/flashing_store.rs` |
| 5 | `crates/service/src/workers/device_action_watchdog.rs` |
| 5 | `crates/service/src/workers/device_heartbeat.rs` |
| 4 | `crates/service/src/devices/analytics_store/builds.rs` |
| 4 | `crates/service/src/devices/postgres/builds.rs` |
| 4 | `crates/service/src/devices/postgres/user_devices.rs` |
| 4 | `crates/service/src/socket/routes/web_cli.rs` |
| 3 | `crates/service/src/devices/analytics_store/state_usage.rs` |
| 3 | `crates/service/src/devices/uart_handlers.rs` |
| 3 | `crates/service/src/persistence/mod.rs` |
| 3 | `crates/service/src/socket/routes/terminal.rs` |
| 3 | `crates/service/src/workers/device_controller.rs` |
| 2 | `crates/service/src/reports/data_source.rs` |
| 2 | `crates/service/src/workers/ipl_distribution.rs` |
| 2 | `crates/service/src/workers/relay_service.rs` |
| 2 | `crates/service/src/workers/test_preparation.rs` |
| 1 | `crates/service/src/devices/device_export_store.rs` |
| 1 | `crates/service/src/devices/postgres/power_relays.rs` |
| 1 | `crates/service/src/devices/test_completion_store.rs` |
| 1 | `crates/service/src/devices/test_export.rs` |
| 1 | `crates/service/src/workers/build_preparation.rs` |
| 1 | `crates/service/src/workers/device_action.rs` |

**Total: 305** across **46** files

### By API

| API | Count |
|-----|------:|
| `sqlx::query` | 122 |
| `sqlx::query_scalar` | 119 |
| `sqlx::query_as` | 64 |

### By SQL verb

| Verb | Count |
|------|------:|
| SELECT | 153 |
| UPDATE | 82 |
| INSERT | 41 |
| DELETE | 21 |
| WITH/SELECT | 7 |
| WITH/INSERT | 1 |

## 2. Tables lacking a SeaORM entity

Existing entities under `crates/service/src/persistence/entity/` map to tables:

`alerts`, `device_action_queue`, `device_controller_heartbeats`, `device_controller_metrics`, `device_controllers`, `device_interfaces`, `device_state_change`, `device_type_folders`, `devices`, `execution_reports`, `fallback_updates`, `faulty_reports`, `gen5_mapping_queue`, `heartbeats`, `log_entries`, `pending_relay_configs`, `relay_channels`, `relays`, `releases`, `test_executions`, `testcase`, `upload_builds`

### Referenced in raw SQL but **no** entity module

| Table | Refs | Notes |
|-------|-----:|-------|
| `device_uart_mappings` | 2 | UART terminal routing; insert/select in uart_handlers + terminal socket |
| `gen5_reboot_attempts` | 4 | Used by gen5 reboot watchdog; schema `farmcontroller.gen5_reboot_attempts` |
| `retention_policies` | 1 | Schema `farmcontroller.retention_policies`; drives dynamic DELETE via RetentionTarget |

CTE aliases and SQL functions (e.g. `claimed`, `durations`, `jsonb_each`) were excluded from this list.

## 3. Priority groups (migration order)

| Priority | Intent | Occurrences |
|----------|--------|------------:|
| **P0** | Hot path: heartbeats, device state, test execution, callbacks, action queue workers | 84 |
| **P1** | Core API CRUD: devices, relays, registration, controllers, inventory, alerts, sockets | 145 |
| **P2** | Builds, reports, analytics, exports, artifact scanning | 73 |
| **P3** | Retention / persistence plumbing / dynamic SQL | 3 |

### P0 files

- `crates/service/src/devices/callback_store.rs` (13)
- `crates/service/src/devices/flashing_store.rs` (5)
- `crates/service/src/devices/heartbeat_store/controller.rs` (6)
- `crates/service/src/test_results.rs` (14)
- `crates/service/src/workers/device_action.rs` (1)
- `crates/service/src/workers/device_action_watchdog.rs` (5)
- `crates/service/src/workers/device_controller.rs` (3)
- `crates/service/src/workers/device_heartbeat.rs` (5)
- `crates/service/src/workers/fallback_flash.rs` (6)
- `crates/service/src/workers/gen5_mapping.rs` (10)
- `crates/service/src/workers/gen5_reboot_watchdog.rs` (12)
- `crates/service/src/workers/relay_service.rs` (2)
- `crates/service/src/workers/test_preparation.rs` (2)

### P1 files

- `crates/service/src/devices/alert_store.rs` (7)
- `crates/service/src/devices/controller_store.rs` (6)
- `crates/service/src/devices/device_registration_store.rs` (13)
- `crates/service/src/devices/legacy_relay_store.rs` (23)
- `crates/service/src/devices/postgres/controllers.rs` (16)
- `crates/service/src/devices/postgres/inventory.rs` (10)
- `crates/service/src/devices/postgres/power_relays.rs` (1)
- `crates/service/src/devices/postgres/user_devices.rs` (4)
- `crates/service/src/devices/registration_store.rs` (13)
- `crates/service/src/devices/relay_configuration_store/apply.rs` (8)
- `crates/service/src/devices/relay_configuration_store/confirmation.rs` (6)
- `crates/service/src/devices/relay_store.rs` (16)
- `crates/service/src/devices/uart_handlers.rs` (3)
- `crates/service/src/socket/routes/terminal.rs` (3)
- `crates/service/src/socket/routes/web_cli.rs` (4)
- `crates/service/src/workers/artifact_scanner.rs` (9)
- `crates/service/src/workers/build_preparation.rs` (1)
- `crates/service/src/workers/ipl_distribution.rs` (2)

### P2 files

- `crates/service/src/devices/analytics_store/builds.rs` (4)
- `crates/service/src/devices/analytics_store/executions.rs` (7)
- `crates/service/src/devices/analytics_store/state_usage.rs` (3)
- `crates/service/src/devices/build_store/mutations.rs` (6)
- `crates/service/src/devices/build_store/queries.rs` (5)
- `crates/service/src/devices/build_store/uploads.rs` (7)
- `crates/service/src/devices/device_export_store.rs` (1)
- `crates/service/src/devices/faulty_report_store.rs` (11)
- `crates/service/src/devices/postgres/builds.rs` (4)
- `crates/service/src/devices/postgres/execution_queries.rs` (10)
- `crates/service/src/devices/postgres/execution_reports.rs` (11)
- `crates/service/src/devices/test_completion_store.rs` (1)
- `crates/service/src/devices/test_export.rs` (1)
- `crates/service/src/reports/data_source.rs` (2)

### P3 files

- `crates/service/src/persistence/mod.rs` (3)

## 4. Full occurrence inventory

All migration statuses are **PENDING**. Suggested SeaORM approaches are starting points — prefer typed Entity APIs; use `Statement` only when irreducible (SKIP LOCKED claims, heavy JSON aggregation, dynamic retention deletes).

### `crates/service/src/devices/alert_store.rs` (7)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 1 | 14 | `query_scalar` | SELECT | `alerts`, `devices` | P1 | list admin: count rows in alerts, devices | Entity::find().find_also_related / join / select_only | PENDING |
| 2 | 35 | `query_scalar` | SELECT | `alerts`, `devices` | P1 | list admin: fetch JSON projection of alerts, devices | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 3 | 101 | `query_scalar` | SELECT | `alerts` | P1 | list all: count rows in alerts | Entity::find().filter(...).count() | PENDING |
| 4 | 111 | `query_scalar` | SELECT | `alerts` | P1 | list all: count rows in alerts | Entity::find().filter(...).count() | PENDING |
| 5 | 121 | `query_scalar` | SELECT | `alerts` | P1 | list all: query alerts | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 6 | 157 | `query` | UPDATE | `alerts` | P1 | mark read: update alerts | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 7 | 171 | `query` | UPDATE | `alerts` | P1 | mark all read: update alerts | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/devices/analytics_store/builds.rs` (4)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 8 | 11 | `query_scalar` | WITH/SELECT | `releases`, `test_executions`, `testcase` | P2 | build comparisons: count rows in releases, test_executions, testcase | Statement if CTE/analytics irreducible; else multi-step Entity::find | PENDING |
| 9 | 60 | `query_scalar` | SELECT | `releases`, `test_executions`, `testcase` | P2 | build comparison by id: existence check on releases, test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 10 | 93 | `query_scalar` | WITH/SELECT | `device_action_queue`, `releases`, `test_executions`, `testcase` | P2 | build performance: count rows in device_action_queue, releases, test_executions, testcase | Statement if CTE/analytics irreducible; else multi-step Entity::find | PENDING |
| 11 | 142 | `query_scalar` | SELECT | `device_action_queue`, `releases`, `test_executions`, `testcase` | P2 | build performance by id: count rows in device_action_queue, releases, test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/analytics_store/executions.rs` (7)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 12 | 18 | `query_scalar` | SELECT | `test_executions`, `testcase` | P2 | recent executions: count rows in test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 13 | 40 | `query_scalar` | SELECT | `test_executions`, `testcase` | P2 | execution by id: count rows in test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 14 | 62 | `query_scalar` | WITH/SELECT | `test_executions`, `testcase` | P2 | execution daily: count rows in test_executions, testcase | Statement if CTE/analytics irreducible; else multi-step Entity::find | PENDING |
| 15 | 95 | `query_as` | SELECT | `test_executions`, `testcase` | P2 | test execution analytics: query test_executions, testcase | Entity::find().find_also_related / join / select_only | PENDING |
| 16 | 160 | `query_scalar` | SELECT | `test_executions`, `testcase` | P2 | in progress test analytics: count rows in test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 17 | 180 | `query_scalar` | SELECT | `test_executions`, `testcase` | P2 | test plan summary: existence check on test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 18 | 205 | `query_scalar` | WITH/SELECT | `test_executions`, `testcase` | P2 | daily test summary: count rows in test_executions, testcase | Statement if CTE/analytics irreducible; else multi-step Entity::find | PENDING |

### `crates/service/src/devices/analytics_store/state_usage.rs` (3)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 19 | 29 | `query_as` | SELECT | `devices` | P2 | device state counts: count rows in devices | Entity::find().filter(...).one()/all() | PENDING |
| 20 | 47 | `query_as` | SELECT | `devices` | P2 | device state details: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 21 | 78 | `query_scalar` | WITH/SELECT | `device_state_change`, `devices` | P2 | device usage: count rows in device_state_change, devices | Statement if CTE/analytics irreducible; else multi-step Entity::find | PENDING |

### `crates/service/src/devices/build_store/mutations.rs` (6)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 22 | 15 | `query_as` | UPDATE | `releases` | P2 | update build status: update releases | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 23 | 29 | `query_scalar` | INSERT | `alerts` | P2 | update build status: insert into alerts | ActiveModel::insert / Entity::insert | PENDING |
| 24 | 65 | `query_as` | UPDATE | `releases` | P2 | flag build: update releases | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 25 | 85 | `query_scalar` | INSERT | `alerts` | P2 | flag build: insert into alerts | ActiveModel::insert / Entity::insert | PENDING |
| 26 | 120 | `query_as` | DELETE | `releases` | P2 | delete build: delete from releases | Entity::delete_many().filter(...) | PENDING |
| 27 | 132 | `query_scalar` | INSERT | `alerts` | P2 | delete build: insert into alerts | ActiveModel::insert / Entity::insert | PENDING |

### `crates/service/src/devices/build_store/queries.rs` (5)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 28 | 7 | `query_scalar` | SELECT | `releases` | P2 | build filters: query releases | Entity::find().filter(...).one()/all() | PENDING |
| 29 | 12 | `query_scalar` | SELECT | `releases` | P2 | build filters: query releases | Entity::find().filter(...).one()/all() | PENDING |
| 30 | 28 | `query_scalar` | SELECT | `releases` | P2 | build by id: fetch JSON projection of releases | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 31 | 55 | `query_scalar` | SELECT | `releases` | P2 | list builds: count rows in releases | Entity::find().filter(...).count() | PENDING |
| 32 | 58 | `query_scalar` | SELECT | `releases` | P2 | list builds: fetch JSON projection of releases | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/build_store/uploads.rs` (7)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 33 | 71 | `query_as` | SELECT | `device_type_folders` | P2 | finalize upload: query device_type_folders | Entity::find().filter(...).one()/all() | PENDING |
| 34 | 131 | `query_scalar` | INSERT | `releases` | P2 | finalize upload: insert into releases returning JSON | ActiveModel::insert then into_json / Entity::insert | PENDING |
| 35 | 159 | `query` | UPDATE | `upload_builds` | P2 | finalize upload: update upload_builds | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 36 | 169 | `query_scalar` | INSERT | `alerts` | P2 | finalize upload: insert into alerts | ActiveModel::insert / Entity::insert | PENDING |
| 37 | 205 | `query` | INSERT | `upload_builds` | P2 | init upload: insert into upload_builds | ActiveModel::insert / Entity::insert | PENDING |
| 38 | 223 | `query` | UPDATE | `upload_builds` | P2 | mark upload started: update upload_builds | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 39 | 241 | `query` | UPDATE | `upload_builds` | P2 | mark upload failed: update upload_builds | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/devices/callback_store.rs` (13)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 40 | 23 | `query_scalar` | SELECT | `device_controllers` | P0 | save gen5 mapping: query device_controllers | Entity::find().filter(...).one()/all() | PENDING |
| 41 | 45 | `query` | UPDATE | `device_controllers` | P0 | save gen5 mapping: update device_controllers | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 42 | 57 | `query` | UPDATE | `gen5_mapping_queue` | P0 | save gen5 mapping: claim rows with FOR UPDATE SKIP LOCKED (gen5_mapping_queue) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 43 | 80 | `query` | UPDATE | `gen5_mapping_queue` | P0 | save gen5 mapping: claim rows with FOR UPDATE SKIP LOCKED (gen5_mapping_queue) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 44 | 119 | `query_as` | SELECT | `device_action_queue` | P0 | confirm flash: claim rows with FOR UPDATE SKIP LOCKED (device_action_queue) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 45 | 140 | `query` | UPDATE | `device_action_queue` | P0 | confirm flash: update device_action_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 46 | 152 | `query` | INSERT | `log_entries` | P0 | confirm flash: insert into log_entries | ActiveModel::insert / Entity::insert | PENDING |
| 47 | 170 | `query_as` | SELECT | `test_executions` | P0 | confirm flash: query test_executions | Entity::find().filter(...).one()/all() | PENDING |
| 48 | 199 | `query` | UPDATE | `device_action_queue` | P0 | confirm flash: update device_action_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 49 | 211 | `query_as` | UPDATE | `test_executions` | P0 | confirm flash: update test_executions | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 50 | 229 | `query` | INSERT | `log_entries` | P0 | confirm flash: insert into log_entries | ActiveModel::insert / Entity::insert | PENDING |
| 51 | 243 | `query_scalar` | UPDATE | `devices` | P0 | confirm flash: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 52 | 256 | `query` | INSERT | `device_state_change` | P0 | confirm flash: insert into device_state_change | ActiveModel::insert / Entity::insert | PENDING |

### `crates/service/src/devices/controller_store.rs` (6)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 53 | 14 | `query_scalar` | SELECT | `device_controllers` | P1 | edit: existence check on device_controllers | Entity::find().filter(...).count() > 0 | PENDING |
| 54 | 30 | `query` | DELETE | `relay_channels`, `relays` | P1 | edit: delete from relay_channels, relays | Entity::delete_many().filter(...) | PENDING |
| 55 | 37 | `query` | DELETE | `relays` | P1 | edit: delete from relays | Entity::delete_many().filter(...) | PENDING |
| 56 | 42 | `query_scalar` | UPDATE | `device_controllers` | P1 | edit: update device_controllers | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 57 | 66 | `query_as` | SELECT | `device_controllers` | P1 | delete target: query device_controllers | Entity::find().filter(...).one()/all() | PENDING |
| 58 | 87 | `query` | DELETE | `device_controllers` | P1 | delete: delete from device_controllers | Entity::delete_many().filter(...) | PENDING |

### `crates/service/src/devices/device_export_store.rs` (1)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 59 | 11 | `query_scalar` | SELECT | `device_interfaces`, `devices` | P2 | export devices: fetch JSON projection of device_interfaces, devices | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/device_registration_store.rs` (13)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 60 | 18 | `query_as` | SELECT | `devices` | P1 | register device: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 61 | 33 | `query` | UPDATE | `devices` | P1 | register device: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 62 | 57 | `query` | UPDATE | `devices` | P1 | register device: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 63 | 79 | `query` | INSERT | `device_state_change` | P1 | register device: insert into device_state_change | ActiveModel::insert / Entity::insert | PENDING |
| 64 | 89 | `query` | INSERT | `devices` | P1 | register device: insert into devices | ActiveModel::insert / Entity::insert | PENDING |
| 65 | 112 | `query` | INSERT | `alerts` | P1 | register device: insert into alerts | ActiveModel::insert / Entity::insert | PENDING |
| 66 | 126 | `query` | DELETE | `device_interfaces` | P1 | register device: delete from device_interfaces | Entity::delete_many().filter(...) | PENDING |
| 67 | 131 | `query` | INSERT | `device_interfaces` | P1 | register device: insert into device_interfaces | ActiveModel::insert / Entity::insert | PENDING |
| 68 | 147 | `query` | INSERT | `device_type_folders` | P1 | register device: insert into device_type_folders | ActiveModel insert + OnConflict / Statement if complex | PENDING |
| 69 | 161 | `query_scalar` | SELECT | `device_interfaces`, `devices` | P1 | register device: fetch JSON projection of device_interfaces, devices | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 70 | 175 | `query` | UPDATE | `devices` | P1 | register device: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 71 | 190 | `query` | INSERT | `gen5_mapping_queue` | P1 | register device: insert into gen5_mapping_queue | ActiveModel::insert / Entity::insert | PENDING |
| 72 | 212 | `query` | UPDATE | `device_controllers` | P1 | register device: update device_controllers | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/devices/faulty_report_store.rs` (11)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 73 | 14 | `query_scalar` | SELECT | `releases` | P2 | create: existence check on releases | Entity::find().filter(...).count() > 0 | PENDING |
| 74 | 27 | `query_scalar` | SELECT | `test_executions` | P2 | create: query test_executions | Entity::find().filter(...).one()/all() | PENDING |
| 75 | 39 | `query_scalar` | SELECT | `devices` | P2 | create: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 76 | 54 | `query` | INSERT | `faulty_reports` | P2 | create: insert into faulty_reports | ActiveModel::insert / Entity::insert | PENDING |
| 77 | 83 | `query_scalar` | UPDATE | `faulty_reports` | P2 | finalize: update faulty_reports | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 78 | 94 | `query_scalar` | WITH/INSERT | `alerts`, `faulty_reports`, `releases` | P2 | finalize: insert into alerts, faulty_reports, releases | Statement if CTE/analytics irreducible; else multi-step Entity::find | PENDING |
| 79 | 132 | `query_scalar` | SELECT | `faulty_reports` | P2 | list: fetch JSON projection of faulty_reports | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 80 | 144 | `query_scalar` | SELECT | `faulty_reports`, `releases` | P2 | by id: fetch JSON projection of faulty_reports, releases | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 81 | 164 | `query_scalar` | UPDATE | `faulty_reports` | P2 | update status: update faulty_reports | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 82 | 175 | `query` | UPDATE | `faulty_reports`, `releases` | P2 | update status: update faulty_reports, releases | update_many or fetch+ActiveModel; Statement if UPDATE…FROM | PENDING |
| 83 | 189 | `query` | DELETE | `faulty_reports` | P2 | delete: delete from faulty_reports | Entity::delete_many().filter(...) | PENDING |

### `crates/service/src/devices/flashing_store.rs` (5)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 84 | 25 | `query_as` | SELECT | `devices` | P0 | mark device flashing: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 85 | 46 | `query_as` | SELECT | `device_action_queue` | P0 | mark device flashing: query device_action_queue | Entity::find().filter(...).one()/all() | PENDING |
| 86 | 72 | `query_scalar` | UPDATE | `devices` | P0 | mark device flashing: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 87 | 89 | `query_as` | SELECT | `device_controllers` | P0 | mark device flashing: query device_controllers | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 88 | 113 | `query_as` | SELECT | `device_controllers`, `relay_channels`, `relays` | P0 | mark device flashing: query device_controllers, relay_channels, relays | Entity::find().find_also_related / join / select_only | PENDING |

### `crates/service/src/devices/heartbeat_store/controller.rs` (6)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 89 | 20 | `query_scalar` | SELECT | `device_controllers` | P0 | record controller: query device_controllers | Entity::find().filter(...).one()/all() | PENDING |
| 90 | 34 | `query` | UPDATE | `device_controllers` | P0 | record controller: update device_controllers | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 91 | 50 | `query` | INSERT | `device_controller_heartbeats` | P0 | record controller: insert into device_controller_heartbeats | ActiveModel::insert / Entity::insert | PENDING |
| 92 | 65 | `query` | UPDATE | `device_controller_metrics` | P0 | record controller: update device_controller_metrics | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 93 | 97 | `query` | INSERT | `device_controller_metrics` | P0 | record controller: insert into device_controller_metrics | ActiveModel::insert / Entity::insert | PENDING |
| 94 | 126 | `query_scalar` | INSERT | `alerts` | P0 | record controller: insert into alerts returning JSON | ActiveModel::insert then into_json / Entity::insert | PENDING |

### `crates/service/src/devices/legacy_relay_store.rs` (23)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 95 | 34 | `query_scalar` | SELECT | `relays` | P1 | list relays: fetch JSON projection of relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 96 | 50 | `query_scalar` | SELECT | `relay_channels` | P1 | list channels: fetch JSON projection of relay_channels | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 97 | 68 | `query_scalar` | SELECT | `relays` | P1 | relay by id: fetch JSON projection of relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 98 | 82 | `query_scalar` | SELECT | `relay_channels` | P1 | channel by id: fetch JSON projection of relay_channels | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 99 | 96 | `query` | UPDATE | `relays` | P1 | delete relay: update relays | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 100 | 112 | `query` | UPDATE | `relay_channels` | P1 | delete channel: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 101 | 138 | `query_as` | SELECT | `device_controllers`, `devices`, `relay_channels`, `relays` | P1 | configure: query device_controllers, devices, relay_channels, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 102 | 158 | `query` | UPDATE | `relay_channels` | P1 | configure: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 103 | 166 | `query` | UPDATE | `relay_channels` | P1 | configure: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 104 | 191 | `query_as` | SELECT | `relays` | P1 | fresh: query relays | Entity::find().filter(...).one()/all() | PENDING |
| 105 | 199 | `query` | DELETE | `relays` | P1 | fresh: delete from relays | Entity::delete_many().filter(...) | PENDING |
| 106 | 217 | `query_as` | SELECT | `device_controllers`, `relays` | P1 | remap: query device_controllers, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 107 | 229 | `query_scalar` | SELECT | `relays` | P1 | remap: query relays | Entity::find().filter(...).one()/all() | PENDING |
| 108 | 239 | `query` | UPDATE | `relay_channels` | P1 | remap: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 109 | 247 | `query` | DELETE | `relays` | P1 | remap: delete from relays | Entity::delete_many().filter(...) | PENDING |
| 110 | 252 | `query_as` | SELECT | `devices`, `relay_channels` | P1 | remap: fetch JSON projection of devices, relay_channels | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 111 | 286 | `query_scalar` | SELECT | `relays` | P1 | check conflicts: query relays | Entity::find().filter(...).one()/all() | PENDING |
| 112 | 295 | `query_scalar` | SELECT | `relay_channels` | P1 | check conflicts: existence check on relay_channels | Entity::find().filter(...).count() > 0 | PENDING |
| 113 | 309 | `query_scalar` | SELECT | `relay_channels` | P1 | check conflicts: existence check on relay_channels | Entity::find().filter(...).count() > 0 | PENDING |
| 114 | 322 | `query_scalar` | SELECT | `relays` | P1 | check conflicts: query relays | Entity::find().filter(...).one()/all() | PENDING |
| 115 | 332 | `query_scalar` | SELECT | `relay_channels` | P1 | check conflicts: existence check on relay_channels | Entity::find().filter(...).count() > 0 | PENDING |
| 116 | 339 | `query` | DELETE | `relays` | P1 | check conflicts: delete from relays | Entity::delete_many().filter(...) | PENDING |
| 117 | 360 | `query_scalar` | SELECT | `relay_channels` | P1 | channel json: fetch JSON projection of relay_channels | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/postgres/builds.rs` (4)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 118 | 91 | `query_scalar` | SELECT | `device_type_folders`, `releases` | P2 | builds for device type: query device_type_folders, releases | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 119 | 114 | `query_as` | INSERT | `device_type_folders` | P2 | configure artifacts: insert into device_type_folders | ActiveModel insert + OnConflict / Statement if complex | PENDING |
| 120 | 142 | `query_as` | SELECT | `device_type_folders` | P2 | artifact folders: query device_type_folders | Entity::find().filter(...).one()/all() | PENDING |
| 121 | 163 | `query_scalar` | SELECT | `device_type_folders`, `devices` | P2 | artifact folder for device: query device_type_folders, devices | Entity::find().find_also_related / join / select_only | PENDING |

### `crates/service/src/devices/postgres/controllers.rs` (16)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 122 | 10 | `query_as` | SELECT | `devices` | P1 | device action target: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 123 | 33 | `query_scalar` | SELECT | `devices` | P1 | apply device action: fetch JSON projection of devices | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 124 | 46 | `query` | INSERT | `log_entries` | P1 | apply device action: insert into log_entries | ActiveModel::insert / Entity::insert | PENDING |
| 125 | 58 | `query` | DELETE | `device_interfaces` | P1 | apply device action: delete from device_interfaces | Entity::delete_many().filter(...) | PENDING |
| 126 | 62 | `query` | DELETE | `devices` | P1 | apply device action: delete from devices | Entity::delete_many().filter(...) | PENDING |
| 127 | 68 | `query_scalar` | UPDATE | `devices` | P1 | apply device action: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 128 | 86 | `query` | INSERT | `device_state_change` | P1 | apply device action: insert into device_state_change | ActiveModel::insert / Entity::insert | PENDING |
| 129 | 119 | `query_scalar` | SELECT | `device_controllers` | P1 | list controllers: count rows in device_controllers | Entity::find().filter(...).count() | PENDING |
| 130 | 133 | `query_scalar` | SELECT | `device_controllers`, `devices`, `relay_channels`, `relays` | P1 | list controllers: fetch JSON projection of device_controllers, devices, relay_channels, relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 131 | 182 | `query_as` | SELECT | `device_controllers` | P1 | list controllers: count rows in device_controllers | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 132 | 189 | `query_as` | SELECT | `relays` | P1 | list controllers: count rows in relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 133 | 236 | `query_as` | SELECT | `device_controllers`, `devices`, `relay_channels`, `relays` | P1 | device delete target: query device_controllers, devices, relay_channels, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 134 | 257 | `query` | UPDATE | `device_controllers` | P1 | delete device: update device_controllers | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 135 | 274 | `query` | DELETE | `device_interfaces` | P1 | delete device: delete from device_interfaces | Entity::delete_many().filter(...) | PENDING |
| 136 | 278 | `query` | DELETE | `relay_channels` | P1 | delete device: delete from relay_channels | Entity::delete_many().filter(...) | PENDING |
| 137 | 282 | `query` | DELETE | `devices` | P1 | delete device: delete from devices | Entity::delete_many().filter(...) | PENDING |

### `crates/service/src/devices/postgres/execution_queries.rs` (10)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 138 | 25 | `query_scalar` | SELECT | `devices`, `test_executions` | P2 | list test executions: count rows in devices, test_executions | Entity::find().find_also_related / join / select_only | PENDING |
| 139 | 29 | `query_scalar` | SELECT | `devices`, `test_executions`, `testcase` | P2 | list test executions: count rows in devices, test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 140 | 67 | `query_scalar` | SELECT | `test_executions` | P2 | test execution by device: fetch JSON projection of test_executions | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 141 | 83 | `query_scalar` | SELECT | `test_executions` | P2 | in progress test executions: query test_executions | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 142 | 98 | `query_scalar` | SELECT | `test_executions`, `testcase` | P2 | single test execution: fetch JSON projection of test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 143 | 114 | `query_scalar` | SELECT | `test_executions` | P2 | execution case ids: existence check on test_executions | Entity::find().filter(...).count() > 0 | PENDING |
| 144 | 125 | `query_scalar` | SELECT | `testcase` | P2 | execution case ids: query testcase | Entity::find().filter(...).one()/all() | PENDING |
| 145 | 141 | `query_scalar` | SELECT | `devices`, `test_executions` | P2 | test execution: fetch JSON projection of devices, test_executions | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 146 | 160 | `query_scalar` | SELECT | `devices`, `test_executions`, `testcase` | P2 | test execution summary: fetch JSON projection of devices, test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 147 | 183 | `query_scalar` | WITH/SELECT | `test_executions`, `testcase` | P2 | test execution summary: count rows in test_executions, testcase | Statement if CTE/analytics irreducible; else multi-step Entity::find | PENDING |

### `crates/service/src/devices/postgres/execution_reports.rs` (11)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 148 | 10 | `query_as` | SELECT | `test_executions` | P2 | report generation target: query test_executions | Entity::find().filter(...).one()/all() | PENDING |
| 149 | 23 | `query_scalar` | SELECT | `execution_reports` | P2 | create execution report record: query execution_reports | Entity::find().filter(...).one()/all() | PENDING |
| 150 | 29 | `query` | DELETE | `execution_reports` | P2 | create execution report record: delete from execution_reports | Entity::delete_many().filter(...) | PENDING |
| 151 | 33 | `query_scalar` | INSERT | `execution_reports` | P2 | create execution report record: insert into execution_reports returning JSON | ActiveModel::insert then into_json / Entity::insert | PENDING |
| 152 | 46 | `query` | UPDATE | `execution_reports` | P2 | update execution report status: update execution_reports | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 153 | 55 | `query_scalar` | SELECT | `execution_reports` | P2 | execution report: fetch JSON projection of execution_reports | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 154 | 69 | `query_as` | SELECT | `test_executions` | P2 | execution report target: query test_executions | Entity::find().filter(...).one()/all() | PENDING |
| 155 | 79 | `query_scalar` | SELECT | `testcase` | P2 | test case log path: query testcase | Entity::find().filter(...).one()/all() | PENDING |
| 156 | 94 | `query_scalar` | SELECT | `test_executions` | P2 | execution cases: existence check on test_executions | Entity::find().filter(...).count() > 0 | PENDING |
| 157 | 98 | `query_scalar` | SELECT | `testcase` | P2 | execution cases: query testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 158 | 112 | `query_scalar` | SELECT | `testcase` | P2 | test case: fetch JSON projection of testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/postgres/inventory.rs` (10)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 159 | 7 | `query_scalar` | SELECT | `devices` | P1 | device families: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 160 | 24 | `query_scalar` | SELECT | `devices` | P1 | device types: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 161 | 43 | `query_scalar` | SELECT | `device_interfaces`, `devices`, `test_executions` | P1 | device by id: fetch JSON projection of device_interfaces, devices, test_executions | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 162 | 74 | `query_scalar` | SELECT | `heartbeats` | P1 | latest heartbeat: query heartbeats | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 163 | 93 | `query_scalar` | SELECT | `device_controller_heartbeats` | P1 | latest heartbeat: query device_controller_heartbeats | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 164 | 113 | `query_scalar` | SELECT | `devices`, `relay_channels`, `relays` | P1 | topology: query devices, relay_channels, relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 165 | 168 | `query_scalar` | SELECT | `devices` | P1 | list devices: count rows in devices | Entity::find().filter(...).count() | PENDING |
| 166 | 190 | `query_scalar` | SELECT | `device_interfaces`, `devices`, `heartbeats`, `relay_channels`, `relays` | P1 | list devices: fetch JSON projection of device_interfaces, devices, heartbeats, relay_channels, relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 167 | 246 | `query_scalar` | SELECT | `devices` | P1 | list devices: count rows in devices | Entity::find().filter(...).count() | PENDING |
| 168 | 251 | `query_as` | SELECT | `devices` | P1 | list devices: count rows in devices | Entity::find().filter(...).one()/all() | PENDING |

### `crates/service/src/devices/postgres/power_relays.rs` (1)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 169 | 10 | `query_as` | SELECT | `device_controllers`, `devices` | P1 | device reboot target: query device_controllers, devices | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/postgres/user_devices.rs` (4)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 170 | 24 | `query_scalar` | SELECT | `devices` | P1 | list user devices: count rows in devices | Entity::find().filter(...).count() | PENDING |
| 171 | 38 | `query_scalar` | SELECT | `device_interfaces`, `devices`, `test_executions`, `testcase` | P1 | list user devices: fetch JSON projection of device_interfaces, devices, test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 172 | 88 | `query_scalar` | SELECT | `device_interfaces`, `devices` | P1 | active devices: fetch JSON projection of device_interfaces, devices | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 173 | 106 | `query` | UPDATE | `devices` | P1 | update heartbeat timeout: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/devices/registration_store.rs` (13)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 174 | 34 | `query_as` | INSERT | `device_controllers` | P1 | register controller: insert into device_controllers | ActiveModel insert + OnConflict / Statement if complex | PENDING |
| 175 | 74 | `query` | INSERT | `alerts` | P1 | register controller: insert into alerts | ActiveModel::insert / Entity::insert | PENDING |
| 176 | 118 | `query_scalar` | SELECT | `relays` | P1 | sync relay inventory: query relays | Entity::find().filter(...).one()/all() | PENDING |
| 177 | 137 | `query_scalar` | SELECT | `relays` | P1 | sync relay inventory: query relays | Entity::find().filter(...).one()/all() | PENDING |
| 178 | 154 | `query_as` | SELECT | `relays` | P1 | sync relay inventory: query relays | Entity::find().filter(...).one()/all() | PENDING |
| 179 | 166 | `query` | UPDATE | `relays` | P1 | sync relay inventory: update relays | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 180 | 203 | `query` | INSERT | `relays` | P1 | create relay: insert into relays | ActiveModel::insert / Entity::insert | PENDING |
| 181 | 222 | `query_scalar` | SELECT | `relay_channels` | P1 | retire relay: count rows in relay_channels | Entity::find().filter(...).count() | PENDING |
| 182 | 227 | `query` | DELETE | `relays` | P1 | retire relay: delete from relays | Entity::delete_many().filter(...) | PENDING |
| 183 | 245 | `query` | UPDATE | `relay_channels` | P1 | soft delete relays: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 184 | 253 | `query` | UPDATE | `relays` | P1 | soft delete relays: update relays | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 185 | 271 | `query` | INSERT | `relay_channels` | P1 | upsert reported channels: insert into relay_channels | ActiveModel insert + OnConflict / Statement if complex | PENDING |
| 186 | 295 | `query_as` | SELECT | `relays` | P1 | load relays: query relays | Entity::find().filter(...).one()/all() | PENDING |

### `crates/service/src/devices/relay_configuration_store/apply.rs` (8)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 187 | 111 | `query_as` | SELECT | `relay_channels`, `relays` | P1 | configure channel: query relay_channels, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 188 | 158 | `query` | UPDATE | `relay_channels` | P1 | configure channel: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 189 | 174 | `query` | UPDATE | `devices` | P1 | configure channel: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 190 | 182 | `query` | INSERT | `pending_relay_configs` | P1 | configure channel: insert into pending_relay_configs | ActiveModel::insert / Entity::insert | PENDING |
| 191 | 212 | `query_as` | SELECT | `device_controllers`, `relay_channels`, `relays` | P1 | load channel: query device_controllers, relay_channels, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 192 | 242 | `query_as` | SELECT | `devices` | P1 | load device: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 193 | 258 | `query` | UPDATE | `device_controllers`, `relays` | P1 | save mappings: update device_controllers, relays | update_many or fetch+ActiveModel; Statement if UPDATE…FROM | PENDING |
| 194 | 276 | `query_scalar` | SELECT | `relay_channels` | P1 | load channel json: fetch JSON projection of relay_channels | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/relay_configuration_store/confirmation.rs` (6)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 195 | 19 | `query_as` | SELECT | `pending_relay_configs`, `relay_channels`, `relays` | P1 | confirm: query pending_relay_configs, relay_channels, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 196 | 45 | `query` | UPDATE | `pending_relay_configs` | P1 | confirm: update pending_relay_configs | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 197 | 62 | `query` | UPDATE | `pending_relay_configs` | P1 | confirm: update pending_relay_configs | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 198 | 69 | `query` | UPDATE | `relay_channels` | P1 | confirm: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 199 | 76 | `query` | UPDATE | `devices`, `relay_channels` | P1 | confirm: update devices, relay_channels | update_many or fetch+ActiveModel; Statement if UPDATE…FROM | PENDING |
| 200 | 85 | `query_scalar` | SELECT | `pending_relay_configs`, `relay_channels` | P1 | confirm: count rows in pending_relay_configs, relay_channels | Entity::find().find_also_related / join / select_only | PENDING |

### `crates/service/src/devices/relay_store.rs` (16)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 201 | 16 | `query_as` | SELECT | `device_controllers`, `devices`, `relay_channels`, `relays` | P1 | device power target: query device_controllers, devices, relay_channels, relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 202 | 51 | `query` | UPDATE | `relay_channels` | P1 | apply device power: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 203 | 60 | `query` | UPDATE | `devices` | P1 | apply device power: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 204 | 70 | `query_scalar` | SELECT | `relay_channels` | P1 | apply device power: fetch JSON projection of relay_channels | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 205 | 90 | `query_scalar` | SELECT | `devices`, `relay_channels`, `relays` | P1 | relays for controller: fetch JSON projection of devices, relay_channels, relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 206 | 113 | `query_scalar` | SELECT | `relays` | P1 | channels for relay: existence check on relays | Entity::find().filter(...).count() > 0 | PENDING |
| 207 | 123 | `query_scalar` | SELECT | `devices`, `relay_channels` | P1 | channels for relay: fetch JSON projection of devices, relay_channels | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 208 | 147 | `query_scalar` | SELECT | `device_controllers`, `relays` | P1 | available devices: query device_controllers, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 209 | 160 | `query_scalar` | SELECT | `devices`, `relay_channels` | P1 | available devices: existence check on devices, relay_channels | Entity::find().find_also_related / join / select_only | PENDING |
| 210 | 173 | `query_scalar` | SELECT | `devices`, `relay_channels` | P1 | available devices: existence check on devices, relay_channels | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 211 | 206 | `query_as` | SELECT | `device_controllers`, `relay_channels`, `relays` | P1 | state target: query device_controllers, relay_channels, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 212 | 228 | `query` | UPDATE | `relay_channels` | P1 | update state: update relay_channels | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 213 | 244 | `query_as` | SELECT | `device_controllers`, `relays` | P1 | identity target: query device_controllers, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 214 | 267 | `query_scalar` | SELECT | `relays` | P1 | update identity: existence check on relays | Entity::find().filter(...).count() > 0 | PENDING |
| 215 | 280 | `query` | UPDATE | `relays` | P1 | update identity: update relays | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 216 | 294 | `query` | UPDATE | `pending_relay_configs` | P1 | update identity: update pending_relay_configs | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/devices/test_completion_store.rs` (1)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 217 | 15 | `query_as` | SELECT | `device_controllers`, `devices`, `relay_channels`, `relays`, `test_executions` | P2 | test completion target: query device_controllers, devices, relay_channels, relays, test_executions | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/test_export.rs` (1)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 218 | 11 | `query_as` | SELECT | `devices`, `test_executions`, `testcase` | P2 | rows: count rows in devices, test_executions, testcase | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/devices/uart_handlers.rs` (3)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 219 | 67 | `query_as` | SELECT | `device_controllers`, `devices`, `pending_relay_configs`, `relay_channels`, `relays` | P1 | load context: query device_controllers, devices, pending_relay_configs, relay_channels, relays | Entity::find().find_also_related / join / select_only | PENDING |
| 220 | 290 | `query` | INSERT | `device_uart_mappings` | P1 | configure uart: insert into device_uart_mappings | ActiveModel insert + OnConflict / Statement if complex (add entity for device_uart_mappings first) | PENDING |
| 221 | 339 | `query` | UPDATE | `device_controllers` | P1 | configure uart: update device_controllers | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/persistence/mod.rs` (3)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 222 | 71 | `query_scalar` | SELECT | — | P3 | ping: readiness `SELECT 1` | `Statement` / keep as trivial scalar (or SeaORM `ConnectionTrait::execute`) | PENDING |
| 223 | 92 | `query_as` | SELECT | `retention_policies` | P3 | enabled retention policies: query retention_policies | Entity::find().filter(...).one()/all() (add entity for retention_policies first) | PENDING |
| 224 | 114 | `query` | DELETE | `(dynamic via retention_policies)` | P3 | clean retention batch: delete from (dynamic via retention_policies) | Statement (dynamic SQL from RetentionTarget / runtime); consider typed helpers per table | PENDING |

### `crates/service/src/reports/data_source.rs` (2)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 225 | 38 | `query_as` | SELECT | `testcase` | P2 | test cases: query testcase | Entity::find().filter(...).one()/all() | PENDING |
| 226 | 65 | `query` | UPDATE | `execution_reports` | P2 | update report status: update execution_reports | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/socket/routes/terminal.rs` (3)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 227 | 329 | `query_scalar` | SELECT | `devices` | P1 | approved device host: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 228 | 348 | `query_scalar` | SELECT | `device_controllers` | P1 | approved controller host: query device_controllers | Entity::find().filter(...).one()/all() | PENDING |
| 229 | 376 | `query_as` | SELECT | `device_controllers`, `device_uart_mappings`, `devices` | P1 | resolve rtos target: query device_controllers, device_uart_mappings, devices | Entity::find().find_also_related / join / select_only (add entity for device_uart_mappings first) | PENDING |

### `crates/service/src/socket/routes/web_cli.rs` (4)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 230 | 157 | `query_scalar` | SELECT | `devices` | P1 | run test: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 231 | 940 | `query_as` | SELECT | `devices` | P1 | list devices: query devices | Entity::find().filter(...).one()/all() | PENDING |
| 232 | 974 | `query_as` | SELECT | `devices` | P1 | device stats: count rows in devices | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |
| 233 | 1005 | `query_scalar` | SELECT | `device_interfaces`, `devices`, `heartbeats` | P1 | device detail: fetch JSON projection of device_interfaces, devices, heartbeats | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/test_results.rs` (14)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 234 | 85 | `query_scalar` | SELECT | `test_executions` | P0 | validate session: existence check on test_executions | Entity::find().filter(...).count() > 0 | PENDING |
| 235 | 125 | `query_as` | SELECT | `testcase` | P0 | process case: query testcase | Entity::find().filter(...).one()/all() | PENDING |
| 236 | 150 | `query_scalar` | UPDATE | `test_executions`, `testcase` | P0 | process case: update test_executions, testcase | update_many or fetch+ActiveModel; Statement if UPDATE…FROM | PENDING |
| 237 | 187 | `query` | UPDATE | `device_action_queue` | P0 | process case: update device_action_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 238 | 204 | `query_scalar` | SELECT | `testcase` | P0 | process case: existence check on testcase | Entity::find().filter(...).count() > 0 | PENDING |
| 239 | 269 | `query_as` | SELECT | `test_executions` | P0 | lock execution: query test_executions | Entity::find().filter(...).one()/all() | PENDING |
| 240 | 330 | `query` | UPDATE | `testcase` | P0 | sync jira defect: update testcase | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 241 | 367 | `query_as` | SELECT | `testcase` | P0 | catalog results: query testcase | Entity::find().filter(...).one()/all() | PENDING |
| 242 | 393 | `query` | UPDATE | `test_executions` | P0 | complete execution: update test_executions | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 243 | 403 | `query` | UPDATE | `device_action_queue` | P0 | complete execution: update device_action_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 244 | 411 | `query_scalar` | SELECT | `testcase` | P0 | complete execution: existence check on testcase | Entity::find().filter(...).count() > 0 | PENDING |
| 245 | 428 | `query` | UPDATE | `devices` | P0 | complete execution: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 246 | 436 | `query` | INSERT | `fallback_updates` | P0 | complete execution: insert into fallback_updates | ActiveModel::insert / Entity::insert | PENDING |
| 247 | 467 | `query` | INSERT | `log_entries` | P0 | insert log: insert into log_entries | ActiveModel::insert / Entity::insert | PENDING |

### `crates/service/src/workers/artifact_scanner.rs` (9)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 248 | 361 | `query_as` | SELECT | `device_type_folders` | P1 | persist results: query device_type_folders | Entity::find().filter(...).one()/all() | PENDING |
| 249 | 375 | `query_as` | SELECT | `releases` | P1 | persist results: query releases | Entity::find().filter(...).one()/all() | PENDING |
| 250 | 385 | `query` | UPDATE | `releases` | P1 | persist results: update releases | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 251 | 414 | `query` | UPDATE | `releases` | P1 | persist results: update releases | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 252 | 436 | `query` | INSERT | `releases` | P1 | persist results: insert into releases | ActiveModel insert + OnConflict / Statement if complex | PENDING |
| 253 | 471 | `query` | UPDATE | `releases` | P1 | persist results: update releases | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 254 | 748 | `query_as` | SELECT | `releases` | P1 | cleanup removed: query releases | Entity::find().filter(...).one()/all() | PENDING |
| 255 | 761 | `query` | UPDATE | `releases` | P1 | cleanup removed: update releases | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 256 | 768 | `query` | DELETE | `releases` | P1 | cleanup removed: delete from releases | Entity::delete_many().filter(...) | PENDING |

### `crates/service/src/workers/build_preparation.rs` (1)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 257 | 59 | `query` | INSERT | `device_action_queue` | P1 | enqueue official build: insert into device_action_queue | ActiveModel::insert / Entity::insert | PENDING |

### `crates/service/src/workers/device_action.rs` (1)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 258 | 572 | `query_as` | SELECT | `device_controllers`, `devices`, `relay_channels`, `relays` | P0 | resolve gen4 ipl target: query device_controllers, devices, relay_channels, relays | Entity::find + app-side JSON, or Statement for heavy aggregation | PENDING |

### `crates/service/src/workers/device_action_watchdog.rs` (5)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 259 | 39 | `query_as` | SELECT | `device_action_queue`, `releases`, `test_executions` | P0 | run cycle: claim rows with FOR UPDATE SKIP LOCKED (device_action_queue, releases, test_executions) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 260 | 69 | `query` | UPDATE | `device_action_queue` | P0 | run cycle: update device_action_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 261 | 129 | `query` | UPDATE | `test_executions` | P0 | fail test action: update test_executions | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 262 | 161 | `query_scalar` | SELECT | `releases` | P0 | fail flash action: query releases | Entity::find().filter(...).one()/all() | PENDING |
| 263 | 174 | `query` | UPDATE | `releases` | P0 | fail flash action: update releases | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/workers/device_controller.rs` (3)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 264 | 43 | `query_scalar` | SELECT | `device_controller_heartbeats`, `device_controllers` | P0 | run controller heartbeat timeout cycle: claim rows with FOR UPDATE SKIP LOCKED (device_controller_heartbeats, device_controllers) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 265 | 75 | `query` | UPDATE | `device_controllers` | P0 | run controller heartbeat timeout cycle: update device_controllers | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 266 | 91 | `query_scalar` | INSERT | `alerts` | P0 | run controller heartbeat timeout cycle: insert into alerts returning JSON | ActiveModel::insert then into_json / Entity::insert | PENDING |

### `crates/service/src/workers/device_heartbeat.rs` (5)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 267 | 37 | `query_as` | SELECT | `device_action_queue`, `devices`, `fallback_updates`, `heartbeats` | P0 | run cycle: claim rows with FOR UPDATE SKIP LOCKED (device_action_queue, devices, fallback_updates, heartbeats) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 268 | 102 | `query` | UPDATE | `devices` | P0 | run cycle: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 269 | 120 | `query` | INSERT | `device_state_change` | P0 | run cycle: insert into device_state_change | ActiveModel::insert / Entity::insert | PENDING |
| 270 | 129 | `query` | INSERT | `heartbeats` | P0 | run cycle: insert into heartbeats | ActiveModel::insert / Entity::insert | PENDING |
| 271 | 163 | `query_scalar` | INSERT | `alerts` | P0 | run cycle: insert into alerts returning JSON | ActiveModel::insert then into_json / Entity::insert | PENDING |

### `crates/service/src/workers/fallback_flash.rs` (6)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 272 | 63 | `query_as` | SELECT | `devices`, `fallback_updates`, `releases` | P0 | claim next: claim rows with FOR UPDATE SKIP LOCKED (devices, fallback_updates, releases) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 273 | 83 | `query` | UPDATE | `fallback_updates` | P0 | claim next: update fallback_updates | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 274 | 90 | `query` | UPDATE | `test_executions` | P0 | claim next: update test_executions | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 275 | 140 | `query` | UPDATE | `fallback_updates` | P0 | fail: update fallback_updates | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 276 | 147 | `query` | UPDATE | `test_executions` | P0 | fail: update test_executions | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 277 | 155 | `query` | INSERT | `log_entries` | P0 | fail: insert into log_entries | ActiveModel::insert / Entity::insert | PENDING |

### `crates/service/src/workers/gen5_mapping.rs` (10)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 278 | 68 | `query_as` | SELECT | `gen5_mapping_queue` | P0 | claim next: claim rows with FOR UPDATE SKIP LOCKED (gen5_mapping_queue) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 279 | 88 | `query` | UPDATE | `gen5_mapping_queue` | P0 | claim next: update gen5_mapping_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 280 | 101 | `query` | UPDATE | `gen5_mapping_queue` | P0 | claim next: update gen5_mapping_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 281 | 121 | `query_scalar` | SELECT | `device_controllers` | P0 | process queued: query device_controllers | Entity::find().filter(...).one()/all() | PENDING |
| 282 | 145 | `query` | UPDATE | `gen5_mapping_queue` | P0 | process queued: update gen5_mapping_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 283 | 170 | `query` | UPDATE | `gen5_mapping_queue` | P0 | update initial failure: update gen5_mapping_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 284 | 211 | `query` | UPDATE | `gen5_mapping_queue` | P0 | process failure poll: update gen5_mapping_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 285 | 242 | `query` | UPDATE | `gen5_mapping_queue` | P0 | schedule poll: update gen5_mapping_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 286 | 270 | `query` | UPDATE | `gen5_mapping_queue` | P0 | cancel and delete device: update gen5_mapping_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 287 | 283 | `query` | DELETE | `devices` | P0 | cancel and delete device: delete from devices | Entity::delete_many().filter(...) | PENDING |

### `crates/service/src/workers/gen5_reboot_watchdog.rs` (12)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 288 | 40 | `query_scalar` | SELECT | `device_action_queue`, `devices`, `fallback_updates` | P0 | run cycle: existence check on device_action_queue, devices, fallback_updates | Entity::find().find_also_related / join / select_only | PENDING |
| 289 | 105 | `query` | DELETE | `device_action_queue`, `devices`, `fallback_updates`, `gen5_reboot_attempts` | P0 | cleanup recovered: delete from device_action_queue, devices, fallback_updates, gen5_reboot_attempts | Entity::delete_many().filter(...) (add entity for gen5_reboot_attempts first) | PENDING |
| 290 | 145 | `query_as` | SELECT | `device_action_queue`, `device_controllers`, `devices`, `fallback_updates`, `gen5_reboot_attempts` | P0 | process device: claim rows with FOR UPDATE SKIP LOCKED (device_action_queue, device_controllers, devices, fallback_updates, gen5_reboot_attempts) | Statement (SKIP LOCKED / claim pattern often irreducible) (add entity for gen5_reboot_attempts first) | PENDING |
| 291 | 199 | `query` | INSERT | `gen5_reboot_attempts` | P0 | process device: insert into gen5_reboot_attempts | ActiveModel insert + OnConflict / Statement if complex (add entity for gen5_reboot_attempts first) | PENDING |
| 292 | 249 | `query_as` | SELECT | `device_action_queue`, `fallback_updates`, `test_executions` | P0 | fail stuck device: query device_action_queue, fallback_updates, test_executions | Entity::find().find_also_related / join / select_only | PENDING |
| 293 | 267 | `query` | UPDATE | `device_action_queue` | P0 | fail stuck device: update device_action_queue | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 294 | 275 | `query` | UPDATE | `fallback_updates` | P0 | fail stuck device: update fallback_updates | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 295 | 284 | `query` | UPDATE | `test_executions` | P0 | fail stuck device: update test_executions | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 296 | 297 | `query` | INSERT | `log_entries` | P0 | fail stuck device: insert into log_entries | ActiveModel::insert / Entity::insert | PENDING |
| 297 | 309 | `query` | UPDATE | `devices` | P0 | fail stuck device: update devices | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |
| 298 | 319 | `query` | INSERT | `device_state_change` | P0 | fail stuck device: insert into device_state_change | ActiveModel::insert / Entity::insert | PENDING |
| 299 | 328 | `query` | DELETE | `gen5_reboot_attempts` | P0 | fail stuck device: delete from gen5_reboot_attempts | Entity::delete_many().filter(...) (add entity for gen5_reboot_attempts first) | PENDING |

### `crates/service/src/workers/ipl_distribution.rs` (2)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 300 | 118 | `query_scalar` | SELECT | `device_controllers` | P1 | prepare and distribute: query device_controllers | Entity::find().filter(...).one()/all() | PENDING |
| 301 | 432 | `query_scalar` | SELECT | `device_controllers` | P1 | active controllers: query device_controllers | Entity::find().filter(...).one()/all() | PENDING |

### `crates/service/src/workers/relay_service.rs` (2)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 302 | 255 | `query_as` | SELECT | `pending_relay_configs`, `relay_channels`, `relays` | P0 | run relay configuration timeout cycle: claim rows with FOR UPDATE SKIP LOCKED (pending_relay_configs, relay_channels, relays) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 303 | 292 | `query` | UPDATE | `pending_relay_configs` | P0 | run relay configuration timeout cycle: update pending_relay_configs | Entity::update_many().col_expr(...).filter(...) or ActiveModel | PENDING |

### `crates/service/src/workers/test_preparation.rs` (2)

| # | Line | API | Verb | Tables | Priority | Purpose | Suggested SeaORM | Status |
|--:|-----:|-----|------|--------|----------|---------|------------------|--------|
| 304 | 83 | `query_as` | WITH/SELECT | `releases`, `test_executions` | P0 | claim: claim rows with FOR UPDATE SKIP LOCKED (releases, test_executions) | Statement (SKIP LOCKED / claim pattern often irreducible) | PENDING |
| 305 | 124 | `query_as` | SELECT | `testcase` | P0 | prepare: query testcase | Entity::find().filter(...).one()/all() | PENDING |

## 5. Notes

- Scanned for `sqlx::query`, `sqlx::query_as`, and `sqlx::query_scalar` (including turbofish forms).
- HTTP `.query()` builders in integrations/core are **not** included.
- One commented-out call in `relay_configuration_store/apply.rs` was excluded.
- If/else SQL string branches count as **one** call site (representative branch documented).
- `persistence::clean_retention_batch` now runs allowlisted DELETE SQL through SeaORM `Statement` (not sqlx).
- Entity table name quirks already in tree: `testcase` (not `test_cases`), `device_action_queue`, `device_state_change`, `gen5_mapping_queue` (singular).

## 6. Verification

```bash
cd farm-controller
rg 'sqlx::query' --glob '*.rs'   # expect: comment-only hit in orm_util.rs
cargo check -p farmcontroller    # expect: Finished
```
