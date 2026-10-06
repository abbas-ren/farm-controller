# API Compatibility Inventory

Last reconciled against route source: 2026-10-06. Detailed request/response/error contracts remain the Step 2 stage gate tracked in `docs/rust-migration-progress.md`.

Public gateway paths are shown below. Auth service routes mount below `/api/v1/auth`; device service routes mount below `/api/v1/device`. Health, metrics, OpenAPI, and Swagger will also have unprefixed routes on the same Rust listener.

Legend: `public` means the current route has no auth middleware, `user` means an authenticated request, and `admin` means the current admin authorization middleware applies.

## Current checkout status

Source of truth for mounted Rust device routes is `crates/service/src/devices/routes.rs`; auth routes are owned by `crates/service/src/auth/handlers.rs`; report routes are owned by `crates/service/src/reports/http.rs` and merged when the Reports module is enabled; `/ws`, `/socket.io`, and `/api/v1/device/ws/send/message` are owned by `crates/service/src/events/mod.rs`; operational routes are in `crates/service/src/api/mod.rs`.

The current checkout is buildable. Retained relay, analytics, notification, and faulty-report route groups are mounted, registered in OpenAPI, and have focused route-contract coverage.

Current validation baseline: format check, all-target/all-feature check, strict Clippy, and all 148 local Rust tests pass through consumer compatibility, IPL lifecycle, generated-contract, system-metrics, pending-user event, HTTP security-policy, direct gateway/TUS routing, and mounted auth route work. Live PostgreSQL migration and repository validation remains pending and is not represented by this local total.

## Source-verified API summary

Implemented or substantially implemented and mounted:

- Auth route set in the table below using the production `KeycloakIdentityProvider` overrides.
- Core device/controller inventory, registration, callbacks, heartbeat, topology, actions, artifact configuration/copy, reboot/power, CSV export, and relay routes.
- Build list/detail/filters, upload initialization, TUS transport, custom/official multipart ingestion, flag, and delete database/local-filesystem core.
- TestRail/Qmetry catalog reads; execution list/detail/latest/single/device/progress/cases/log/results/build/cancel reads/mutations; execution report metadata/HTML/generation/upload bridge.
- All fourteen authenticated device, execution, build, usage, and test analytics routes.
- All five notification list/read routes.
- All seven faulty-report create/list/detail/download/status/delete routes.
- Native report queue/status/Confluence routes `/report`, `/report/{job_id}`, `/upload-confluence` when Reports is enabled.
- EdgeController heartbeat WebSocket `/ws`, browser Socket.IO polling/WebSocket transport at `/socket.io`, public `user` event dispatch, and operational `/health`, `/ready`, `/metrics`, `/openapi.json`, `/swagger-ui/`.

Environment-dependent or incomplete verification:

- No inventoried device HTTP route remains unmounted. Native controller, device-heartbeat, testcase-result, authenticated frontend SSH/RTOS, and authenticated WebCLI protocols are ported.
- The unified `/health` replaces the old reports-process health endpoint because reports no longer run as a separate service.
- Upload/scanner IPL storage and distribution, official queue execution, registration backfill, and local/remote deletion are implemented. Live PostgreSQL, controller, and external-system verification remain.

## Authentication and users

| Method | Existing path | Access | Known consumer or integration | Rust status |
|---|---|---|---|---|
| POST | `/api/v1/auth/signin` | public | frontend login | ported and Keycloak-tested |
| POST | `/api/v1/auth/register` | admin | frontend user administration | ported; Keycloak mock-tested |
| POST | `/api/v1/auth/register/request` | public | frontend registration | ported; Keycloak mock-tested |
| POST | `/api/v1/auth/register/action` | admin | user approval flow | ported; Keycloak mock-tested |
| POST | `/api/v1/auth/forgot-password` | public | frontend login | ported; Keycloak mock-tested |
| POST | `/api/v1/auth/reset-password` | public | frontend login | ported; route registered, contract coverage incomplete |
| POST | `/api/v1/auth/user-registration/confirmation/send-mail` | public | Keycloak event-listener provider | ported; live provider validation pending |
| POST | `/api/v1/auth/verify-user` | public | frontend token/user check | ported; route registered, contract coverage incomplete |
| GET | `/api/v1/auth/validate` | user | device-service legacy internal auth call | ported; reusable provider replaces internal hop |
| GET | `/api/v1/auth/validate/admin` | admin | device-service legacy internal auth call | ported; realm/client role tested |
| GET | `/api/v1/auth/validate/user` | user role | device-service legacy internal auth call | ported; realm/client role tested |
| GET | `/api/v1/auth/user/:id` | public | frontend/user lookup | ported |
| GET | `/api/v1/auth/users` | admin | frontend user administration | ported |
| GET | `/api/v1/auth/users/active` | admin | frontend user administration | ported |
| GET | `/api/v1/auth/users/requests` | admin | frontend approvals | ported; Keycloak mock-tested |
| DELETE | `/api/v1/auth/user/:userId` | admin | frontend user administration | ported |

Authentication compatibility includes `Authorization: Bearer ...`, both observed refresh headers (`refreshtoken` and `x-refresh-token`), and the `refreshToken` cookie. The frontend treats unexpected `401` responses as logout signals.

## Devices and controllers

