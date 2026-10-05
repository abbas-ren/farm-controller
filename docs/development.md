# Development

Requirements are Rust 1.98.1, a C toolchain/CMake, and PostgreSQL for live repository tests. Keycloak and external test/report systems are needed only for their integration paths.

```bash
cargo build
cargo run -- --config config/default.toml
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Database migration tests require a disposable database whose name ends in `_test`:

```bash
TEST_DATABASE_URL=postgresql://user:password@localhost/farmcontroller_test cargo test --test database_migrations -- --ignored
```

Do not point that variable at a production or shared database. Use the mock-backed unit tests for Keycloak, TestRail, Qmetry, and Confluence during normal development. Preserve event publication after commit, bounded workers, cancellation ownership, static parameterized SQL, and traversal-safe file paths when adding behavior.