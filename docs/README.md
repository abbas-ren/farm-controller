# FarmController Documentation

## Architecture and implementation

- [Architecture](architecture.md): crates, dependency direction, runtime flow,
  boundaries, persistence, errors, and observability.
- [Architecture decisions](architecture-decisions.md): accepted decisions,
  compatibility constraints, and intentional debt.
- [Rearchitecture report](rearchitecture.md): before/after structure, completed
  work, validation, limitations, and recommended improvements.
- [Development](development.md): workspace commands, focused tests, and coding
  constraints.

## API and compatibility

- [API](api.md): endpoint and protocol documentation.
- [API compatibility](api-compatibility.md): legacy wire contracts and source of
  truth.
- [Compatibility](compatibility.md): broader consumer compatibility policy.
- [Migration matrix](migration-matrix.md): implementation and verification ledger.

## Configuration and operations

- [Configuration](configuration.md): TOML, environment, CLI, defaults, and
  validation.
- [Deployment](deployment.md): Linux/container deployment and rollback.
- [Observability](observability.md): tracing, metrics, dashboards, and safe labels.
- [Troubleshooting](troubleshooting.md): operational diagnosis.

## Database and migration history

- [Database migration](database-migration.md): schema safety and preflight rules.
- [Migration](migration.md): cutover and rollback workflow.
- [Rust migration progress](rust-migration-progress.md): detailed historical
  implementation ledger.

Historical migration documents remain useful evidence. When they conflict with
current build or source paths, the root README, architecture document, Cargo
manifests, and generated OpenAPI are authoritative.