| Method | Existing path | Access | Known consumer or integration | Rust status |
|---|---|---|---|---|
| POST | `/api/v1/device/` | public | device registration | ported; validation and route-contract tested, live PostgreSQL test blocked |
| POST | `/api/v1/device/controller` | public | EdgeController registration (sends trailing slash) | ported and fixture-tested |
| POST | `/api/v1/device/mapping-gen5` | public | EdgeController mapping callback | ported and fixture-tested |
| GET | `/api/v1/device/flash-confirm` | public | EdgeController Gen5 callback | ported and fixture-tested |
| GET | `/api/v1/device/flash-confirm-gen4` | public | EdgeController Gen4 callback | ported and fixture-tested |
| GET | `/api/v1/device/` | user | frontend device list | ported; focused frontend-shape test |
| GET | `/api/v1/device/controller` | user | frontend configuration | ported |
| GET | `/api/v1/device/user` | user | frontend user devices | ported |
| GET | `/api/v1/device/all/active` | public | device integrations | ported |
| GET | `/api/v1/device/topology` | user | frontend topology | ported; focused frontend-shape test |
| GET | `/api/v1/device/export` | admin | frontend CSV export | ported; formatter and HTTP download contract tested, live PostgreSQL test blocked |
| GET | `/api/v1/device/families` | user | frontend filters | ported and frontend-shape tested |
| GET | `/api/v1/device/deviceTypes` | user | frontend filters | ported and frontend-shape tested |
| GET | `/api/v1/device/builds` | user | frontend build selection | ported |
| PUT | `/api/v1/device/config/artifacts` | admin | frontend configuration | ported |
| PUT | `/api/v1/device/config/artifacts/default` | admin | frontend configuration | ported; safe archive/parser tests |
| GET | `/api/v1/device/:id` | user | frontend device detail | ported and frontend-shape tested |
| GET | `/api/v1/device/:id/heartbeat` | user | frontend heartbeat view | ported and frontend-shape tested |
| GET | `/api/v1/device/:id/builds` | user | frontend build selection | ported |
| GET | `/api/v1/device/:id/flashing` | public | EdgeController callback | ported; route/version tests, live PostgreSQL test blocked |
| GET | `/api/v1/device/:id/test-completed` | public | EdgeController callback | ported active contract including best-effort RTOS log side effect |
| GET | `/api/v1/device/:id/reboot` | public | device flow | ported |
| PUT | `/api/v1/device/:id/action` | admin | frontend device control | ported |
| PUT | `/api/v1/device/heartbeat/timeout` | admin | frontend configuration | ported |
| PUT | `/api/v1/device/controller/:id` | admin | frontend configuration | ported |
| DELETE | `/api/v1/device/controller/:id` | admin | frontend configuration | ported |
| DELETE | `/api/v1/device/:id` | admin | frontend configuration | ported |

### Device registration contract

`POST /api/v1/device/` is public because deployed devices register without a bearer token. The Rust router accepts the exact trailing-slash path and the non-trailing nested-root form.

- Request: required string fields `macAddress`, `ipAddress`, and non-empty `deviceName`; optional `deviceFamily`, `deviceType`, `buildId`, numeric integer `timeout`, `interfaces` as an object of string arrays, `softwareVersion`, `nfsPath`, `state`, and `status`.
- Defaults: `timeout = 5`, `state = free`, and `status = requested`. `controllerId` is reset to the empty string as in the legacy transform.
- Validation: MAC must contain six two-digit hexadecimal octets separated by `:` or `-`; IPv4 only; state/status must be legacy enum values. Invalid fields and malformed/missing JSON fields return `400`.
- Identity: lower-case MAC with separators removed becomes `deviceId`.
- Success: always `201 Created` with `{ "success": true, "data": Device }`; `data.interfaces` contains flattened `{deviceId,type,interfaceId,...}` rows.
- Persistence: one transaction creates a new row, updates an active duplicate, or restores a soft-deleted duplicate; replaces the complete interface set; records `unknown` state on restore; and auto-creates a missing device-type artifact folder. Requested devices are auto-approved after capturing the legacy response representation.
- Duplicate semantics: normalized MAC is the identity, so repeated posts update rather than create duplicate rows. Active updates preserve the current device state, matching the heartbeat ownership boundary.
- Registration persists the legacy new-device admin alert and emits `device-addition`/`device-approval` alert envelopes. Best-effort Gen3 controller mapping remains deferred to the Step 6.5 worker boundary.
- Tests: pure MAC/IPv4/interface tests and anonymous HTTP success/malformed-input tests run without infrastructure. Create/update/restore/rollback behavior still needs `TEST_DATABASE_URL` against a legacy-shaped PostgreSQL schema.

### Device CSV export contract

`GET /api/v1/device/export` requires the admin role and is consumed by the frontend as a blob download.

- Query: optional `fromDate`, `toDate`, `search`, and `status`. Search trims whitespace and case-insensitively matches device name, ID, type, or MAC. Date bounds apply inclusively to `createdAt`. The legacy implementation accepts but does not apply `status`; Rust preserves that behavior.
- Selection: excludes soft-deleted devices, includes interfaces, and orders devices by `createdAt DESC`.
- Success: `200 OK`, `Content-Type: text/csv`, and `Content-Disposition: attachment; filename="devices-<ISO timestamp>.csv"`.
- Columns: fixed fields `deviceId`, `deviceName`, `deviceType`, `macAddress`, `ipAddress`, `status`, `state`, `createdAt`, `updatedAt`, `stateUpdatedAt`, `totalInterfaces`, and `lastConnectedOn`, followed by one `<type>_interfaces` column per interface type found in the result. Multiple IDs are joined with `, ` and standards-compliant CSV quoting is applied.
- Dates: legacy `Date.toDateString()` shape, for example `Fri Oct 02 2026`; absent values are empty fields.
- Errors: missing/invalid authorization returns `401`/`403`; database or formatting failure returns `500`.
- Tests: pure dynamic-header/date/quoting coverage and HTTP authorization/content-header/body coverage. Search/date query execution still needs `TEST_DATABASE_URL` against a legacy-shaped PostgreSQL schema.

### Device flashing callback contract

`GET /api/v1/device/:id/flashing` is a public device callback where `id` is the normalized device ID.

