# Configuration

Precedence is defaults, TOML file, environment, then CLI. Use `--config <path>`, `FARMCONTROLLER__SECTION__FIELD`, and the CLI overrides shown by `farmcontroller --help`.

`server.cors_allowed_origins` accepts exact HTTP(S) browser origins without paths. An empty list preserves the legacy permissive CORS behavior; production deployments should set a comma-separated `FARMCONTROLLER__SERVER__CORS_ALLOWED_ORIGINS` allowlist such as `https://farm.example`. Responses include `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, and `Referrer-Policy: strict-origin-when-cross-origin`.

Outbound HTTP clients share bounded connect/request timeouts and never follow redirects, preventing configured or device endpoints from redirecting requests to a different origin. Configured service URLs must use HTTP(S) and cannot embed credentials. The common client does not retry automatically; only the retained device flash/dispatch workflows perform their explicit bounded three-attempt command retry.

Production starts from `config/production.toml.example`. Keep secrets out of TOML and supply at least:

```env
FARMCONTROLLER__DATABASE__URL=postgresql://user:password@db:5432/farmcontroller
FARMCONTROLLER__AUTH__CLIENT_SECRET=replace-me
FARMCONTROLLER__DEVICE__SERVER_IP=192.0.2.10
# Required only when EdgeController uses DEV_CONTROLLER_TOKEN.
EDGE_CONTROLLER_TOKEN=replace-with-the-edge-token
```

Runtime modules are `api`, `auth`, `device`, `reports`, `workers`, `events`, `metrics`, `swagger`, and `health`. Repeated and comma-separated forms are accepted:

```bash
farmcontroller --enable api,device --enable workers --disable swagger
```

Enabling a domain automatically requires its declared dependencies. Startup rejects missing database URLs, auth secrets, invalid sizes, zero timeouts, and incompatible module selections.

Filesystem paths must be writable by the service account: NFS test staging, TFTP output, artifacts, IPL storage, test results, uploads, temporary artifacts, faulty reports, and an optional log file. External integration credentials for TestRail, Qmetry, GitLab, and Confluence use the corresponding `[tests]` and `[reports]` keys or environment overlays.

Gen4/Gen5 build uploads store validated `ipl/` payloads below `device.ipl_artifacts_dir`. Gen5 additionally requires `device.gen5_ipl_script`. Configure family-scoped `gen4_ipl_*` or `gen5_ipl_*` username, password, port, and absolute remote path fields to enable native SFTP distribution to approved active controllers. Empty usernames disable remote distribution for that family while retaining local payload storage. Transfers use `terminal_known_hosts_file` and `terminal_accept_unknown_host_keys`; production should reject unknown keys and provision the known-hosts file.

Jira is optional. To enable failed-test defects, configure `tests.jira_base_url`, `tests.jira_project_key`, and `tests.jira_api_token` together; the token belongs in a protected environment file or secret manager. `jira_endpoint`, `jira_timeout_seconds`, and `jira_script_base_url` control API v2 routing, request bounds, and script links.