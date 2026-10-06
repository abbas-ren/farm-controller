use sqlx::PgPool;
use uuid::Uuid;

use super::{
    error::DeviceRepositoryError,
    repository_types::{FaultyReportCreate, FaultyReportCreation, FaultyReportFinalization},
};

pub(crate) async fn create(
    pool: &PgPool,
    request: &FaultyReportCreate,
) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError> {
    let release_exists = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(SELECT 1 FROM releases WHERE id::text = $1)"#,
    )
    .bind(&request.release_id)
    .fetch_one(pool)
    .await?;
    if !release_exists {
        return Ok(None);
    }

    let last_test_execution_id = match request.test_execution_id.as_deref() {
        Some(test_id) => Some(test_id.to_owned()),
        None => {
            let latest = sqlx::query_scalar::<_, String>(
                r#"SELECT "testId" FROM test_executions
                   WHERE "buildId"::text = $1 AND "deviceFamily" = $2
                   ORDER BY "createdAt" DESC LIMIT 1"#,
            )
            .bind(&request.release_id)
            .bind(&request.device_family)
            .fetch_optional(pool)
            .await?;
            match latest {
                Some(test_id) => Some(test_id),
                None => {
                    sqlx::query_scalar::<_, String>(
                        r#"SELECT "lastTestExecution" FROM devices
                           WHERE "deviceType" = $1 AND "deviceFamily" = $2
                             AND "lastTestExecution" IS NOT NULL
                           ORDER BY "createdAt" DESC LIMIT 1"#,
                    )
                    .bind(&request.device_type)
                    .bind(&request.device_family)
                    .fetch_optional(pool)
                    .await?
                }
            }
        }
    };

    sqlx::query(
        r#"INSERT INTO faulty_reports
              (id, "releaseId", "deviceType", "deviceFamily", "lastTestExecutionId",
               description, "filePath", "logsPath", status, "createdBy", "createdAt", "updatedAt")
           VALUES ($1, $2::uuid, $3, $4, $5, $6, NULL, NULL, 'pending', $7, now(), now())"#,
    )
    .bind(request.id)
    .bind(&request.release_id)
    .bind(&request.device_type)
    .bind(&request.device_family)
    .bind(&last_test_execution_id)
    .bind(&request.description)
    .bind(&request.created_by)
    .execute(pool)
    .await?;

    Ok(Some(FaultyReportCreation {
        last_test_execution_id,
    }))
}

pub(crate) async fn finalize(
    pool: &PgPool,
    report_id: Uuid,
    file_path: Option<&str>,
    logs_path: Option<&str>,
) -> Result<FaultyReportFinalization, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let report = sqlx::query_scalar::<_, serde_json::Value>(
        r#"UPDATE faulty_reports
           SET "filePath" = $2, "logsPath" = $3, "updatedAt" = now()
           WHERE id = $1
           RETURNING to_jsonb(faulty_reports)"#,
    )
    .bind(report_id)
    .bind(file_path)
    .bind(logs_path)
    .fetch_one(&mut *transaction)
    .await?;
    let alert = sqlx::query_scalar::<_, serde_json::Value>(
        r#"WITH inserted AS (
               INSERT INTO alerts
                  (id, user_id, title, message, type, status, is_read, faulty_report_id,
                   build_id, data, created_at, updated_at)
               SELECT gen_random_uuid(), 'ADMIN', 'Faulty Report',
                      'New faulty report submitted for release ' || fr."releaseId"::text,
                      'warning', 'unread', false, fr.id::text, fr."releaseId"::text,
                      jsonb_build_object(
                          'faultyReportId', fr.id,
                          'buildId', fr."releaseId",
                          'buildVersion', r.version,
                          'deviceType', fr."deviceType",
                          'deviceFamily', fr."deviceFamily",
                          'createdBy', fr."createdBy"
                      ), now(), now()
               FROM faulty_reports fr
               JOIN releases r ON r.id = fr."releaseId"
               WHERE fr.id = $1
               RETURNING *
           )
           SELECT jsonb_build_object(
               'id', a.id, 'userId', a.user_id, 'title', a.title, 'message', a.message,
               'type', a.type::text, 'status', a.status::text, 'isRead', a.is_read,
               'faultyReportId', a.faulty_report_id, 'buildId', a.build_id,
               'buildVersion', a.data->>'buildVersion', 'data', a.data,
               'createdAt', a.created_at)
           FROM inserted a"#,
    )
    .bind(report_id)
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(FaultyReportFinalization { report, alert })
}

pub(crate) async fn list(pool: &PgPool) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT to_jsonb(fr) FROM faulty_reports fr ORDER BY fr."createdAt" DESC"#,
    )
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn by_id(
    pool: &PgPool,
    report_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    Ok(sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT jsonb_build_object(
               'report', to_jsonb(fr),
               'buildVersion', COALESCE(r.version, 'Unknown'))
           FROM faulty_reports fr
           LEFT JOIN releases r ON r.id = fr."releaseId"
           WHERE fr.id = $1"#,
    )
    .bind(report_id)
    .fetch_optional(pool)
    .await?)
}

pub(crate) async fn update_status(
    pool: &PgPool,
    report_id: Uuid,
    status: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let mut transaction = pool.begin().await?;
    let report = sqlx::query_scalar::<_, serde_json::Value>(
        r#"UPDATE faulty_reports
              SET status = $2::"enum_faulty_reports_status", "updatedAt" = now()
           WHERE id = $1
           RETURNING to_jsonb(faulty_reports)"#,
    )
    .bind(report_id)
    .bind(status)
    .fetch_optional(&mut *transaction)
    .await?;
    if status == "approved" {
        sqlx::query(
            r#"UPDATE releases SET "isFaulty" = true, "updatedAt" = now()
               WHERE id = (SELECT "releaseId" FROM faulty_reports WHERE id = $1)"#,
        )
        .bind(report_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(report)
}

pub(crate) async fn delete(pool: &PgPool, report_id: Uuid) -> Result<bool, DeviceRepositoryError> {
    Ok(sqlx::query(r#"DELETE FROM faulty_reports WHERE id = $1"#)
        .bind(report_id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}