- Missing device: `404` with `Device not found`.
- Non-busy device: no flag mutation; still returns the legacy success response.
- Busy device: reads the newest `waiting` or `running` action queue job. A test/non-flash job or no job sets `flashing=true`. A flash job compares target with installed software after removing leading `v`, suffixes, and insignificant zero segments: newer sets `upgrading=true`, older sets `flashing=true`, equal or missing versions leave both false.
- Success: `200` with `{ "success": true, "message": "Device <id> is now marked as upgrading" }`.
- Transaction: locks the active device, reads the latest queue decision, updates both flags and `updatedAt`, then commits.
- The legacy `device_state_update` emission is ported. Test jobs for Gen4/Gen5 still require the best-effort RTOS-start transport.
- Tests: anonymous HTTP success/not-found and pure version comparison. Queue selection and rollback need `TEST_DATABASE_URL`.

### Test-completed callback contract

`GET /api/v1/device/:id/test-completed?testId=...` is public. `id` is the device ID and `testId` is required.

- Normalization: removes single quotes, double quotes, and all whitespace from `testId` before lookup.
- Missing/empty input: an empty normalized ID returns `400`; an unknown execution returns `404` with `Test execution <id> not found`.
- Terminal execution: `completed`, `cancelled`, and `failed` executions return success without further work.
- Nonterminal execution: the active legacy path does not mutate test or queue status. Gen4/Gen5 devices make a best-effort RTOS-end request, store returned text at `test-results/<deviceType>/<buildVersion>/<testId>/rtosLogFile.txt`, update `rtosLogPath`, and emit `test_execution_update`. Transport or storage failure is logged and swallowed.
- Success: `200` with `{ "success": true, "message": "Test completion handled for device <id>" }`.
- Current Rust scope: execution lookup, terminal no-op, Gen4 relay and Gen5 UART mapping resolution, bounded `/rtos/end`, traversal-safe artifact storage, persisted `rtosLogPath`, and post-persistence owner event are ported. Transport/storage failures remain best-effort.
- Tests: anonymous success with quoted/spaced ID, unknown execution, empty normalized ID, Gen5 payload/UART behavior, artifact containment, and owner event shape. Live execution lookup/persistence needs `TEST_DATABASE_URL`.

The EdgeController registration payload and heartbeat schema are defined by `edgecontroller/src/models.rs`; they must be encoded as exact Rust compatibility fixtures before these rows become `ported`.

## Builds and tests

| Method | Existing path | Access | Notes | Rust status |
|---|---|---|---|---|
| GET | `/api/v1/device/build` | user | list with filters/pagination | ported; auth/shape tested, live PostgreSQL test blocked |
| GET | `/api/v1/device/build/filters` | user | static route must beat dynamic ID matching | ported; auth/shape tested |
| GET | `/api/v1/device/build/:id` | user | detail | ported; auth/success/not-found tested |
| POST | `/api/v1/device/build/upload/init` | user | initializes upload metadata | ported; auth/success/validation tested, live PostgreSQL test blocked |
| `OPTIONS,HEAD,PATCH,POST` | `/api/v1/device/build/upload/tus/*` | user | resumable TUS transport | protocol/storage/ZIP ingestion, transactional lifecycle events, IPL distribution, and workflow dispatch ported; live infrastructure pending |
| POST | `/api/v1/device/build/upload/custom` | user | multipart custom build | streaming/ingestion, lifecycle events, and IPL side effects ported; live infrastructure pending |
| POST | `/api/v1/device/build/upload` | admin | multipart official build | streaming/ingestion, lifecycle events, IPL side effects, and official workflow dispatch ported; live infrastructure pending |
| PUT | `/api/v1/device/build/:id/flag` | admin | official/flag state | ported; auth/input/success/not-found tested |
| DELETE | `/api/v1/device/build/:id` | admin | delete build | ported DB/local/remote IPL cleanup plus committed alerts/events; live controller validation pending |
| GET | `/api/v1/device/test/export` | user | CSV | ported; formatter/auth/download tested, live PostgreSQL test blocked |
| GET | `/api/v1/device/test/plan` | user | TestRail-backed | ported; route and mock-provider pagination/auth/filter tested |
| GET | `/api/v1/device/test/suite` | user | TestRail-backed | ported; validation/route/mock-provider tested |
| GET | `/api/v1/device/test/testcase` | user | TestRail-backed | ported; validation/route/mock-provider/normalization tested |
| GET | `/api/v1/device/test/plan/:planID/testcase` | user | Qmetry-backed legacy route | ported; validation/route/mock-provider tested |
| GET | `/api/v1/device/test/plan/:planID/suite/:suiteID/testcase` | user | Qmetry-backed legacy route | ported; validation/route/mock-provider tested |
| POST | `/api/v1/device/test/execution` | user | create execution | ported through durable preparation, assignment, dispatch, watchdog, and result processing; live systems pending |
| GET | `/api/v1/device/test/execution` | user | list executions | ported; auth/pagination/shape tested, live PostgreSQL blocked |
| GET | `/api/v1/device/test/execution/:id` | user | execution detail; static subpaths must take priority | default/latest/table summary ported and focused-tested |
| GET | `/api/v1/device/test/execution/single/:testId` | user | single execution | ported; static routing/success/not-found tested |
| GET | `/api/v1/device/test/execution/list/:id` | public | testcase IDs for active/nonfailed execution | ported; public shape tested |
| GET | `/api/v1/device/test/execution/device/:id` | user | latest user execution for device | ported; logs/shape tested |
| GET | `/api/v1/device/test/execution/progress/list` | user | in-progress list | ported; user-scoped shape tested |
| GET | `/api/v1/device/test/execution/cases/:id` | user | execution cases | ported; ownership/shape tested, live PostgreSQL blocked |
| GET | `/api/v1/device/test/execution/logs/:id` | user | log download | ported; headers/body tested |
| GET | `/api/v1/device/test/execution/report/:id` | user | report metadata | ported; success/not-found tested |
| GET | `/api/v1/device/test/execution/report/:id/html` | user | report HTML | ported; path safety/content tested |
| PUT | `/api/v1/device/test/execution/report/:id` | user | create/update report | ported via native bounded report queue with durable status synchronization; live database pending |
| PUT | `/api/v1/device/test/execution/report/:id/upload` | user | upload report | ported via native Confluence service; live upstream validation pending |
| GET | `/api/v1/device/test/execution/build/:buildId` | user | build executions | ported; pagination/shape tested, live PostgreSQL blocked |
| PUT | `/api/v1/device/test/execution/:id` | user | update TestRail case result | ported; auth/validation/legacy acknowledgement tested, live TestRail pending |
| GET | `/api/v1/device/test/execution/results/:id` | user | execution results | ported; success/not-found contract tested |
| PUT | `/api/v1/device/test/cancel/:id` | user | cancel execution | persistence, idempotency, queue-aware worker handling, and committed user/build events ported; live worker validation pending |
| GET | `/api/v1/device/test/testcase/:testCaseId` | user | case detail | ported; success/legacy errors tested |
| GET | `/api/v1/device/test/testcase/:testCaseId/log` | user | case log download | ported; root containment/headers/body tested |

