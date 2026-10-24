//! Analytics HTTP handlers under `/api/v1/device/analytics/*`.
//!
//! Ports Node `analyticsRoutes.ts` / `AnalyticsController` from
//! `farm_controller_old/device-service`. Frontend dashboards call these authenticated GETs.

mod builds;
mod context;
mod devices;
mod executions;
mod tests;
mod usage;

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use chrono::{Datelike, Days, Duration, Months, NaiveDate, Utc};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{auth, error::ErrorResponse, state::AppState};

use super::{
    DeviceStateAnalytics, DeviceStateDetailedAnalytics, error_response,
    repository::DeviceRepository, repository_error,
};

pub(crate) use builds::*;
pub(crate) use context::{
    DeviceUsageQuery, ExecutionDailyQuery, RecentExecutionQuery, TestAnalyticsQuery, TestDailyQuery,
};
use context::{analytics_context, invalid_date_response};
pub(crate) use devices::*;
pub(crate) use executions::*;
pub(crate) use tests::*;
pub(crate) use usage::*;
