use super::*;

#[async_trait]
impl FaultyReportRepository for RegistrationRepository {
    async fn create_faulty_report(
        &self,
        request: &FaultyReportCreate,
    ) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError> {
        if request.release_id == "missing" {
            return Ok(None);
        }
        Ok(Some(FaultyReportCreation {
            last_test_execution_id: request.test_execution_id.clone(),
        }))
    }

    async fn finalize_faulty_report(
        &self,
        report_id: Uuid,
        file_path: Option<&str>,
        logs_path: Option<&str>,
    ) -> Result<repository_types::FaultyReportFinalization, DeviceRepositoryError> {
        Ok(repository_types::FaultyReportFinalization {
            report: serde_json::json!({
                "id": report_id,
                "releaseId": "11111111-1111-4111-8111-111111111111",
                "deviceType": "Racer",
                "deviceFamily": "Gen5",
                "description": "Intermittent failure",
                "filePath": file_path,
                "logsPath": logs_path,
                "status": "pending",
                "createdBy": "user-1",
            }),
            alert: serde_json::json!({
                "id": "alert-1",
                "userId": "ADMIN",
                "title": "Faulty Report",
                "message": "New faulty report submitted for release 11111111-1111-4111-8111-111111111111",
                "type": "warning",
                "status": "unread",
                "isRead": false,
                "faultyReportId": report_id,
                "buildId": "11111111-1111-4111-8111-111111111111",
                "buildVersion": "v1",
                "data": {"faultyReportId": report_id},
                "createdAt": "2026-10-02T12:00:00Z"
            }),
        })
    }

    async fn faulty_reports(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "id": "11111111-1111-4111-8111-111111111111",
            "status": "pending",
            "createdBy": "user-1",
        })])
    }

    async fn faulty_report_by_id(
        &self,
        report_id: Uuid,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let file_path = self
            .callbacks
            .lock()
            .unwrap()
            .iter()
            .find_map(|value| value.strip_prefix("faulty-file:").map(str::to_owned));
        Ok((report_id != Uuid::nil()).then(|| {
            serde_json::json!({
                "report": {
                    "id": report_id,
                    "releaseId": "11111111-1111-4111-8111-111111111111",
                    "deviceType": "Racer",
                    "deviceFamily": "Gen5",
                    "description": "Intermittent failure",
                    "filePath": file_path,
                    "logsPath": null,
                    "status": "pending",
                    "createdBy": "user-1",
                },
                "buildVersion": "v1",
            })
        }))
    }

    async fn update_faulty_report_status(
        &self,
        report_id: Uuid,
        status: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((report_id != Uuid::nil()).then(|| {
            serde_json::json!({
                "id": report_id,
                "status": status,
            })
        }))
    }

    async fn delete_faulty_report(&self, report_id: Uuid) -> Result<bool, DeviceRepositoryError> {
        Ok(report_id != Uuid::nil())
    }
}