### Build-list contract

`GET /api/v1/device/build` requires an authenticated user and returns the frontend-compatible object `{builds,totalCount,currentPage,totalPages,requestedCount}`.

- Filters: exact `deviceType`, `deviceFamily`, `buildVersion`, and `flagged=true|false`; invalid `flagged` values are ignored. `search` case-insensitively matches device type, family, version, or tag.
- Pagination: `page` defaults to 1 and `limit` to 10; both are clamped to at least 1. `requestedCount` is the returned row count and `totalPages` is never below 1, including empty results.
- Sorting: allowlisted `createdAt`, `version`, `deviceFamily`, `deviceType`, or `tag`; unknown/missing columns fall back to `createdAt`. Only `sortOrder=asc` selects ascending; all other values use descending.
- Data: each entry preserves the complete legacy `releases` row JSON needed by `BuildRelease`, including artifacts and operational fields beyond the frontend's minimum type.
- Errors: missing/invalid authentication returns `401`; repository failures return `500`; unavailable persistence returns `503`.
- Security: all SQL is static and every filter/sort selector is bound or mapped through a closed allowlist.
- Tests: unauthenticated rejection and authenticated pagination/response shape. Filter/count/sort execution requires `TEST_DATABASE_URL`.

### Build detail and filters contracts

- `GET /api/v1/device/build/:id` requires authentication and returns the complete release row directly. Missing IDs return the controller-intended `404 {message/error: Build not found}` contract rather than the legacy service's accidental throw-before-null-check `500`.
- `GET /api/v1/device/build/filters` requires authentication and returns `{deviceTypes:string[],deviceFamilies:string[]}` from distinct non-null release values. Rust sorts both arrays for deterministic clients.
- Routing: Rust registers `/filters` as a static route alongside `/{id}`; Axum's static precedence prevents the Express declaration-order shadowing risk.
- Tests: authenticated detail success/not-found and exact filter arrays. Live release lookup/distinct queries require `TEST_DATABASE_URL`.

### Build upload initialization contract

`POST /api/v1/device/build/upload/init` requires authentication and JSON `{fileCount:number}`.

- Validation: `fileCount` must be an integer from 1 through 5. Missing, malformed, zero, negative, or greater values return `400` with `fileCount is required and must be between 1 and 5` for range failures.
- Persistence: creates one `upload_builds` session with a generated UUID, `status=not_started`, empty `filesArray`, `totalFiles=fileCount`, `completedFiles=0`, and timestamps.
- Ownership compatibility: the legacy controller reads `req.user.id`, while its active middleware sets `req.params.userId`; consequently it persists `userId=system`. Rust preserves this observed behavior until identity propagation is deliberately corrected as a documented compatibility difference.
- Success: `201 Created` with `{uploadId:<uuid>}`.
- Tests: authenticated success, range failure, and missing field. Live row/default validation requires `TEST_DATABASE_URL`.

### TUS resumable upload contract

The frontend uses `tus-js-client` against `/api/v1/device/build/upload/tus` with 5 MiB chunks and retries after 0, 1, 3, and 5 seconds. Every method is authenticated by the build router.

- Protocol surface: collection `POST` plus resource `HEAD` and `PATCH`, with TUS `OPTIONS`; relative `Location` responses must retain the full public mount path.
- Headers/metadata: standard TUS version, upload length/offset, and base64 `Upload-Metadata` entries `filename`, `filetype`, `uploadId`, and `tag`. Requests also carry bearer authorization, legacy `refreshtoken`, and `x-user-id`.
- Limits/storage: default per-file maximum is 500 MiB (`BUILD_MAX_SIZE` MiB). Legacy files are stored under resolved `UPLOAD_DIR` or `./tmp/uploads`; generated storage names prefix the original filename with a unique suffix.
- Classification: non-empty trimmed `tag` means custom build; empty tag means official build. User-scope tag validation occurs in the frontend, not the TUS server.
- Creation side effects: metadata gains `isCustomBuild`; `upload_progress` emits zero percent with user/session/file identity.
- Completion: process the stored file through build ingestion using filename/tag/uploadId/user/custom metadata, then emit `upload_complete`. Processing failure emits `upload_error`, persists/admin-broadcasts a build-upload-failed alert, and fails the TUS completion response.
- Rust publishes progress after persisted append state, and completion/build/alert events only after transactional finalization. Finalization failure marks the session failed, emits `upload_error`, and rolls back the TUS offset/file length for retry.

Current Rust evidence:

