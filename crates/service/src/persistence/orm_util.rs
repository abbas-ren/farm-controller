//! Shared SeaORM helpers used while retiring sqlx::query* call sites.

use sea_orm::{ConnectionTrait, DbErr, FromQueryResult, Statement};

/// Run a typed SELECT through the SeaORM connection (prefer Entity APIs when possible).
pub async fn query_all<C, T>(conn: &C, sql: &str, values: Vec<sea_orm::Value>) -> Result<Vec<T>, DbErr>
where
    C: ConnectionTrait,
    T: FromQueryResult,
{
    T::find_by_statement(Statement::from_sql_and_values(
        conn.get_database_backend(),
        sql,
        values,
    ))
    .all(conn)
    .await
}

pub async fn query_one<C, T>(
    conn: &C,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<Option<T>, DbErr>
where
    C: ConnectionTrait,
    T: FromQueryResult,
{
    T::find_by_statement(Statement::from_sql_and_values(
        conn.get_database_backend(),
        sql,
        values,
    ))
    .one(conn)
    .await
}

pub async fn execute<C>(
    conn: &C,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<u64, DbErr>
where
    C: ConnectionTrait,
{
    let result = conn
        .execute_raw(Statement::from_sql_and_values(
            conn.get_database_backend(),
            sql,
            values,
        ))
        .await?;
    Ok(result.rows_affected())
}
