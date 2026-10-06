//! Postgres-backed device analytics queries grouped by domain capability.

mod builds;
mod executions;
mod state_usage;

pub(crate) use builds::{
    build_comparison_by_id, build_comparisons, build_performance, build_performance_by_id,
};
pub(crate) use executions::{
    daily_test_summary, execution_by_id, execution_daily, in_progress_test_analytics,
    recent_executions, test_execution_analytics, test_plan_summary,
};
pub(crate) use state_usage::{device_state_counts, device_state_details, device_usage};
