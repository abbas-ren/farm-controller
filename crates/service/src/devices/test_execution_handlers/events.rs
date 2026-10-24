//! Construct execution and report events with the established client payloads.

use crate::{
    reports::ReportStatus,
    socket::types::{
        farm::ServerEvent,
        frontend::{
            FrontendMessagePayload, TestExecutionUpdateData, TestExecutionUpdateType, TestStatus,
        },
    },
};

pub(crate) fn new_execution_event(test_id: &str, _user_id: &str) -> ServerEvent {
    let _api_trace = crate::observability::ApiTrace::enter("devices::test_execution_handlers::events", "new_execution_event");
    ServerEvent::frontend(FrontendMessagePayload::TestExecution {
        test_id: test_id.to_owned(),
        r#type: TestExecutionUpdateType::New,
        data: TestExecutionUpdateData {
            status: Some(TestStatus::NotExecuted),
            timestamp: chrono::Utc::now().to_rfc3339(),
            cancel_requested: None,
            phase: None,
            rtos_log_path: None,
            message: Some(format!("Test execution {test_id} created")),
        },
    })
}

pub(crate) fn report_update_events(
    _report_id: uuid::Uuid,
    test_id: &str,
    _user_id: &str,
    status: &str,
    error: Option<&str>,
    upload_error: Option<&str>,
) -> [ServerEvent; 1] {
    [ServerEvent::frontend(
        FrontendMessagePayload::ExecutionReportUpdate {
            test_execution_id: Some(test_id.to_owned()),
            status: Some(ReportStatus::from(status)),
            error: error.map(|e| e.to_owned()),
            upload_error: upload_error.map(|e| e.to_owned()),
        },
    )]
}
