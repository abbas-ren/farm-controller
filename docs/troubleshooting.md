# Troubleshooting

## Service is not ready

Check `/ready`, structured logs, PostgreSQL reachability, module dependencies, database URL, and Keycloak configuration. `/health` succeeding does not prove dependencies are ready.

## Controllers or devices disconnect

Confirm `/ws` reaches FarmController without a proxy rewriting upgrade headers or query parameters. Controller sessions use `web-cli-protocol`; browser Socket.IO uses `/socket.io` and must not be routed to native `/ws` handling.

## Uploads fail

Check configured upload limits, free space, mount ownership, TUS offset headers, ZIP root layout, and archive expansion/path rejection logs. Failed final chunks roll back their persisted offset for retry.

## Tests stall

Inspect action/preparation/fallback rows, device/controller heartbeat state, NFS/TFTP paths, worker metrics, and execution logs. Do not run legacy and Rust dispatch workers simultaneously.

## Reports or artifacts are missing

Verify `/test-results`, `/artifacts`, `/nfs_share`, and `/tftp_data` mounts are writable by the service user. Validate TestRail/GitLab/Qmetry/Confluence URLs and credentials without printing secrets.

## Shutdown hangs

Allow the systemd `TimeoutStopSec` window. Check for external network calls exceeding configured request timeouts and inspect worker stop logs. Avoid `SIGKILL` unless normal SIGTERM shutdown has failed.