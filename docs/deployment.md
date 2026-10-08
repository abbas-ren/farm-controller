# Deployment

## systemd

1. Build with `cargo build --locked --release`.
2. Install the binary at `/usr/local/bin/farmcontroller`.
3. Create user/group `farmcontroller` and writable directories listed in the unit.
4. Install `config/production.toml.example` as `/etc/farmcontroller/config.toml`.
5. Install protected environment values from `deploy/systemd/farmcontroller.env.example` as `/etc/farmcontroller/farmcontroller.env` with mode `0600`.
6. Install `deploy/systemd/farmcontroller.service`, then run `systemctl daemon-reload && systemctl enable --now farmcontroller`.
7. Verify `/health`, `/ready`, and `/metrics` before directing traffic to the process.

The packaged unit creates `/var/lib/farmcontroller` for the mode-`0600` admin control document and uses `Restart=always`. This is required for the Control Center restart action: FarmController returns `202`, terminates itself with SIGTERM, and the supervisor starts it with the persisted staged configuration. EdgeController restart remains allowlisted to `dev-con.service`.

SIGTERM stops intake through Axum graceful shutdown, cancels workers, joins their tasks, and lets nonblocking tracing guards flush during process teardown.

## Frontend and edge routing

The Rust process is the only backend application server; it does not embed frontend assets. Serve the prebuilt frontend from an edge Nginx instance, object storage, or a CDN, with no Node.js runtime. Route `/api/v1`, `/socket.io`, and `/ws` unchanged to the unified listener. Do not strip the `/api/v1/auth` or `/api/v1/device` prefix and do not add the legacy `X-Forwarded-Prefix` rewriting behavior.

`deploy/nginx/farmcontroller.conf.example` is the unified-runtime Nginx example. It disables request buffering for streaming multipart and TUS traffic, preserves WebSocket upgrades, and permits the configured 500 MiB build limit. TUS uploads use 5 MiB frontend chunks and reach Axum directly, so each PATCH remains within the default 10 MiB request-body limit while the completed upload is bounded separately by `device.build_upload_max_bytes`. The root `nginx/nginx.conf` remains a legacy rollback asset for the separate Node services.

## Containers

`Dockerfile.rust` builds only the unified binary. `docker-compose.rust.yml` provides FarmController plus PostgreSQL, Keycloak, and TFTP external dependencies without starting the old Node/Python backend services.

```bash
docker compose -f docker-compose.rust.yml build
docker compose -f docker-compose.rust.yml up -d
docker compose -f docker-compose.rust.yml ps
```

Supply all referenced variables through a protected `.env` or deployment secret manager. The NFS, artifact, and test-result host mounts must already exist with UID/GID 10001 access. Production Keycloak must use real admin/client secrets and an appropriate hostname/TLS configuration.

Keep the legacy stack available until the compatibility checks in [migration.md](migration.md) pass. Roll back by restoring traffic to the old gateway; additive Rust control-plane migrations do not remove legacy domain data.