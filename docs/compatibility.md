# Compatibility

FarmController preserves public gateway paths, frontend payloads, EdgeController callbacks, browser Socket.IO rooms/events, native `/ws` protocol isolation, legacy PostgreSQL names, refresh-token inputs, upload/file semantics, and selected historical error behavior.

The authoritative route-by-route record is [api-compatibility.md](api-compatibility.md). Focused Rust tests encode many hardcoded frontend and EdgeController expectations. Live PostgreSQL, Keycloak, external catalog/report systems, frontend, EdgeController, and racer E2E runs are still required where marked.

Any deliberate compatibility change must update route tests, generated OpenAPI, the inventory, migration notes, and rollback impact. Do not normalize a legacy quirk solely for aesthetics while active consumers depend on it.