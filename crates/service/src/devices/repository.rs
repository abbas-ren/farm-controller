use async_trait::async_trait;
use uuid::Uuid;

use super::{
    AlertList, AvailableRelayDevicesQuery, BuildFilters, BuildFlagRequest, BuildList,
    BuildListQuery, CallbackStatus, ControllerEditRequest, ControllerHeartbeat, ControllerList,
    ControllerListQuery, ControllerRegistration, DeviceAction, DeviceCsvQuery, DeviceList,
    DeviceListQuery, DeviceRegistration, DeviceStateAnalytics, DeviceStateDetailedAnalytics,
    DeviceTypeFolder, ExecutionList, ExecutionListQuery, LegacyRelayChannelListQuery,
    LegacyRelayConfiguration, LegacyRelayConflictQuery, LegacyRelayListQuery, LogCreateRequest,
    LogListQuery, RelayChannelConfiguration, RelayConfirmationResponse, RelayConflictState,
    UserDeviceListQuery,
    error::DeviceRepositoryError,
    repository_types::{
        BuildDeleteTarget, BuildFlagResult, BuildUploadFinalization, BuildUploadResult,
        ControllerDeleteTarget, ControllerHeartbeatResult, DeviceActionTarget, DeviceDeleteTarget,
        DeviceFlashingResult, DeviceHeartbeatResult, DevicePowerTarget, DeviceRebootTarget,
        DeviceRegistrationResult, ExecutionCreation, FaultyReportCreate, FaultyReportCreation,
        FaultyReportFinalization, FlashConfirmationResult, RegistrationResult,
        RelayConfigurationResult, RelayIdentityTarget, RelayStateTarget, TestCancellationResult,
        TestCompletionTarget, TestExportRow,
    },
    types::TtyEntry,
};

