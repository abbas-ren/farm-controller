# SeaORM entities converted from the supplied Sequelize models

These entities target SeaORM 2.x and PostgreSQL.

## Important schema assumptions

1. The original Sequelize models use camelCase DB columns unless `underscored: true` is set. Therefore most entities use `#[sea_orm(rename_all = "camelCase")]`; `Alert` uses snake_case.
2. Sequelize automatically adds an `id` integer primary key to `DeviceStateChange` because that model did not explicitly define a primary key. The Rust entity therefore includes `id`.
3. Sequelize `DataTypes.ENUM` on PostgreSQL creates native PostgreSQL enum types. The enum names below follow Sequelize's conventional `enum_<table>_<column>` naming. If your migrations renamed any enum type, update the `enum_name` attribute in the corresponding Rust enum.
4. `DataTypes.JSONB` is represented by `serde_json::Value`; structured JSON payloads are given typed Rust structs where the supplied code defined a stable shape.
5. Associations are reproduced only where `dbRelations.ts` explicitly declared them. No new relationships are invented for tables that merely happen to share a column name.
6. The supplied code has several Sequelize-level defaults and hooks. Those are not automatically reproduced by SeaORM entity definitions. Database defaults should live in migrations where possible; application-level behavior such as `Alert::mark_as_read` belongs in a service/ActiveModel hook.
7. The original Sequelize code has no explicit relation for `Alert`, `DeviceControllerHeartbeat`, `DeviceControllerMetrics`, `Heartbeat`, `FallbackUpdate`, `Gen5MappingQueue`, `PendingRelayConfig`, `UploadBuild`, `LogEntry`, or `DeviceStateChange` beyond the associations explicitly present in `dbRelations.ts`.

## Suggested usage

```rust
use sea_orm::{Database, EntityTrait, QueryFilter, ColumnTrait};
use db_entities::entity::test_execution;

let db = Database::connect(database_url).await?;

let execution = test_execution::Entity::find()
    .filter(test_execution::Column::TestId.eq(test_id))
    .one(&db)
    .await?;
```

For relations, use `find_with_related`, `find_also_related`, `Related`, and SeaORM 2.0's generated relationship support.

## Why this version uses the compact entity format

The supplied Sequelize associations use several non-primary-key joins (`deviceId` -> `deviceId`, `testId` -> `testId`). I kept the explicit SeaORM relation definitions in this first conversion because they make those joins unambiguous and are backward-compatible with SeaORM 2.0. SeaORM 2.0 also supports the dense `#[sea_orm::model]` format and its Entity Loader, which is the next layer I would add once the actual PostgreSQL schema has been inspected.
