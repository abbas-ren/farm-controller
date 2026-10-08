#[cfg(test)]
pub mod tests;

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderName, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Serialize;
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::{Any, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    sensitive_headers::SetSensitiveRequestHeadersLayer,
};
use utoipa::{Modify, OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    admin_control, auth, config::Module, devices, error::ErrorResponse, events,
    observability::track_request, reports, runtime_log_handlers, state::AppState,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        ready,
        metrics,
        admin_control::get,
        admin_control::patch,
        admin_control::action,
        runtime_log_handlers::logs,
        runtime_log_handlers::update_level,
        auth::signin,
        auth::request_registration,
        auth::registration_action,
        auth::pending_users,
        auth::all_users,
        auth::active_users,
        auth::user_by_id,
        auth::delete_user,
        auth::verify_user,
        auth::reset_password,
        auth::forgot_password,
        auth::register_user,
        auth::user_event_webhook,
        auth::validate,
        auth::validate_admin,
        auth::validate_user,
        events::send_user_message,
        events::upgrade,
        devices::build_handlers::build_by_id,
        devices::build_handlers::build_filters,
        devices::build_handlers::delete_build,
        devices::build_handlers::flag_build,
        devices::build_handlers::init_upload,
        devices::build_handlers::list_builds,
        devices::build_handlers::upload_custom,
        devices::build_handlers::upload_official,
        devices::tus_handlers::create,
        devices::tus_handlers::head,
        devices::tus_handlers::options,
        devices::tus_handlers::options_resource,
        devices::tus_handlers::patch,
        devices::device_export_handlers::export_devices,
        devices::device_registration_handlers::register_device,
        devices::flashing_handlers::mark_device_flashing,
        devices::log_handlers::create,
        devices::log_handlers::list,
        devices::log_handlers::search,
        devices::test_completion_handlers::test_completed,
            devices::analytics_handlers::device_state,
            devices::analytics_handlers::detailed_device_state,
            devices::analytics_handlers::recent_executions,
            devices::analytics_handlers::daily_executions,
            devices::analytics_handlers::execution_by_id,
            devices::analytics_handlers::build_comparisons,
            devices::analytics_handlers::build_comparison_by_id,
            devices::analytics_handlers::builds_performance,
            devices::analytics_handlers::build_performance_by_id,
            devices::analytics_handlers::daily_device_usage,
            devices::analytics_handlers::device_usage_summary,
            devices::analytics_handlers::test_analytics,
            devices::analytics_handlers::test_plan_summary,
            devices::analytics_handlers::daily_test_summary,
        devices::alert_handlers::admin_alerts,
        devices::alert_handlers::all_alerts,
        devices::alert_handlers::mark_single_read,
        devices::alert_handlers::mark_read,
        devices::alert_handlers::mark_all_read,
        devices::faulty_report_handlers::create,
        devices::faulty_report_handlers::list,
        devices::faulty_report_handlers::by_id,
        devices::faulty_report_handlers::download_file,
        devices::faulty_report_handlers::download_logs,
        devices::faulty_report_handlers::update_status,
        devices::faulty_report_handlers::delete,
        devices::test_export_handlers::export_tests,
        devices::test_execution_handlers::execution,
        devices::test_execution_handlers::create,
        devices::test_execution_handlers::update,
        devices::test_execution_handlers::list,
        devices::test_execution_handlers::single,
        devices::test_execution_handlers::case_ids,
        devices::test_execution_handlers::by_device,
        devices::test_execution_handlers::progress,
        devices::test_execution_handlers::cases,
        devices::test_execution_handlers::testcase,
        devices::test_execution_handlers::logs,
        devices::test_execution_handlers::report,
        devices::test_execution_handlers::report_html,
        devices::test_execution_handlers::testcase_log,
        devices::test_execution_handlers::create_report,
        devices::test_execution_handlers::upload_report,
        devices::test_execution_handlers::by_build,
        devices::test_execution_handlers::results,
        devices::test_execution_handlers::cancel,
        devices::test_catalog_handlers::plans,
        devices::test_catalog_handlers::suites,
        devices::test_catalog_handlers::cases,
        devices::test_catalog_handlers::qmetry_plan_cases,
        devices::test_catalog_handlers::qmetry_suite_cases,
        devices::register_controller,
        devices::mapping_gen5,
        devices::flash_confirm,
        devices::flash_confirm_gen4,
        devices::device_families,
        devices::device_types,
        devices::device_by_id,
        devices::latest_heartbeat,
        devices::device_topology,
        devices::list_devices,
        devices::device_action,
        devices::list_controllers,
        devices::list_user_devices,
        devices::active_devices,
        devices::update_heartbeat_timeout,
        devices::edit_controller,
        devices::delete_controller,
        devices::delete_device,
        devices::builds_for_device_type,
        devices::configure_artifacts,
        devices::artifact_handlers::copy_default_artifacts,
        devices::builds_for_device,
        devices::reboot_device,
        devices::toggle_device_power,
        devices::relays_for_controller,
        devices::channels_for_relay,
        devices::available_relay_devices,
        devices::relay_handlers::relay_device_state,
        devices::relay_handlers::update_relay_identity,
        devices::relay_handlers::configure_relay_channels,
        devices::relay_handlers::confirm_relay_configuration,
        devices::uart_handlers::configure_uart,
        devices::relay_handlers::configure_legacy_relay,
        devices::relay_handlers::fresh_legacy_relay,
        devices::relay_handlers::remap_legacy_relay,
        devices::relay_handlers::legacy_relay_conflicts,
        devices::relay_handlers::legacy_relays,
        devices::relay_handlers::legacy_relay_channels,
        devices::relay_handlers::legacy_relay_channel_by_id,
        devices::relay_handlers::legacy_relay_by_id,
        devices::relay_handlers::delete_legacy_relay_channel,
        devices::relay_handlers::delete_legacy_relay,
        reports::enqueue_report,
        reports::report_status,
        reports::upload_confluence
    ),
    components(schemas(
        HealthResponse,
        ReadinessResponse,
        ErrorResponse,
        auth::SigninRequest,
        auth::SigninResponse,
        auth::RegistrationRequest,
        auth::RegistrationAction,
        auth::RegistrationActionRequest,
        auth::ManagedUser,
        auth::VerifyUserRequest,
        auth::ResetPasswordRequest,
        auth::ForgotPasswordRequest,
        auth::AdminRegistrationRequest,
        auth::AdminRegistrationResult,
        auth::UserEventRequest,
        auth::AuthenticatedUser,
        auth::AuthMessageResponse,
        auth::AuthErrorResponse,
        auth::ManagedUsersResponse,
        auth::ActiveUsersResponse,
        auth::AuthUserResponse,
        auth::AdminRegistrationResponse,
        auth::ValidationResponse,
        events::UserMessageRequest,
        events::UserMessageResponse,
        events::EventErrorResponse,
        devices::ControllerRegistration,
        devices::ControllerRegistrationResponse,
        devices::DeviceRegistration,
        devices::DeviceRegistrationResponse,
        devices::DeviceDataResponse,
        devices::DeviceFlashingResponse,
        devices::DeviceHeartbeatResponse,
        devices::DeviceTopologyResponse,
        devices::DeviceCsvQuery,
        devices::TestCompletionQuery,
        devices::DeviceController,
        devices::RelayRegistration,
        devices::RelayChannelRegistration,
        devices::Relay,
        devices::RelayRecord,
        devices::RelayChannelRecord,
        devices::MappingCallback,
        devices::CallbackResponse,
        devices::DeviceListQuery,
        devices::DeviceList,
        devices::DeviceAction,
        devices::DeviceActionRequest,
        devices::ControllerListQuery,
        devices::ControllerList,
        devices::UserDeviceListQuery,
        devices::HeartbeatTimeoutRequest,
        devices::ControllerEditRequest,
        devices::BuildsQuery,
        devices::BuildListQuery,
        devices::BuildList,
        devices::BuildFilters,
        devices::BuildFlagRequest,
        devices::BuildUploadInitRequest,
        devices::BuildUploadInitResponse,
        devices::BuildUploadResponse,
        devices::BuildFlagResponse,
        devices::ExecutionListQuery,
        devices::ExecutionList,
        devices::ActiveExecutionRecord,
        devices::ExecutionCaseRecord,
        devices::ExecutionReportRecord,
        devices::BuildExecutionList,
        devices::EmptyObjectResponse,
        devices::CreateExecutionRequest,
        devices::ExecutionCaseInput,
        devices::ExecutionSelection,
        devices::UpdateExecutionRequest,
        devices::LogCreateRequest,
        devices::LogListQuery,
        devices::LogEntry,
        devices::LogList,
            devices::DeviceStateAnalytics,
            devices::DeviceStateDetailedItem,
            devices::DeviceStateDetailedAnalytics,
        devices::AlertList,
        devices::MessageResponse,
        devices::CompatibilityErrorResponse,
        devices::FaultyReportCreateRequest,
        devices::FaultyReportDetail,
        devices::FaultyReportRecord,
        devices::FaultyReportStatus,
        devices::FaultyReportUser,
        devices::SuccessResponse,
        devices::FaultyReportStatusRequest,
        devices::test_catalog_handlers::TestPlanQuery,
        devices::test_catalog_handlers::TestSuiteQuery,
        devices::test_catalog_handlers::TestCaseQuery,
        crate::test_catalog::TestPlan,
        crate::test_catalog::TestSuite,
        crate::test_catalog::TestCase,
        devices::DeviceTypeFolder,
        devices::DeviceTypeFolderRequest,
        devices::DefaultArtifactCopyQuery,
        devices::DefaultArtifactCopyResponse,
        devices::AvailableRelayDevicesQuery,
        devices::AvailableRelayDevice,
        devices::AvailableRelayDevicesResponse,
        devices::PaginationResponse,
        devices::LegacyRelayConfigurationResponse,
        devices::RelayConflictResponse,
        devices::RelayIdentityUpdateRequest,
        devices::RelayIdentityUpdateResponse,
        devices::TestCompletionResponse,
        devices::RelayChannelConfiguration,
        devices::RelayConfigurationRequest,
        devices::RelayConfigurationConfirmation,
        devices::RelayConfigurationAccepted,
        devices::RelayConfirmationResponse,
        devices::UartConfigurationRequest,
        devices::UartConfigurationResponse,
        devices::LegacyRelayConfiguration,
        devices::LegacyRelayRemapRequest,
        devices::RelayConflictState,
        reports::ReportRequest,
        reports::ReportAcknowledgement,
        reports::ReportJobSnapshot,
        reports::ReportStatus,
        reports::ConfluenceUploadRequest,
        reports::ConfluenceUploadResponse
    )),
    tags((name = "Operations", description = "FarmController process and dependency status")),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some(
                            "Keycloak access token. Expired tokens may be renewed with the legacy refresh-token header or cookie.",
                        ))
                        .build(),
                ),
            );
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
struct HealthResponse {
    /// Process liveness state.
    status: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
struct ReadinessResponse {
    /// Whether all dependencies required by enabled modules are ready.
    status: &'static str,
    /// Enabled runtime modules.
    modules: Vec<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    let request_id_header = HeaderName::from_static("x-request-id");
    let cors = cors_layer(&state.config.server.cors_allowed_origins);
    let socket_io_layer = state
        .config
        .module_enabled(Module::Events)
        .then(|| events::socket_io_layer(&state.event_publisher));
    let mut router = Router::new();

