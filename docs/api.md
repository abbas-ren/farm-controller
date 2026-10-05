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

Most application APIs retain `/api/v1/auth` and `/api/v1/device`. Use `Authorization: Bearer <token>` unless a route is explicitly documented as a deployed device/controller callback. Refresh compatibility accepts `refreshtoken`, `x-refresh-token`, and the `refreshToken` cookie.

The detailed path, access, payload, historical error, file, upload, and event inventory is [api-compatibility.md](api-compatibility.md). Swagger is the executable reference for mounted routes; the compatibility inventory records legacy behavior and remaining external validation.

Multipart official/custom build uploads and TUS create, inspect, and append operations are described in OpenAPI, including required protocol headers and content types.

Socket.IO uses its own event protocol rather than HTTP operations: clients join and leave `build`, `test`, `dashboard`, `device`, and `device-controller` rooms with the corresponding `join:*` and `leave:*` events. Native `/ws` modes and query parameters are documented in OpenAPI.