#[async_trait]
pub(crate) trait DeviceRepository: Send + Sync {
    async fn create_test_execution(
        &self,
        execution: &ExecutionCreation,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn test_execution_build_version(
        &self,
        test_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError>;

    async fn create_log(
        &self,
        request: &LogCreateRequest,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;
    async fn list_logs(
        &self,
        query: &LogListQuery,
        search: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn executions_by_build(
        &self,
        build_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn test_results(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn request_test_cancellation(
        &self,
        test_id: &str,
        user_id: &str,
        nfs_host_path: &str,
    ) -> Result<Option<TestCancellationResult>, DeviceRepositoryError>;

    async fn report_generation_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError>;

    async fn create_execution_report_record(
        &self,
        report_id: Uuid,
        test_id: &str,
        device_type: &str,
        build_version: &str,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn update_execution_report_status(
        &self,
        report_id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DeviceRepositoryError>;

    async fn execution_report(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn execution_report_target(
        &self,
        test_id: &str,
    ) -> Result<Option<(String, String)>, DeviceRepositoryError>;

    async fn test_case_log_path(
        &self,
        case_id: i64,
    ) -> Result<Option<String>, DeviceRepositoryError>;

    async fn execution_cases(
        &self,
        user_id: &str,
        test_id: &str,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError>;

    async fn test_case(
        &self,
        case_id: i64,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn list_test_executions(
        &self,
        user_id: &str,
        query: &ExecutionListQuery,
    ) -> Result<ExecutionList, DeviceRepositoryError>;

    async fn test_execution_by_device(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn in_progress_test_executions(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn single_test_execution(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn execution_case_ids(
        &self,
        test_id: &str,
    ) -> Result<Option<Vec<String>>, DeviceRepositoryError>;

    async fn test_execution(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn test_execution_summary(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn test_export_rows(
        &self,
        user_id: &str,
    ) -> Result<Vec<TestExportRow>, DeviceRepositoryError>;

    async fn flag_build(
        &self,
        build_id: &str,
        request: &BuildFlagRequest,
    ) -> Result<Option<BuildFlagResult>, DeviceRepositoryError>;

    async fn delete_build(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildDeleteTarget>, DeviceRepositoryError>;

    async fn finalize_build_upload(
        &self,
        request: &BuildUploadFinalization,
    ) -> Result<BuildUploadResult, DeviceRepositoryError>;

    async fn init_build_upload(
        &self,
        file_count: i32,
        user_id: &str,
    ) -> Result<Uuid, DeviceRepositoryError>;

    async fn mark_build_upload_started(
        &self,
        upload_id: &str,
        user_id: &str,
    ) -> Result<(), DeviceRepositoryError>;

    async fn mark_build_upload_failed(&self, upload_id: &str) -> Result<(), DeviceRepositoryError>;

    async fn build_filters(&self) -> Result<BuildFilters, DeviceRepositoryError>;

    async fn build_by_id(
        &self,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn list_builds(&self, query: &BuildListQuery)
    -> Result<BuildList, DeviceRepositoryError>;

    async fn test_completion_target(
        &self,
        test_id: &str,
        device_id: &str,
    ) -> Result<Option<TestCompletionTarget>, DeviceRepositoryError>;

    async fn store_rtos_log_path(
        &self,
        test_id: &str,
        path: &str,
    ) -> Result<(), DeviceRepositoryError>;

    async fn mark_device_flashing(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceFlashingResult>, DeviceRepositoryError>;

    async fn export_devices(
        &self,
        query: &DeviceCsvQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn register_device(
        &self,
        registration: &DeviceRegistration,
    ) -> Result<DeviceRegistrationResult, DeviceRepositoryError>;

    async fn register_controller(
        &self,
        registration: &ControllerRegistration,
    ) -> Result<RegistrationResult, DeviceRepositoryError>;

    async fn save_gen5_mapping(
        &self,
        caller_ip: &str,
        mac: &str,
        status: CallbackStatus,
        tty_entry: Option<&TtyEntry>,
    ) -> Result<(), DeviceRepositoryError>;

    async fn confirm_flash(
        &self,
        device_type: &str,
        status: CallbackStatus,
    ) -> Result<FlashConfirmationResult, DeviceRepositoryError>;

    async fn device_families(&self) -> Result<Vec<String>, DeviceRepositoryError>;

    async fn device_types(
        &self,
        device_family: Option<&str>,
    ) -> Result<Vec<String>, DeviceRepositoryError>;

    async fn device_by_id(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn latest_heartbeat(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn topology(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn list_devices(
        &self,
        query: &DeviceListQuery,
        is_admin: bool,
    ) -> Result<DeviceList, DeviceRepositoryError>;

    async fn device_action_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceActionTarget>, DeviceRepositoryError>;

    async fn apply_device_action(
        &self,
        device_id: &str,
        action: DeviceAction,
        user_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn list_controllers(
        &self,
        query: &ControllerListQuery,
    ) -> Result<ControllerList, DeviceRepositoryError>;

    async fn list_user_devices(
        &self,
        query: &UserDeviceListQuery,
    ) -> Result<DeviceList, DeviceRepositoryError>;

    async fn active_devices(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn update_heartbeat_timeout(
        &self,
        device_id: &str,
        value: i64,
    ) -> Result<bool, DeviceRepositoryError>;

    async fn edit_controller(
        &self,
        controller_id: &str,
        request: &ControllerEditRequest,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn controller_delete_target(
        &self,
        controller_id: &str,
    ) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError>;

    async fn delete_controller(&self, controller_id: &str) -> Result<bool, DeviceRepositoryError>;

    async fn device_delete_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceDeleteTarget>, DeviceRepositoryError>;

    async fn delete_device(&self, device_id: &str) -> Result<bool, DeviceRepositoryError>;

    async fn builds_for_device_type(
        &self,
        device_type: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn configure_artifacts(
        &self,
        entries: &[DeviceTypeFolder],
    ) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError>;

    async fn artifact_folders(&self) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError>;

    async fn artifact_folder_for_device(
        &self,
        device_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError>;

    async fn device_reboot_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceRebootTarget>, DeviceRepositoryError>;

    async fn device_power_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePowerTarget>, DeviceRepositoryError>;

    async fn apply_device_power(
        &self,
        device_id: &str,
        channel_id: Option<Uuid>,
        state: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn relays_for_controller(
        &self,
        controller_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn channels_for_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<Vec<serde_json::Value>>, DeviceRepositoryError>;

    async fn available_relay_devices(
        &self,
        query: &AvailableRelayDevicesQuery,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn relay_state_target(
        &self,
        device_id: &str,
    ) -> Result<Option<RelayStateTarget>, DeviceRepositoryError>;

    async fn update_relay_state(
        &self,
        channel_id: Uuid,
        state: &str,
    ) -> Result<(), DeviceRepositoryError>;

    async fn relay_identity_target(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<RelayIdentityTarget>, DeviceRepositoryError>;

    async fn update_relay_identity(
        &self,
        relay_id: Uuid,
        old_serial: &str,
        serial_number: &str,
        vendor_id: &str,
        product_id: &str,
    ) -> Result<bool, DeviceRepositoryError>;

    async fn configure_relay_channels(
        &self,
        configurations: &[RelayChannelConfiguration],
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;

    async fn confirm_relay_configuration(
        &self,
        mac: &str,
        succeeded: bool,
    ) -> Result<RelayConfirmationResponse, DeviceRepositoryError>;

    async fn legacy_relays(
        &self,
        query: &LegacyRelayListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn legacy_relay_channels(
        &self,
        query: &LegacyRelayChannelListQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn legacy_relay_by_id(
        &self,
        relay_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn legacy_relay_channel_by_id(
        &self,
        channel_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn delete_legacy_relay(&self, relay_id: Uuid) -> Result<bool, DeviceRepositoryError>;

    async fn delete_legacy_relay_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<bool, DeviceRepositoryError>;

    async fn configure_legacy_relay(
        &self,
        request: &LegacyRelayConfiguration,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;

    async fn fresh_legacy_relay(&self, relay_id: Uuid) -> Result<(), DeviceRepositoryError>;

    async fn remap_legacy_relay(
        &self,
        relay_id: Uuid,
    ) -> Result<RelayConfigurationResult, DeviceRepositoryError>;

    async fn legacy_relay_conflicts(
        &self,
        query: &LegacyRelayConflictQuery,
    ) -> Result<RelayConflictState, DeviceRepositoryError>;

    async fn device_state_analytics(&self) -> Result<DeviceStateAnalytics, DeviceRepositoryError>;

    async fn detailed_device_state_analytics(
        &self,
    ) -> Result<DeviceStateDetailedAnalytics, DeviceRepositoryError>;

    async fn recent_execution_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn execution_analytics_by_id(
        &self,
        test_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn daily_execution_analytics(
        &self,
        user_id: &str,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn build_comparison_analytics(
        &self,
        user_id: &str,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn build_comparison_analytics_by_id(
        &self,
        user_id: &str,
        build_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn build_performance_analytics(
        &self,
        count: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn build_performance_analytics_by_id(
        &self,
        build_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn device_usage_analytics(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
        device_id: Option<&str>,
        device_family: Option<&str>,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn test_execution_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn in_progress_test_analytics(
        &self,
        user_id: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn test_plan_analytics(
        &self,
        user_id: &str,
    ) -> Result<serde_json::Value, DeviceRepositoryError>;

    async fn daily_test_analytics(
        &self,
        user_id: &str,
        days: i64,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn admin_alerts(
        &self,
        page: i64,
        limit: i64,
        sort_by: &str,
        descending: bool,
    ) -> Result<AlertList, DeviceRepositoryError>;

    async fn all_alerts(
        &self,
        page: i64,
        limit: i64,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<AlertList, DeviceRepositoryError>;

    async fn mark_alert_read(&self, alert_id: &str) -> Result<bool, DeviceRepositoryError>;

    async fn mark_all_alerts_read(&self) -> Result<(), DeviceRepositoryError>;

    async fn create_faulty_report(
        &self,
        request: &FaultyReportCreate,
    ) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError>;

    async fn finalize_faulty_report(
        &self,
        report_id: Uuid,
        file_path: Option<&str>,
        logs_path: Option<&str>,
    ) -> Result<FaultyReportFinalization, DeviceRepositoryError>;

    async fn faulty_reports(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError>;

    async fn faulty_report_by_id(
        &self,
        report_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn update_faulty_report_status(
        &self,
        report_id: Uuid,
        status: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError>;

    async fn delete_faulty_report(&self, report_id: Uuid) -> Result<bool, DeviceRepositoryError>;

    async fn record_controller_heartbeat(
        &self,
        heartbeat: &ControllerHeartbeat,
    ) -> Result<ControllerHeartbeatResult, DeviceRepositoryError>;

    async fn record_device_heartbeat(
        &self,
        device_id: &str,
        heartbeat: &serde_json::Value,
    ) -> Result<Option<DeviceHeartbeatResult>, DeviceRepositoryError>;
}