- Configurable upload/artifact directories and 500 MiB default per-file limit.
- Authenticated create/head/patch routes, persisted JSON sidecars and payload files, relative public `Location`, strict TUS version/content-type/offset checks, CORS-safe OPTIONS capability headers, and per-upload append locks.
- Final-chunk rollback restores both file length and sidecar offset when metadata or ingestion fails, preserving retry semantics.
- Safe ZIP validation/extraction rejects parent/absolute/symlink paths and expansion overflow, enforces root Image/DTB/tarball and Gen4/Gen5 IPL layouts, stages extraction, and transactionally registers release/upload completion before publishing artifacts.
- Tests cover storage reload, route-level create/head/conflict/patch/options, finalization success/rollback, valid legacy ZIP layout, traversal, and expansion limits.
- Remaining: live legacy-shaped PostgreSQL, NFS/TFTP, controller SFTP, scanner, and official external-workflow verification.

### Multipart build upload contracts

- `POST /api/v1/device/build/upload/custom` requires an authenticated user; `POST /api/v1/device/build/upload` requires admin.
- Both stream one multipart `file` field to temporary storage and require `uploadId`; custom uploads additionally require non-empty `tag`. Only `.zip` filenames are accepted.
- Route-local body-limit disabling is paired with explicit streaming enforcement of `build_upload_max_bytes`, preserving the global 10 MiB API body limit.
- Both use the same safe staged ZIP ingestion and release/session transaction as TUS. Custom/official classification is explicit rather than inferred from an official upload's optional tag.
- Success: `200 {message:"Upload successful",uploadId,filename}`. Missing file/session/tag, wrong extension, oversize, malformed multipart, ZIP/layout/duplicate/ingestion failures return `400`; storage infrastructure failures return `500`.
- Tests: real multipart custom success, missing custom tag, and official success. Live release/session transaction and external workflow side effects remain blocked/deferred as listed above.

### Build flag and delete contracts

- `PUT /api/v1/device/build/:id/flag` is admin-only and requires JSON boolean `isFaulty`. It atomically sets `isFaulty` and status to `failed` when true or `passed` when false, then returns `200 {id,isFaulty}`. Missing/wrong input is `400`; the active legacy missing-release path is `500`.
- `DELETE /api/v1/device/build/:id` is admin-only, hard-deletes the release, and returns `204`. Missing releases follow the active legacy `500` behavior.
- After database deletion, Rust best-effort removes `<build_artifacts_dir>/<folderName>/<version>` only when both database-derived values are single normal path segments.
- IPL farm/controller cleanup and `build_deleted`/`build_flagged` persisted alerts and broadcasts are ported. Live controller cleanup remains unverified.
- Tests cover flag success, malformed input, missing release, delete success, and missing release. Live mutation/cleanup validation requires `TEST_DATABASE_URL` and production-shaped artifact paths.

### Test execution CSV export contract

`GET /api/v1/device/test/export` requires authentication and scopes rows to the validated access-token subject through `test_executions.createdBy`.

- Data joins executions to device type and test cases. Columns are `S.No`, `Test Name`, `Status`, `Device`, `Duration`, `Total`, `Passed`, `Failed`, `Qmetry Test Cycle Id`, `Execution percentage`, `Started At`, and `Ended At`.
- Completed executions with unaccounted cases are exported as `Failed`. Duration uses ended time or current time and legacy month/week/day/hour/minute/second labels. Zero-case percentage is the legacy `false`; timestamps are ISO milliseconds.
- Success: `200 text/csv` with `attachment; filename="test-executions-<ISO timestamp>.csv"`.
- Tests cover calculation/formatting, authentication, headers, filename, and body shape. User-scoped join execution requires `TEST_DATABASE_URL`.

### TestRail plan contract

`GET /api/v1/device/test/plan` requires authentication. Optional `filter` performs case-insensitive name/description matching after removing surrounding quotes; accepted `buildId` and `deviceId` are legacy no-ops.

- Upstream: Basic-auth TestRail `get_suites/<projectId>` with limit 250 and offset 0, following `_links.next` until exhausted.
- Response: bare array `{id,name,description,testSuits:[]}`.
- Configuration: base URL, API version, username, API key, project ID, and bounded timeout under `[tests]`/environment overlays. Empty base URL leaves the provider disabled and returns `503`.
- Security: pagination URLs must remain same-origin with the configured TestRail base. Upstream/status/JSON/pagination failures return `500` without exposing credentials.
- Tests: route authentication/shape/filter forwarding and mock-backed Basic auth, pagination, and filtering.

### TestRail suite contract

`GET /api/v1/device/test/suite?planId=<id>` requires authentication and a positive numeric `planId`; missing, zero, or malformed values return `400`.

- Upstream: Basic-auth `get_sections/<projectId>?limit=250&offset=0&suite_id=<planId>`, following same-origin `_links.next` pages.
- Response: bare array `{id,name,planId,order,description}` mapped from section ID/name/suite ID/display order/description.
- Errors: disabled provider `503`; bounded upstream/status/JSON/pagination failures `500`.
- Tests: required-plan validation, route response shape, and mock-backed upstream query/projection.

### TestRail testcase contract

`GET /api/v1/device/test/testcase` requires authentication and positive numeric `planId` and `suiteId`; missing/zero/malformed IDs return `400`. Optional `filter` is forwarded. Legacy `limit`/`offset` inputs are accepted but the service consistently fetches all pages in batches of 250.

- Upstream: Basic-auth `get_cases/<projectId>` with `suite_id=<planId>`, `section_id=<suiteId>`, filter, limit 250, offset 0, then same-origin `_links.next` pagination.
- Response: bare array `{id,title,planId,suiteId,order,priorityId,scriptFile,preCondition,labels}`.
- Text: percent-decodes script/precondition fields, strips HTML tags, normalizes whitespace and common entities, and maps label titles.
- Tests: route success/required IDs and mock-backed query, projection, percent/HTML cleanup, and labels.