    if state.config.module_enabled(Module::Health) {
        router = router
            .route("/health", get(health))
            .route("/ready", get(ready));
    }
    if state.config.module_enabled(Module::Auth) {
        router = router.nest("/api/v1/auth", auth::router());
    }
    if state.config.module_enabled(Module::Device) {
        router = router
            .route(
                "/api/v1/device/",
                get(devices::list_devices)
                    .post(devices::device_registration_handlers::register_device),
            )
            .nest("/api/v1/device", devices::router());
    }
    if state.config.module_enabled(Module::Events) {
        router = router.merge(events::router());
    }
    if state.config.module_enabled(Module::Reports) {
        router = router.merge(reports::router());
    }
    if state.config.module_enabled(Module::Metrics) {
        router = router.route("/metrics", get(metrics));
    }
    router = router
        .route("/api/v1/operations/logs", get(runtime_log_handlers::logs))
        .route(
            "/api/v1/operations/logs/level",
            axum::routing::put(runtime_log_handlers::update_level),
        )
        .route(
            "/api/v1/admin/control",
            get(admin_control::get)
                .patch(admin_control::patch)
                .post(admin_control::action),
        );
    if state.config.module_enabled(Module::Swagger) {
        router = router
            .route(
                "/swagger",
                get(|| async { Redirect::permanent("/swagger-ui/") }),
            )
            .merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", ApiDoc::openapi()));
    }
    router = router.fallback(not_found);
    if let Some(layer) = socket_io_layer {
        router = router.layer(layer);
    }

