use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseBackend, DatabaseConnection, EntityTrait,
    FromQueryResult, QueryFilter, QueryOrder, QuerySelect, Set, Statement, TransactionTrait,
    sea_query::Expr,
};
use uuid::Uuid;

use crate::persistence::entity::{device, faulty_report, release, test_execution};

use super::{
    error::DeviceRepositoryError,
    repository_types::{FaultyReportCreate, FaultyReportCreation, FaultyReportFinalization},
};

#[derive(Debug, FromQueryResult)]
struct JsonRow {
    value: serde_json::Value,
}

pub(crate) async fn create(
    orm: &DatabaseConnection,
    request: &FaultyReportCreate,
) -> Result<Option<FaultyReportCreation>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_store", "create");
    let Ok(release_id) = Uuid::parse_str(&request.release_id) else {
        return Ok(None);
    };
    let release_exists = release::Entity::find_by_id(release_id).one(orm).await?.is_some();
    if !release_exists {
        return Ok(None);
    }

    let last_test_execution_id = match request.test_execution_id.as_deref() {
        Some(test_id) => Some(test_id.to_owned()),
        None => {
            let latest = test_execution::Entity::find()
                .filter(test_execution::Column::BuildId.eq(&request.release_id))
                .filter(test_execution::Column::DeviceFamily.eq(&request.device_family))
                .order_by_desc(test_execution::Column::CreatedAt)
                .select_only()
                .column(test_execution::Column::TestId)
                .into_tuple::<String>()
                .one(orm)
                .await?;
            match latest {
                Some(test_id) => Some(test_id),
                None => {
                    device::Entity::find()
                        .filter(device::Column::DeviceType.eq(&request.device_type))
                        .filter(device::Column::DeviceFamily.eq(&request.device_family))
                        .filter(device::Column::LastTestExecution.is_not_null())
                        .order_by_desc(device::Column::CreatedAt)
                        .select_only()
                        .column(device::Column::LastTestExecution)
                        .into_tuple::<Option<String>>()
                        .one(orm)
                        .await?
                        .flatten()
                }
            }
        }
    };

    let now = chrono::Utc::now();
    faulty_report::ActiveModel {
        id: Set(request.id),
        release_id: Set(release_id),
        device_type: Set(request.device_type.clone()),
        device_family: Set(request.device_family.clone()),
        last_test_execution_id: Set(last_test_execution_id.clone()),
        description: Set(request.description.clone()),
        file_path: Set(None),
        logs_path: Set(None),
        status: Set(faulty_report::FaultyStatus::Pending),
        created_by: Set(request.created_by.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(orm)
    .await?;

    Ok(Some(FaultyReportCreation {
        last_test_execution_id,
    }))
}

pub(crate) async fn finalize(
    orm: &DatabaseConnection,
    report_id: Uuid,
    file_path: Option<&str>,
    logs_path: Option<&str>,
) -> Result<FaultyReportFinalization, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::faulty_report_store", "finalize");
    let transaction = orm.begin().await?;
    let now = chrono::Utc::now();
    faulty_report::Entity::update_many()
        .col_expr(
            faulty_report::Column::FilePath,
            Expr::value(file_path.map(str::to_owned)),
        )
        .col_expr(
            faulty_report::Column::LogsPath,
            Expr::value(logs_path.map(str::to_owned)),
        )
        .col_expr(faulty_report::Column::UpdatedAt, Expr::value(now))
        .filter(faulty_report::Column::Id.eq(report_id))
        .exec(&transaction)
        .await?;

    let report = JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(faulty_reports) AS value FROM faulty_reports WHERE id = $1"#,
        [report_id.into()],
    ))
    .one(&transaction)
    .await?
    .ok_or_else(|| DeviceRepositoryError::NotFound("Faulty report not found".to_owned()))?
    .value;

    // Nested alert insert + jsonb response shape kept as raw SQL.
    let alert = JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
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
               'createdAt', a.created_at) AS value
           FROM inserted a"#,
        [report_id.into()],
    ))
    .one(&transaction)
    .await?
    .ok_or_else(|| DeviceRepositoryError::Internal("failed to create faulty report alert".to_owned()))?
    .value;
    transaction.commit().await?;
    Ok(FaultyReportFinalization { report, alert })
}

pub(crate) async fn list(orm: &DatabaseConnection) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_store", "list");
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(fr) AS value FROM faulty_reports fr ORDER BY fr."createdAt" DESC"#,
        [],
    ))
    .all(orm)
    .await?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

pub(crate) async fn by_id(
    orm: &DatabaseConnection,
    report_id: Uuid,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_store", "by_id");
    Ok(JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT jsonb_build_object(
               'report', to_jsonb(fr),
               'buildVersion', COALESCE(r.version, 'Unknown')) AS value
           FROM faulty_reports fr
           LEFT JOIN releases r ON r.id = fr."releaseId"
           WHERE fr.id = $1"#,
        [report_id.into()],
    ))
    .one(orm)
    .await?
    .map(|row| row.value))
}

pub(crate) async fn update_status(
    orm: &DatabaseConnection,
    report_id: Uuid,
    status: &str,
) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
    let _api_trace =
        crate::observability::ApiTrace::enter("devices::faulty_report_store", "update_status");
    let status_enum = match status {
        "pending" => faulty_report::FaultyStatus::Pending,
        "approved" => faulty_report::FaultyStatus::Approved,
        "rejected" => faulty_report::FaultyStatus::Rejected,
        _ => {
            return Err(DeviceRepositoryError::Validation(format!(
                "Invalid faulty report status: {status}"
            )));
        }
    };
    let transaction = orm.begin().await?;
    let now = chrono::Utc::now();
    let updated = faulty_report::Entity::update_many()
        .col_expr(faulty_report::Column::Status, Expr::value(status_enum))
        .col_expr(faulty_report::Column::UpdatedAt, Expr::value(now))
        .filter(faulty_report::Column::Id.eq(report_id))
        .exec(&transaction)
        .await?;
    if updated.rows_affected == 0 {
        transaction.commit().await?;
        return Ok(None);
    }
    if status == "approved" {
        let report = faulty_report::Entity::find_by_id(report_id)
            .one(&transaction)
            .await?
            .ok_or_else(|| DeviceRepositoryError::NotFound("Faulty report not found".to_owned()))?;
        release::Entity::update_many()
            .col_expr(release::Column::IsFaulty, Expr::value(Some(true)))
            .col_expr(release::Column::UpdatedAt, Expr::value(now))
            .filter(release::Column::Id.eq(report.release_id))
            .exec(&transaction)
            .await?;
    }
    let report = JsonRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"SELECT to_jsonb(faulty_reports) AS value FROM faulty_reports WHERE id = $1"#,
        [report_id.into()],
    ))
    .one(&transaction)
    .await?
    .map(|row| row.value);
    transaction.commit().await?;
    Ok(report)
}

pub(crate) async fn delete(orm: &DatabaseConnection, report_id: Uuid) -> Result<bool, DeviceRepositoryError> {
    let _api_trace = crate::observability::ApiTrace::enter("devices::faulty_report_store", "delete");
    let result = faulty_report::Entity::delete_by_id(report_id).exec(orm).await?;
    Ok(result.rows_affected > 0)
}