### Qmetry testcase path contracts

- Both routes require authentication and IDs at least 7, matching the legacy coercing schema. Invalid IDs return `400`.
- Plan path fetches Qmetry testcase-folder hierarchy (`sort=NAME:asc`, `withCount=true`), locates the plan, searches each child folder, and returns an object keyed by folder ID. Entries include `{id,name,script_file,key,seqNo,version,labels}`.
- Suite path searches only `suiteID` and returns a bare array without labels, matching the legacy projection; `planID` is validated but otherwise unused.
- Qmetry uses configurable base URL, `apikey`, Jira project ID, and bounded timeout. Search is POST `/testcases/search/?fields=seqNo,key,version,summary,priority,status` with `{filter:{projectId,folderId}}`.
- Disabled provider returns `503`; upstream/status/JSON failures return `500`.
- Tests cover route shapes/validation and mock-backed API-key, hierarchy, search payload, Script File custom field, labels, and shape differences.

### Test execution detail contract

`GET /api/v1/device/test/execution/:id` requires authentication and scopes default detail to `createdBy = validated token subject`.

- `id=latest` selects the newest execution in `not_executed`, `queued`, or `in_progress`; other IDs match `testId` exactly.
- Response preserves the full execution row, joined `Device`, and `logs` loaded from configured `<test_logs_dir>/<testId>.json`; absent/invalid log files yield `[]`.
- Missing or non-owned execution returns `404 Test Execution Not Found`.
- `?table=...` returns the distinct legacy summary contract with joined cases, computed duration/counts, current user analytics, and configured filesystem logs. Legacy summary lookup is not owner-scoped; only analytics uses the token subject.
- Tests cover direct/latest, not-found, configured logs, and table mode. Live ownership/status/query validation requires `TEST_DATABASE_URL`.

### Execution creation and update contracts

- Creation accepts both legacy `testPlanId`/`testSuites`/`testCases`/`isAllSelected` and selection-based `ALL`/`PARTIAL` payloads. Rust expands catalog selections, applies exclusions, deduplicates by testcase ID, and atomically inserts the execution plus cases with `not_executed`/`PREPARE_ARTIFACTS` state.
- Creation is queue-first and persists no device assignment. Response is `201` with the hydrated execution. Owned workers perform artifact preparation, TestRail run creation, durable action queueing, and committed `test_execution_update` events.
- Update treats path `id` as the TestRail case ID and body `testID` as the run/execution lookup ID, resolves the stored build version, maps the five legacy result names through TestRail statuses, and returns legacy `201 {}` even when the upstream update fails. `log=true` best-effort persists an error log.

### Additional execution read contracts

- `/execution/single/:testId` requires authentication but is not owner-scoped; returns full execution plus ID-ordered `testCases`, or `404`.
- `/execution/list/:id` is public despite its legacy name; for executions not `cancelled`/`failed`, returns testcase IDs as strings ordered by suite then row ID. Missing/excluded execution is `404`.
- `/execution/device/:id` requires authentication, returns the user's newest execution for the device by `updatedAt`, and attaches configured JSON logs. Missing/non-owned execution is `404`.
- `/execution/progress/list` requires authentication and returns user-owned `{deviceId,status,testId}` entries in `in_progress`, `not_executed`, or `queued`.
- Static route tests confirm these paths are not captured by `/execution/:id`. Live repository semantics require `TEST_DATABASE_URL`.

### Paginated execution list contract

`GET /api/v1/device/test/execution` requires authentication and scopes rows to the validated token subject.

- Query defaults: `sortBy=createdAt`, `desc=true`, `page=1`, `limit=10`; page/limit clamp to at least 1. Search matches joined device type only.
- Sort is allowlisted to `createdAt`, `status`, `testPlanName`, `buildVersion`, or joined `deviceType`, with `createdAt` fallback.
- Response: `{data,total,currentPage,totalPages}`. Each row contains computed duration, total/pass/fail counts, build/device/status/plan/cycle/timestamp/RTOS fields, `testCases:{}`, and `logs:[]`, matching the legacy table projection.
- Tests cover auth plumbing, page normalization, pagination keys, and row shape. Live filters/counts/sorts require `TEST_DATABASE_URL`.

### Execution testcase and log contracts

- `/execution/cases/:id` requires authentication and ownership, returning selected testcase fields ordered by row ID. The legacy service wraps missing/non-owned execution into `500`; Rust preserves this active behavior.
- `/testcase/:testCaseId` requires authentication but is not owner-scoped, returns the complete testcase row, and preserves legacy `500` on missing case.
- `/execution/logs/:id` requires authentication but is not owner-scoped. It reads configured JSON logs, joins entries with newlines, and returns `text/plain` with `attachment; filename="test-<id>-logs.txt"`; missing/invalid files yield an empty download.
- Tests cover execution cases, testcase detail, content headers, filename, and log body. Live ownership/query projections require `TEST_DATABASE_URL`.

### Execution report read contracts

- `/execution/report/:id` requires authentication and returns the newest `execution_reports` row for the test execution, or `404 No report found for this execution`.
- `/execution/report/:id/html` requires authentication, resolves execution device type/build version, validates each path segment, and reads `<test_results_dir>/<deviceType>/<buildVersion>/<testId>/report.html` as `text/html; charset=utf-8`. Missing execution/HTML is `404`.
- `/testcase/:testCaseId/log` requires authentication, reads the testcase `outputFilePath`, canonicalizes both candidate and configured `test_results_dir`, rejects paths outside that root, and returns `text/plain; charset=utf-8` with basename attachment filename.
- Tests use real temporary report/log files for metadata, HTML content type/body, root-contained testcase log, and attachment filename. Live metadata/path lookup requires `TEST_DATABASE_URL`.

### Execution report generation/upload contracts

