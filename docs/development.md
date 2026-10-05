# Development

Requirements are Rust 1.98.1, a C toolchain/CMake, and PostgreSQL for live repository tests. Keycloak and external test/report systems are needed only for their integration paths.

```bash
cargo build --workspace
cargo run -p farmcontroller-app --bin farmcontroller -- --config config/default.toml
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Run a focused crate while iterating:

```bash
cargo test -p farmcontroller-core
cargo test -p farmcontroller-integrations
cargo test -p farmcontroller
```

Database migration tests require a disposable database whose name ends in `_test`:

```bash
TEST_DATABASE_URL=postgresql://user:password@localhost/farmcontroller_test \
	cargo test -p farmcontroller persistence::tests -- --ignored
```

Do not point that variable at a production or shared database. Use the mock-backed unit tests for Keycloak, TestRail, Qmetry, and Confluence during normal development. Preserve event publication after commit, bounded workers, cancellation ownership, static parameterized SQL, and traversal-safe file paths when adding behavior.

Crate dependency direction is enforced by manifests: `core` has no internal
dependencies, `integrations` depends on `core`, `service` depends on both, and
`app` depends on `service`. Do not add a dependency from a lower crate back to
`service` or `app`; move the owned abstraction downward instead.