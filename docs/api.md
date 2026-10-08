# API

The service generates OpenAPI from Rust route definitions:

- OpenAPI JSON: `/openapi.json`
- Swagger redirect: `/swagger`
- Swagger UI: `/swagger-ui/`
- Health: `/health`
- Readiness: `/ready`
- Prometheus: `/metrics`
- Browser Socket.IO: `/socket.io`
- Native protocols: `/ws`
- Runtime logs: `/api/v1/operations/logs`
- Runtime log level: `/api/v1/operations/logs/level`
- Controller administration: `GET`, `PATCH`, and `POST /api/v1/admin/control`

Most application APIs retain `/api/v1/auth` and `/api/v1/device`. Use `Authorization: Bearer <token>` unless a route is explicitly documented as a deployed device/controller callback. Refresh compatibility accepts `refreshtoken`, `x-refresh-token`, and the `refreshToken` cookie.

The administrator-only runtime log endpoints accept `farmcontroller` or `edgecontroller` as the source. Edge requests require a stored controller ID and are proxied by FarmController; browsers never connect to EdgeController directly.

The administrator-only control resource uses the same `source` and optional `controllerId` query parameters. `GET` returns active and staged settings, capabilities, runtime state, and restart status. `PATCH` validates and persists typed settings; secrets are write-only and represented only by configured/not-configured status. `POST` accepts allowlisted maintenance and restart actions. FarmController resolves and proxies approved EdgeController addresses, and stores each enabled Edge API token in its mode-`0600` control document so browsers never receive or send it directly.

Native `/ws?client=frontend&adminTerminal=true` requires the admin role. In that mode, `farmcontroller` resolves to loopback and approved EdgeController IP addresses remain database-allowlisted. Usernames and credentials always come from FarmController configuration rather than browser messages.

The detailed path, access, payload, historical error, file, upload, and event inventory is [api-compatibility.md](api-compatibility.md). Swagger is the executable reference for mounted routes; the compatibility inventory records legacy behavior and remaining external validation.

Multipart official/custom build uploads and TUS create, inspect, and append operations are described in OpenAPI, including required protocol headers and content types.

Socket.IO uses its own event protocol rather than HTTP operations: clients join and leave `build`, `test`, `dashboard`, `device`, and `device-controller` rooms with the corresponding `join:*` and `leave:*` events. Native `/ws` modes and query parameters are documented in OpenAPI.