- Generation requires authentication and a completed execution. It enqueues the native bounded report worker and creates a legacy `execution_reports` row using the native job UUID, preserving generating/conflict/regeneration semantics. Returns `200` report metadata.
- Upload requires authentication, a completed/uploaded report or completed native job, and calls the native Confluence uploader. Success persists `uploaded` and returns `202`; failure records `failed`/`uploadError` and returns `502`.
- The native report worker owns generation and filesystem artifacts; no internal HTTP hop remains.
- Tests exercise the real bounded queue, legacy response record, terminal `execution_reports` synchronization, and completion/failure events. Live PostgreSQL conflict/status persistence and live Confluence remain environment-dependent.

## Relay routes

| Method | Path below `/api/v1/device/relay` | Access | Rust status |
|---|---|---|---|
| POST | `/config` | admin | ported; legacy auto-unmapping and hardware dispatch |
| POST | `/config/fresh` | admin | ported; hard duplicate cleanup |
| POST | `/config/remap` | admin | ported; channel migration and hardware resynchronization |
| POST | `/config/confirmation` | public | ported and route-tested |
| GET | `/check-conflicts` | admin | ported; includes stale duplicate cleanup |
| GET | `/` | admin | ported; optional `controllerId` filter |
| GET | `/channel` | admin | ported; optional `deviceId`/`relayId` filters |
| GET | `/channel/:id` | admin | ported |
| GET | `/:id` | admin | ported |
| DELETE | `/channel/:id` | admin | ported; paranoid soft delete |
| DELETE | `/:id` | admin | ported; paranoid soft delete |
| GET | `/controller/:id` | admin | ported |
| GET | `/channels/relay/:id` | admin | ported |
| GET | `/devices/available` | admin | ported |
| POST | `/configure` | admin | ported; batch/move/conflict tests |
| GET | `/state/:id` | admin | ported |
| PUT | `/toggle/:deviceId` | user | ported |

## Analytics routes

All require an authenticated user and are ported with static parameterized SQL, OpenAPI registration, and focused HTTP contract tests. Execution lists/summaries, build comparisons, and test analytics are scoped to the JWT subject. Historical exceptions are preserved: execution lookup by test ID and build performance are unscoped, while build comparison selects user-associated builds but aggregates completed executions system-wide. Device usage includes approved, non-deleted devices. Date windows use UTC in Rust; live legacy-schema query compatibility remains blocked by the unavailable `TEST_DATABASE_URL`.

| Method | Path below `/api/v1/device/analytics` | Query/defaults and behavior | Rust status |
|---|---|---|---|
| GET | `/state` | approved device totals grouped by state | ported and shape-tested |
| GET | `/state/detailed` | approved devices grouped by family and state | ported and shape-tested |
| GET | `/execution` | `count`, default 3; current-user recent executions | ported and shape-tested |
| GET | `/execution/daily` | `from`/`to` as `YYYY-MM-DD`, default trailing 7 UTC dates | ported; invalid dates return `400` |
| GET | `/execution/:testId` | historical unscoped lookup | ported; missing execution returns `404` |
| GET | `/builds/comparison` | `count`, default 3, clamped 1-50 | ported and shape-tested |
| GET | `/builds/comparison/:buildId` | user-associated build lookup | ported; missing comparison returns `404` |
| GET | `/builds/performance` | `count`, default 10, clamped 1-50; historically unscoped | ported and shape-tested |
| GET | `/builds/performance/:buildId` | historically unscoped | ported and shape-tested |
| GET | `/usage/daily` | optional `day`, `deviceId`, `deviceFamily`; defaults to trailing 24 hours | ported; invalid day returns `400` |
| GET | `/usage/summary` | four weekly and four monthly UTC periods | ported and shape-tested |
| GET | `/test` | required `type=execution|inProgress`; current-user aggregates | ported; invalid type returns explicit `400` instead of the legacy controller's incidental `500` |
| GET | `/test/plan` | current-user plan status totals | ported and shape-tested |
| GET | `/test/daily` | `days`, default 8, clamped 1-90 | ported and shape-tested |

## Notification routes

All five routes are ported. List routes are admin-only; the canonical `/:id/read` mutation accepts any authenticated user, while the retained `/read/:id` and `/all/read` routes are admin-only. The admin list preserves active/recent filtering, approved-device/related-entity visibility, sort direction, pagination, and the historical empty-result `500`; legacy `search` and `filter` parameters remain no-ops. The all-alert list preserves invalid-date tolerance and includes `totalUnreadCount`. The canonical single-read route returns `404` for a missing ID, while the retained admin route returns success for zero matching rows. Mark-all remains best effort and returns success after logging a database failure. Rust caps list limits at 100; live enum/table validation remains blocked by unavailable PostgreSQL infrastructure.

| Method | Path below `/api/v1/device/notification` | Access | Rust status |
|---|---|---|---|
| GET | `/alerts` | admin | ported; active/recent visibility and pagination shape tested |
| GET | `/alerts/all` | admin | ported; date tolerance and unread count tested |
| PUT | `/alerts/:id/read` | user | ported; success and missing-ID behavior tested |
| PUT | `/alerts/read/:id` | admin | ported; legacy zero-row success preserved |
| PUT | `/alerts/all/read` | admin | ported; legacy best-effort success preserved |

## Faulty-report routes

All seven routes are ported. Create accepts authenticated multipart field `image`, streams at most 10 MiB, accepts `.txt`, `.log`, `.pdf`, `.png`, `.jpg`, `.jpeg`, `.gif`, or `.zip`, stores beneath configurable `device.faulty_report_upload_dir`, and optionally copies root-contained test logs. It preserves release lookup, latest execution/device fallback, pending status, alert persistence, committed browser alert emission, and the `201` report response. Admin reads preserve list ordering, Keycloak reporter enrichment, attachment/log download headers, approved/rejected validation, approved-release `isFaulty` mutation, and hard delete. Rust adds upload-root containment for downloads and execution-ID path validation. Live legacy-schema SQL remains blocked by unavailable PostgreSQL infrastructure.

