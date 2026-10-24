use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "enum_testcase_result"
)]
pub enum TestResult {
    #[sea_orm(string_value = "PASS")]
    Pass,
    #[sea_orm(string_value = "FAIL")]
    Fail,
}
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "testcase", rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub test_case_id: i64,
    pub title: String,
    pub execution_id: Option<String>,
    pub result: Option<TestResult>,
    pub comment: Option<String>,
    pub cmd: Option<String>,
    pub jira_defect: Option<String>,
    pub suite_id: i64,
    pub script_file: Option<String>,
    pub plan_id: i64,
    pub priority_id: Option<i32>,
    pub order: Option<i32>,
    pub suite_name: String,
    pub updated_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub started_at: Option<DateTimeUtc>,
    pub ended_at: Option<DateTimeUtc>,
    pub output_file_path: Option<String>,
    pub dmesg_file_path: Option<String>,
    pub labels: Option<Vec<String>>,
    pub pre_condition: Option<String>,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::test_execution::Entity",
        from = "Column::ExecutionId",
        to = "super::test_execution::Column::TestId"
    )]
    Execution,
}
impl Related<super::test_execution::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Execution.def()
    }
}
impl ActiveModelBehavior for ActiveModel {}
