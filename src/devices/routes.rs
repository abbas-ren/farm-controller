use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};

use crate::state::AppState;

use super::alert_handlers::{admin_alerts, all_alerts, mark_all_read, mark_read, mark_single_read};
use super::analytics_handlers::{
    build_comparison_by_id, build_comparisons, build_performance_by_id, builds_performance,
    daily_device_usage, daily_executions, daily_test_summary, detailed_device_state, device_state,
    device_usage_summary, execution_by_id, recent_executions, test_analytics, test_plan_summary,
};
use super::artifact_handlers::copy_default_artifacts;
use super::build_handlers::{
    build_by_id, build_filters, delete_build, flag_build, init_upload,
    list_builds as list_build_inventory, upload_custom, upload_official,
};
use super::device_export_handlers::export_devices;
use super::device_registration_handlers::register_device;
use super::faulty_report_handlers;
use super::flashing_handlers::mark_device_flashing;
use super::log_handlers;
use super::relay_handlers::{
    configure_legacy_relay, configure_relay_channels, confirm_relay_configuration,
    delete_legacy_relay, delete_legacy_relay_channel, fresh_legacy_relay, legacy_relay_by_id,
    legacy_relay_channel_by_id, legacy_relay_channels, legacy_relay_conflicts, legacy_relays,
    remap_legacy_relay, update_relay_identity,
};
use super::test_catalog_handlers::{
    cases as test_cases, plans as test_plans, qmetry_plan_cases, qmetry_suite_cases,
    suites as test_suites,
};
use super::test_completion_handlers::test_completed;
use super::test_execution_handlers::{
    by_build as executions_by_build, by_device as execution_by_device, cancel as cancel_execution,
    case_ids as execution_case_ids, cases as execution_cases, create as create_execution,
    create_report, execution as test_execution, list as execution_list, logs as execution_logs,
    progress as execution_progress, report as execution_report,
    report_html as execution_report_html, results as execution_results, single as single_execution,
    testcase as single_testcase, testcase_log, update as update_execution, upload_report,
};
use super::test_export_handlers::export_tests;
use super::tus_handlers;
use super::{
    active_devices, available_relay_devices, builds_for_device, builds_for_device_type,
    channels_for_relay, configure_artifacts, delete_controller, delete_device, device_action,
    device_by_id, device_families, device_topology, device_types, edit_controller, flash_confirm,
    flash_confirm_gen4, latest_heartbeat, list_controllers, list_devices, list_user_devices,
    mapping_gen5, reboot_device, register_controller, relay_device_state, relays_for_controller,
    toggle_device_power, update_heartbeat_timeout,
};

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_devices).post(register_device))
        .route("/export", get(export_devices))
        .route("/controller", post(register_controller))
        .route("/controller", get(list_controllers))
        .route(
            "/controller/{id}",
            axum::routing::put(edit_controller).delete(delete_controller),
        )
        .route("/controller/", post(register_controller))
        .route("/mapping-gen5", post(mapping_gen5))
        .route("/flash-confirm", get(flash_confirm))
        .route("/flash-confirm-gen4", get(flash_confirm_gen4))
        .route("/log", get(log_handlers::list).post(log_handlers::create))
        .route("/log/", get(log_handlers::list).post(log_handlers::create))
        .route("/log/search", get(log_handlers::search))
        .route("/analytics/state", get(device_state))
        .route("/analytics/state/detailed", get(detailed_device_state))
        .route("/analytics/execution", get(recent_executions))
        .route("/analytics/execution/daily", get(daily_executions))
        .route("/analytics/execution/{testId}", get(execution_by_id))
        .route("/analytics/builds/comparison", get(build_comparisons))
        .route(
            "/analytics/builds/comparison/{buildId}",
            get(build_comparison_by_id),
        )
        .route("/analytics/builds/performance", get(builds_performance))
        .route(
            "/analytics/builds/performance/{buildId}",
            get(build_performance_by_id),
        )
        .route("/analytics/usage/daily", get(daily_device_usage))
        .route("/analytics/usage/summary", get(device_usage_summary))
        .route("/analytics/test", get(test_analytics))
        .route("/analytics/test/plan", get(test_plan_summary))
        .route("/analytics/test/daily", get(daily_test_summary))
        .route("/notification/alerts", get(admin_alerts))
        .route("/notification/alerts/all", get(all_alerts))
        .route(
            "/notification/alerts/{id}/read",
            axum::routing::put(mark_single_read),
        )
        .route(
            "/notification/alerts/read/{id}",
            axum::routing::put(mark_read),
        )
        .route(
            "/notification/alerts/all/read",
            axum::routing::put(mark_all_read),
        )
        .route(
            "/faulty/report",
            get(faulty_report_handlers::list)
                .post(faulty_report_handlers::create)
                .layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/faulty/report/{id}",
            get(faulty_report_handlers::by_id).delete(faulty_report_handlers::delete),
        )
        .route(
            "/faulty/report/{id}/file",
            get(faulty_report_handlers::download_file),
        )
        .route(
            "/faulty/report/{id}/logs",
            get(faulty_report_handlers::download_logs),
        )
        .route(
            "/faulty/report/{id}/status",
            axum::routing::patch(faulty_report_handlers::update_status),
        )
        .route("/families", get(device_families))
        .route("/deviceTypes", get(device_types))
        .route("/builds", get(builds_for_device_type))
        .route("/build", get(list_build_inventory))
        .route("/build/filters", get(build_filters))
        .route("/build/upload/init", post(init_upload))
        .route(
            "/build/upload/custom",
            post(upload_custom).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/build/upload",
            post(upload_official).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/build/upload/tus",
            post(tus_handlers::create).options(tus_handlers::options),
        )
        .route(
            "/build/upload/tus/{id}",
            axum::routing::head(tus_handlers::head)
                .patch(tus_handlers::patch)
                .options(tus_handlers::options_resource),
        )
        .route("/build/{id}", get(build_by_id))
        .route("/build/{id}/flag", axum::routing::put(flag_build))
        .route("/build/{id}", axum::routing::delete(delete_build))
        .route("/test/export", get(export_tests))
        .route("/test/plan", get(test_plans))
        .route("/test/suite", get(test_suites))
        .route("/test/testcase", get(test_cases))
        .route(
            "/test/execution/{id}",
            get(test_execution).put(update_execution),
        )
        .route(
            "/test/execution",
            get(execution_list).post(create_execution),
        )
        .route("/test/execution/single/{testId}", get(single_execution))
        .route("/test/execution/list/{id}", get(execution_case_ids))
        .route("/test/execution/device/{id}", get(execution_by_device))
        .route("/test/execution/progress/list", get(execution_progress))
        .route("/test/execution/cases/{id}", get(execution_cases))
        .route("/test/testcase/{testCaseId}", get(single_testcase))
        .route("/test/execution/logs/{id}", get(execution_logs))
        .route("/test/execution/report/{id}", get(execution_report))
        .route(
            "/test/execution/report/{id}",
            axum::routing::put(create_report),
        )
        .route(
            "/test/execution/report/{id}/html",
            get(execution_report_html),
        )
        .route(
            "/test/execution/report/{id}/upload",
            axum::routing::put(upload_report),
        )
        .route("/test/execution/build/{buildId}", get(executions_by_build))
        .route("/test/execution/results/{id}", get(execution_results))
        .route("/test/cancel/{id}", axum::routing::put(cancel_execution))
        .route("/test/testcase/{testCaseId}/log", get(testcase_log))
        .route("/test/plan/{planID}/testcase", get(qmetry_plan_cases))
        .route(
            "/test/plan/{planID}/suite/{suiteID}/testcase",
            get(qmetry_suite_cases),
        )
        .route("/config/artifacts", axum::routing::put(configure_artifacts))
        .route(
            "/config/artifacts/default",
            axum::routing::put(copy_default_artifacts),
        )
        .route(
            "/relay/toggle/{device_id}",
            axum::routing::put(toggle_device_power),
        )
        .route("/relay/controller/{id}", get(relays_for_controller))
        .route("/relay/channels/relay/{id}", get(channels_for_relay))
        .route("/relay/devices/available", get(available_relay_devices))
        .route("/relay/state/{id}", get(relay_device_state))
        .route("/relay/configure", post(configure_relay_channels))
        .route("/relay/config", post(configure_legacy_relay))
        .route("/relay/config/fresh", post(fresh_legacy_relay))
        .route("/relay/config/remap", post(remap_legacy_relay))
        .route(
            "/relay/config/confirmation",
            post(confirm_relay_configuration),
        )
        .route("/relay/check-conflicts", get(legacy_relay_conflicts))
        .route("/relay", get(legacy_relays))
        .route("/relay/", get(legacy_relays))
        .route("/relay/channel", get(legacy_relay_channels))
        .route(
            "/relay/channel/{id}",
            get(legacy_relay_channel_by_id).delete(delete_legacy_relay_channel),
        )
        .route(
            "/relay/{id}",
            get(legacy_relay_by_id).delete(delete_legacy_relay),
        )
        .route(
            "/relay/{id}/identity",
            axum::routing::put(update_relay_identity),
        )
        .route("/user", get(list_user_devices))
        .route("/all/active", get(active_devices))
        .route(
            "/heartbeat/timeout",
            axum::routing::put(update_heartbeat_timeout),
        )
        .route("/topology", get(device_topology))
        .route("/{id}", get(device_by_id))
        .route("/{id}", axum::routing::delete(delete_device))
        .route("/{id}/action", axum::routing::put(device_action))
        .route("/{id}/builds", get(builds_for_device))
        .route("/{id}/heartbeat", get(latest_heartbeat))
        .route("/{id}/flashing", get(mark_device_flashing))
        .route("/{id}/test-completed", get(test_completed))
        .route("/{id}/reboot", get(reboot_device))
}