    let body_limit = state.config.server.request_body_limit_bytes;
    router
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(CatchPanicLayer::new())
        .layer(cors)
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn(tus_options_headers))
        .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(
            axum::http::header::AUTHORIZATION,
        )))
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(middleware::from_fn_with_state(state.clone(), track_request))
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
        .with_state(state)
}

fn cors_layer(allowed_origins: &[String]) -> CorsLayer {
    if allowed_origins.is_empty() {
        return CorsLayer::permissive();
    }
    let origins = allowed_origins
        .iter()
        .map(|origin| HeaderValue::from_str(origin).expect("CORS origins are validated at startup"))
        .collect::<Vec<_>>();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods(Any)
        .allow_headers(Any)
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    response
}

async fn tus_options_headers(request: Request<Body>, next: Next) -> Response {
    let is_tus_options = request.method() == axum::http::Method::OPTIONS
        && request
            .uri()
            .path()
            .starts_with("/api/v1/device/build/upload/tus");
    let mut response = next.run(request).await;
    if is_tus_options {
        response.headers_mut().insert(
            "tus-resumable",
            axum::http::HeaderValue::from_static("1.0.0"),
        );
        response
            .headers_mut()
            .insert("tus-version", axum::http::HeaderValue::from_static("1.0.0"));
        response.headers_mut().insert(
            "tus-extension",
            axum::http::HeaderValue::from_static("creation"),
        );
    }
    response
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "Operations",
    summary = "Check process liveness",
    description = "Returns success when the FarmController process and HTTP event loop are alive.",
    responses((status = 200, description = "Process is alive", body = HealthResponse, example = json!({"status": "ok"})))
)]
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[utoipa::path(
    get,
    path = "/ready",
    tag = "Operations",
    summary = "Check traffic readiness",
    description = "Returns success after every dependency required by the enabled runtime modules is ready.",
    responses((status = 200, description = "Enabled modules are ready", body = ReadinessResponse), (status = 503, description = "A required dependency is not ready", body = ReadinessResponse))
)]
async fn ready(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let modules = state
        .config
        .modules
        .enabled
        .iter()
        .map(|module| format!("{module:?}").to_lowercase())
        .collect();
    if let Some(database) = &state.database
        && database.ping().await.is_err()
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessResponse {
                status: "not_ready",
                modules,
            }),
        );
    }
    (
        StatusCode::OK,
        Json(ReadinessResponse {
            status: "ready",
            modules,
        }),
    )
}

#[utoipa::path(
    get,
    path = "/metrics",
    tag = "Operations",
    summary = "Export Prometheus metrics",
    description = "Returns low-cardinality process and HTTP metrics in Prometheus text exposition format.",
    responses(
        (status = 200, description = "Prometheus text exposition", body = String, content_type = "text/plain"),
        (status = 500, description = "Metric encoding failed", body = ErrorResponse)
    )
)]
async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let database_pool =
        state
            .database
            .as_ref()
            .map(|database| crate::observability::DatabasePoolSnapshot {
                open: database.pool().size(),
                idle: database.pool().num_idle(),
                max: state.config.database.max_connections,
            });
    match state
        .metrics
        .encode(state.started_at.elapsed(), database_pool)
    {
        Ok(body) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; version=0.0.4",
            )],
            body,
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%error, "failed to encode Prometheus metrics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "metrics_encoding".to_owned(),
                    message: "Metrics are temporarily unavailable".to_owned(),
                    request_id: None,
                }),
            )
                .into_response()
        }
    }
}

async fn not_found(request: Request<Body>) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            code: "not_found".to_owned(),
            message: format!("No route for {} {}", request.method(), request.uri().path()),
            request_id: request
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        }),
    )
}
