use super::*;

#[utoipa::path(get, path = "/api/v1/device/analytics/usage/daily", tag = "Devices", summary = "Summarize device state durations", description = "Calculates approved-device state durations for one UTC day or the trailing 24 hours, with optional device and family filters.", params(DeviceUsageQuery), security(("bearer_auth" = [])), responses((status = 200, description = "Device state duration analytics"), (status = 400, description = "Invalid day", body = ErrorResponse), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn daily_device_usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DeviceUsageQuery>,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let (start, end) = if let Some(day) = query.day.as_deref() {
        let day = match NaiveDate::parse_from_str(day, "%Y-%m-%d") {
            Ok(day) => day,
            Err(_) => return invalid_date_response(),
        };
        day_bounds(day)
    } else {
        let end = Utc::now();
        (end - Duration::hours(24), end)
    };
    match context
        .repository
        .device_usage_analytics(
            start,
            end,
            query.device_id.as_deref(),
            query.device_family.as_deref(),
        )
        .await
    {
        Ok(usage) => (context.response_headers, Json(usage)).into_response(),
        Err(error) => repository_error(error, "Failed to fetch device usage analytics"),
    }
}

#[utoipa::path(get, path = "/api/v1/device/analytics/usage/summary", tag = "Devices", summary = "Summarize device usage by week and month", description = "Returns four UTC weekly periods and four UTC monthly periods of approved-device usage.", security(("bearer_auth" = [])), responses((status = 200, description = "Weekly and monthly device usage analytics"), (status = 401, description = "Authentication required", body = ErrorResponse), (status = 500, description = "Analytics query failed", body = ErrorResponse), (status = 503, description = "Device persistence unavailable", body = ErrorResponse)))]
pub(crate) async fn device_usage_summary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let context = match analytics_context(&state, &headers).await {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let today = Utc::now().date_naive();
    let current_week = today
        .checked_sub_days(Days::new(u64::from(today.weekday().num_days_from_sunday())))
        .unwrap_or(today);
    let current_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let mut weekly = Vec::with_capacity(4);
    let mut monthly = Vec::with_capacity(4);
    for offset in (0..4).rev() {
        let week_start = current_week
            .checked_sub_days(Days::new(offset * 7))
            .unwrap_or(current_week);
        let week_end = week_start
            .checked_add_days(Days::new(6))
            .unwrap_or(week_start);
        let (start, end) = day_range(week_start, week_end);
        let usage = match context
            .repository
            .device_usage_analytics(start, end, None, None)
            .await
        {
            Ok(usage) => usage,
            Err(error) => return repository_error(error, "Failed to fetch device usage summary"),
        };
        weekly.push(with_period(
            usage,
            "weekStart",
            week_start,
            "weekEnd",
            week_end,
        ));

        let month_start = current_month
            .checked_sub_months(Months::new(offset as u32))
            .unwrap_or(current_month);
        let month_end = month_start
            .checked_add_months(Months::new(1))
            .and_then(|next| next.checked_sub_days(Days::new(1)))
            .unwrap_or(month_start);
        let (start, end) = day_range(month_start, month_end);
        let usage = match context
            .repository
            .device_usage_analytics(start, end, None, None)
            .await
        {
            Ok(usage) => usage,
            Err(error) => return repository_error(error, "Failed to fetch device usage summary"),
        };
        monthly.push(with_period(
            usage,
            "monthStart",
            month_start,
            "monthEnd",
            month_end,
        ));
    }
    (
        context.response_headers,
        Json(serde_json::json!({"weekly": weekly, "monthly": monthly})),
    )
        .into_response()
}

fn day_bounds(day: NaiveDate) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    day_range(day, day)
}

fn day_range(start: NaiveDate, end: NaiveDate) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let start = start.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
    let end = end
        .checked_add_days(Days::new(1))
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .unwrap_or_default()
        .and_utc()
        - Duration::nanoseconds(1);
    (start, end)
}

fn with_period(
    mut usage: serde_json::Value,
    start_key: &str,
    start: NaiveDate,
    end_key: &str,
    end: NaiveDate,
) -> serde_json::Value {
    if let Some(object) = usage.as_object_mut() {
        object.insert(start_key.to_owned(), serde_json::json!(start.to_string()));
        object.insert(end_key.to_owned(), serde_json::json!(end.to_string()));
    }
    usage
}