| Method | Path below `/api/v1/device/faulty` | Access | Rust status |
|---|---|---|---|
| POST | `/report` | user, multipart `image` | ported; required fields, bounded streaming, fallback lookup, storage, and alert persistence |
| GET | `/report` | admin | ported; descending creation order |
| GET | `/report/:id` | admin | ported; build version and Keycloak user enrichment |
| GET | `/report/:id/file` | admin | ported; root-contained attachment with content headers tested |
| GET | `/report/:id/logs` | admin | ported; root-contained log download |
| PATCH | `/report/:id/status` | admin | ported; approve/reject and release fault mutation |
| DELETE | `/report/:id` | admin | ported; legacy hard delete without file cleanup |

## Log and WebSocket-dispatch routes

| Method | Path | Access | Rust status |
|---|---|---|---|
| POST | `/api/v1/device/log/` | user | ported; defaults/validation/auth/response tested, live PostgreSQL blocked |
| GET | `/api/v1/device/log/` | user | ported; filtering/pagination/sort/auth/shape tested, live PostgreSQL blocked |
| GET | `/api/v1/device/log/search` | user | ported; required query/pagination/search shape tested, live PostgreSQL blocked |
| POST | `/api/v1/device/ws/send/message` | public | ported; dispatches the `user` envelope through the owned event publisher and installed Socket.IO adapter, returning `{ok:true}` |

All active legacy route rows are ported. Remaining contract work is live consumer/provider verification and documentation of any operationally observed differences.

## Reports

The reports service is currently reached internally rather than through the API gateway's service map. The unified Rust listener will expose the final compatibility prefix after all current callers and deployment routing are verified.

| Method | Current reports-service path | Access | Behavior | Rust status |
|---|---|---|---|---|
| GET | `/health` | public | `{status: "ok", service: "reports-service"}` | inventory |
| POST | `/report` | internal | validate request, enqueue, return `202` with job ID | ported; bounded queue and artifact lifecycle tests |
| GET | `/report/:jobId` | internal | return job state or FastAPI-style `404` detail | ported |
| POST | `/upload-confluence` | internal | validate safe test-result path and existing report, return `202` | ported; path and Confluence tests |
| Socket.IO | `/socket.io` | internal/frontend event clients | join/leave job rooms and progress events | ported; report completion/failure events target user and test rooms |

The native worker preserves the legacy process-local queue/status lifetime and persists terminal execution-report state separately. Filesystem fixtures verify escaped consolidated artifacts, fatal sanity-analysis failures, per-command performance comparisons, radar/category PNGs, CSV/JSON summaries, combined report image/link references, and post-persistence events. Missing performance history remains nonfatal as in the Python service; missing required sanity result input fails the job. Live Confluence upload remains external.

## Unified operational API

The Rust application adds these required routes on the same listener. They are additive and do not replace prefixed compatibility routes.

| Method | Path | Module | Intended semantics |
|---|---|---|---|
| GET | `/health` | health | process liveness only |
| GET | `/ready` | health | readiness of enabled required dependencies |
| GET | `/metrics` | metrics | Prometheus text exposition |
| GET | `/openapi.json` | swagger | generated public API contract |
| GET | `/swagger` | swagger | Swagger UI redirect or page |
| GET | `/swagger-ui/*` | swagger | Swagger UI assets/routes |

## Known contract risks

- Express route declaration order currently places some dynamic routes before static routes. Rust route matching must implement intended frontend behavior and compatibility tests must capture actual responses.
- Current authorization is inconsistent: several destructive or state-changing routes are public, while some `adminRoleAuthorization` routes do not visibly compose `authMiddleware`. Preserve observed behavior during parity, document it, then harden through an explicit compatibility decision.
- The frontend uses Socket.IO in addition to native WebSocket. A plain Axum WebSocket endpoint is not protocol-compatible with Socket.IO.
- EdgeController heartbeat messages are binary JSON frames, not text JSON.
- Reports progress is asynchronous and room-scoped; acknowledging a report request is not equivalent to completing report generation.
- OpenAPI includes every mounted operation with a summary, behavioral description, success response, stable envelopes, bounded errors, multipart/binary/CSV media, representative examples, event routes, and TUS headers. The generated-document test enforces the summary/description/success invariant globally.
- Many SQLx repositories assume a legacy Sequelize-created public schema. Route-level tests with fake repositories do not prove production-schema compatibility.
- Current format/check/strict-Clippy/full-test gates pass; live PostgreSQL compatibility remains environment-blocked.

## Compatibility status

### EdgeController

Sibling source calls public registration, mapping, flash confirmation, relay confirmation, and `/ws`; corresponding Rust routes and focused fixtures exist. Binary JSON heartbeat and `web-cli-protocol` are preserved. Persisted controller timeout/recovery is ported; live EdgeController-to-Rust integration has not been run end to end.

### Frontend

Active HTTP paths and response shapes used by Axios services are mounted and source-reconciled, including analytics, notifications, faulty reports, uploads, reports, relay-device selection, heartbeat timeout, power toggle, and execution creation/detail/table summary. Focused fixtures follow power toggle and heartbeat timeout through the controller boundary and verify execution/report/relay event publication. The active Socket.IO listener/producer/room matrix is reconciled, including global build-status delivery, dashboard/build-room performance fan-out, five-second `system_metrics_update`, and successful pending-user `user` events. Live frontend/EdgeController/E2E runs remain.

## NEXT AGENT ACTION

Run the live legacy-shaped PostgreSQL suite and frontend/EdgeController smoke path against the unified listener when the required environments are available.