//! Construct execution and report events with the established client payloads.

use crate::events::ServerEvent;

pub(crate) fn new_execution_event(test_id: &str, user_id: &str) -> ServerEvent {
    ServerEvent {
        event: "test_execution".to_owned(),
        payload: serde_json::json!({
            "testId": test_id,
            "type": "new",
            "data": {
                "status": "not_executed",
                "message": format!("Test execution {test_id} created"),
                "timestamp": chrono::Utc::now().to_rfc3339(),
            },
        }),
        room: Some(format!("user:{user_id}")),
    }
}

pub(crate) fn report_update_events(
    report_id: uuid::Uuid,
    test_id: &str,
    user_id: &str,
    status: &str,
    error: Option<&str>,
    upload_error: Option<&str>,
) -> [ServerEvent; 2] {
    let mut payload = serde_json::json!({
        "reportId": report_id,
        "testExecutionId": test_id,
        "status": status,
        "createdBy": user_id,
    });
    if let Some(error) = error {
        payload["error"] = error.into();
    }
    if let Some(upload_error) = upload_error {
        payload["uploadError"] = upload_error.into();
    }
    [format!("user:{user_id}"), format!("test:{test_id}")].map(|room| ServerEvent {
        event: "execution_report_update".to_owned(),
        payload: payload.clone(),
        room: Some(room),
    })
}
