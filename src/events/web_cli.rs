use std::{collections::BTreeMap, sync::Arc, time::Duration};

use axum::extract::ws::{Message, WebSocket};

use crate::{
    devices::{
        BuildListQuery, CreateExecutionRequest, ExecutionSelection, ExecutionSuiteSelection,
    },
    state::AppState,
};

const HELP: &str = "\
\x1b[1mNAME\x1b[0m\r\n    racer - RACER WebCLI\r\n\r\n\
\x1b[1mCOMMANDS\x1b[0m\r\n\
    racer-list-devices          List all devices with full details\r\n\
    racer-list-devices-summary  List devices (ID, Name, Status, State)\r\n\
    racer-available-devices     List only available devices\r\n\
    racer-device-stats          Show device statistics by type\r\n\
    racer-get-device <id>       Get device details by ID\r\n\
    racer-list-builds [family] [type] [-l N] [-f]\r\n\
    racer-list-test-plans [-f filter]\r\n\
    racer-list-test-suites <planId>\r\n\
    racer-list-test-cases <planId> <suiteId> [-f filter]\r\n\
    racer-run-test [family] [type] [buildId] [planId] [--all|--suites ids|--cases spec] [--logs]\r\n\
    racer-clear                 Clear the terminal screen\r\n\
    racer-help                  Show this help message";

pub(super) async fn session(
    state: Arc<AppState>,
    user_id: String,
    user_name: String,
    mut socket: WebSocket,
) {
    let banner = format!(
        "\x1b[1mRACER Management Console v1.0\x1b[0m\r\n\
         \x1b[2mHardware Testing & Automation\x1b[0m\r\n\r\n\
         \x1b[2mLogged in as:\x1b[0m \x1b[33m{user_name}\x1b[0m\r\n\
         \x1b[2mType \x1b[33mracer-help\x1b[0m\x1b[2m for commands | \
         \x1b[33mracer-clear\x1b[0m\x1b[2m to clear screen\x1b[0m"
    );
    if send_cli(&mut socket, &banner).await.is_err()
        || send_cli(&mut socket, "\r\n ").await.is_err()
    {
        return;
    }
    let mut events = state.event_publisher.subscribe();
    loop {
        tokio::select! {
            message = socket.recv() => {
                let Some(message) = message else { break };
                match message {
                    Ok(Message::Text(text)) => {
                        let Ok(message) = serde_json::from_str::<serde_json::Value>(&text) else {
                            if send_cli(&mut socket, "\x1b[31m Error: Invalid CLI message.\x1b[0m").await.is_err() { break; }
                            continue;
                        };
                        if message.get("type").and_then(serde_json::Value::as_str) != Some("cli") {
                            continue;
                        }
                        let command = message
                            .get("command")
                            .and_then(serde_json::Value::as_str)
                            .map(str::trim)
                            .unwrap_or_default();
                        let output = if command == "racer-run-test"
                            || command.starts_with("racer-run-test ")
                        {
                            run_test(&state, &user_id, command, &mut socket).await
                        } else {
                            execute(&state, command).await
                        };
                        if send_cli(&mut socket, &output).await.is_err()
                            || send_cli(&mut socket, "\r\n ").await.is_err()
                        {
                            break;
                        }
                    }
                    Ok(Message::Ping(bytes)) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() { break; }
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
            event = events.recv() => {
                let Ok(event) = event else { continue };
                if event.room.as_deref() != Some(&format!("user:{user_id}"))
                    || event.event != "test_execution_update"
                {
                    continue;
                }
                let mut payload = event.payload;
                if let Some(object) = payload.as_object_mut() {
                    object.insert("type".to_owned(), serde_json::Value::String("test_execution".to_owned()));
                }
                if socket.send(Message::Text(payload.to_string().into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn run_test(
    state: &AppState,
    user_id: &str,
    command: &str,
    socket: &mut WebSocket,
) -> String {
    let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
    let mut parsed = match parse_run_test_args(&args) {
        Ok(parsed) => parsed,
        Err(error) => return cli_error(error),
    };
    if parsed.family.is_none() {
        parsed.family = prompt_string(
            socket,
            "select",
            "Select device family:",
            Some(vec![
                choice("Gen3", "Gen3"),
                choice("Gen4", "Gen4"),
                choice("Gen5", "Gen5"),
            ]),
        )
        .await
        .ok();
    }
    let Some(family) = parsed.family.clone() else {
        return cancelled();
    };
    if parsed.device_type.is_none() {
        let Some(database) = &state.database else {
            return cli_error("device persistence is unavailable");
        };
        let types = match sqlx::query_scalar::<_, String>(
            r#"SELECT DISTINCT "deviceType" FROM devices
               WHERE "deletedAt" IS NULL AND status::text = 'approved'
                 AND "deviceFamily" = $1 AND "deviceType" IS NOT NULL
               ORDER BY "deviceType""#,
        )
        .bind(&family)
        .fetch_all(database.pool())
        .await
        {
            Ok(types) => types,
            Err(error) => return cli_error(error),
        };
        if types.is_empty() {
            return cli_error(format!("No devices found for family: {family}"));
        }
        parsed.device_type = if types.len() == 1 {
            types.into_iter().next()
        } else {
            prompt_string(
                socket,
                "select",
                "Select device type:",
                Some(
                    types
                        .into_iter()
                        .map(|value| choice(&value, &value))
                        .collect(),
                ),
            )
            .await
            .ok()
        };
    }
    let Some(device_type) = parsed.device_type.clone() else {
        return cancelled();
    };
    if parsed.build_id.is_none() {
        let Some(database) = &state.database else {
            return cli_error("device persistence is unavailable");
        };
        let query = BuildListQuery {
            device_family: Some(family.clone()),
            device_type: Some(device_type.clone()),
            flagged: Some("false".to_owned()),
            page: Some(1),
            limit: Some(20),
            sort_by: Some("createdAt".to_owned()),
            sort_order: Some("desc".to_owned()),
            ..BuildListQuery::default()
        };
        let builds = match crate::devices::build_store::list_builds(database.pool(), &query).await {
            Ok(builds) => builds.builds,
            Err(error) => return cli_error(error),
        };
        if builds.is_empty() {
            return cli_error(format!("No active builds found for {family} {device_type}"));
        }
        let choices = builds
            .iter()
            .filter_map(|build| {
                let id = build.get("id")?.as_str()?;
                let version = build
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(id);
                let tag = build
                    .get("tag")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("official");
                Some(choice(&format!("{version} - {tag}"), id))
            })
            .collect();
        parsed.build_id = prompt_string(socket, "select", "Select build:", Some(choices))
            .await
            .ok();
    }
    let Some(build_id) = parsed.build_id.clone() else {
        return cancelled();
    };
    let Some(catalog) = &state.test_catalog else {
        return cli_error("TestRail is not configured");
    };
    let plans = match catalog.plans(None).await {
        Ok(plans) => plans,
        Err(error) => return cli_error(error),
    };
    if plans.is_empty() {
        return cli_error("No test plans are available");
    }
    if parsed.plan_id.is_none() {
        let choices = plans
            .iter()
            .map(|plan| choice(&plan.name, &plan.id.to_string()))
            .collect();
        parsed.plan_id = prompt_string(socket, "select", "Select test plan:", Some(choices))
            .await
            .ok()
            .and_then(|value| value.parse().ok());
    }
    let Some(plan_id) = parsed.plan_id else {
        return cancelled();
    };
    if !plans.iter().any(|plan| plan.id == plan_id) {
        return cli_error(format!("Test plan {plan_id} was not found"));
    }
    let plan_name = plans
        .iter()
        .find(|plan| plan.id == plan_id)
        .map(|plan| plan.name.clone())
        .unwrap_or_else(|| format!("Plan #{plan_id}"));
    if parsed.selection.is_none() {
        let scope = match prompt_string(
            socket,
            "select",
            "Select test scope:",
            Some(vec![
                choice("All tests", "all"),
                choice("Specific suites", "suites"),
                choice("Specific cases", "cases"),
            ]),
        )
        .await
        {
            Ok(scope) => scope,
            Err(_) => return cancelled(),
        };
        parsed.selection = match interactive_selection(socket, catalog, plan_id, &scope).await {
            Ok(selection) => Some(selection),
            Err(error) if error == "cancelled" => return cancelled(),
            Err(error) => return cli_error(error),
        };
    }
    let Some(selection) = parsed.selection else {
        return cli_error("select tests with --all, --suites, or --cases");
    };
    let summary = format!(
        "\r\n\x1b[1mTest Execution Summary\x1b[0m\r\n{}\r\nDevice Family:    {family}\r\nDevice Type:      {device_type}\r\nBuild ID:         {build_id}\r\nTest Plan:        {plan_name}\r\n",
        "=".repeat(60)
    );
    if send_cli(socket, &summary).await.is_err() {
        return cancelled();
    }
    let confirmed =
        match prompt_value(socket, "confirm", "Proceed with test execution?", None).await {
            Ok(value) => value.as_bool().unwrap_or(false),
            Err(_) => return cancelled(),
        };
    if !confirmed {
        return cancelled();
    }
    let request = CreateExecutionRequest {
        name: None,
        device_family: family,
        test_plan_name: Some(plan_name),
        device_type,
        device_id: None,
        build_id,
        test_queue_id: None,
        test_plan_id: Some(plan_id as i64),
        test_suites: Vec::new(),
        is_all_selected: matches!(selection, ExecutionSelection::All { .. }),
        logs: None,
        created_by: Some(user_id.to_owned()),
        updated_by: Some(user_id.to_owned()),
        test_cases: BTreeMap::new(),
        selection: Some(selection),
    };
    match crate::devices::test_execution_handlers::create_for_user(
        state,
        request,
        user_id.to_owned(),
    )
    .await
    {
        Ok(created) => {
            let Some(test_id) = created
                .get("testId")
                .or_else(|| created.get("id"))
                .and_then(serde_json::Value::as_str)
            else {
                return cli_error("execution response did not include a test id");
            };
            let message_type = if parsed.logs {
                "start_log_capture"
            } else {
                "test_execution_link"
            };
            if send_json(
                socket,
                serde_json::json!({"type": message_type, "testId": test_id}),
            )
            .await
            .is_err()
            {
                return cancelled();
            }
            format!(
                "\r\n\x1b[32m\x1b[1mTest execution started successfully\x1b[0m\r\n  Test ID: {test_id}\r\n  Status: {}",
                created
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("not_executed")
            )
        }
        Err(error) => cli_error(format!("Failed to execute test: {error}")),
    }
}

async fn interactive_selection(
    socket: &mut WebSocket,
    catalog: &Arc<dyn crate::test_catalog::TestCatalog>,
    plan_id: u64,
    scope: &str,
) -> Result<ExecutionSelection, String> {
    if scope == "all" {
        return Ok(all_selection(plan_id));
    }
    let suites = catalog
        .suites(plan_id)
        .await
        .map_err(|error| error.to_string())?;
    if suites.is_empty() {
        return Err("No test suites found for this plan".to_owned());
    }
    if scope == "suites" {
        let choices = suites
            .iter()
            .map(|suite| choice(&suite.name, &suite.id.to_string()))
            .collect();
        let values =
            prompt_value(socket, "multi_select", "Select test suites:", Some(choices)).await?;
        let selected = value_strings(&values)?;
        if selected.is_empty() {
            return Err("At least one suite is required".to_owned());
        }
        return Ok(partial_selection(
            plan_id,
            selected
                .into_iter()
                .map(|suite_id| ExecutionSuiteSelection {
                    suite_id,
                    select_all: true,
                    cases: None,
                    exclude_cases: None,
                })
                .collect(),
        ));
    }
    let suite_id = prompt_string(
        socket,
        "select",
        "Select test suite:",
        Some(
            suites
                .iter()
                .map(|suite| choice(&suite.name, &suite.id.to_string()))
                .collect(),
        ),
    )
    .await?;
    let suite_id_number =
        positive_id(Some(&suite_id)).ok_or_else(|| "Invalid suite id".to_owned())?;
    let cases = catalog
        .cases(plan_id, suite_id_number, None)
        .await
        .map_err(|error| error.to_string())?;
    if cases.is_empty() {
        return Err("No test cases found for this suite".to_owned());
    }
    let values = prompt_value(
        socket,
        "multi_select",
        "Select test cases:",
        Some(
            cases
                .iter()
                .map(|case| choice(&case.title, &case.id.to_string()))
                .collect(),
        ),
    )
    .await?;
    let selected = value_strings(&values)?;
    if selected.is_empty() {
        return Err("At least one test case is required".to_owned());
    }
    Ok(partial_selection(
        plan_id,
        vec![ExecutionSuiteSelection {
            suite_id,
            select_all: false,
            cases: Some(selected),
            exclude_cases: None,
        }],
    ))
}

async fn prompt_string(
    socket: &mut WebSocket,
    prompt_type: &str,
    message: &str,
    choices: Option<Vec<serde_json::Value>>,
) -> Result<String, String> {
    let value = prompt_value(socket, prompt_type, message, choices).await?;
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
        .ok_or_else(|| "Prompt returned an invalid value".to_owned())
}

async fn prompt_value(
    socket: &mut WebSocket,
    prompt_type: &str,
    message: &str,
    choices: Option<Vec<serde_json::Value>>,
) -> Result<serde_json::Value, String> {
    let id = format!("prompt-{}", uuid::Uuid::new_v4().simple());
    send_cli(socket, &format!("\r\n{message}"))
        .await
        .map_err(|error| error.to_string())?;
    let mut data = serde_json::json!({"id": id, "type": prompt_type, "message": message});
    if let Some(choices) = choices {
        data["choices"] = serde_json::Value::Array(choices);
        if prompt_type == "multi_select" {
            data["min"] = serde_json::json!(1);
        }
    }
    if prompt_type == "confirm" {
        data["default"] = serde_json::json!(true);
    }
    send_json(socket, serde_json::json!({"type": "prompt", "data": data}))
        .await
        .map_err(|error| error.to_string())?;
    tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            let message = socket.recv().await.ok_or_else(|| "cancelled".to_owned())?;
            match message {
                Ok(Message::Text(text)) => {
                    let value: serde_json::Value = serde_json::from_str(&text)
                        .map_err(|_| "Invalid prompt response".to_owned())?;
                    if value.get("type").and_then(serde_json::Value::as_str)
                        != Some("prompt_response")
                        || value
                            .pointer("/data/id")
                            .and_then(serde_json::Value::as_str)
                            != Some(&id)
                    {
                        continue;
                    }
                    if value
                        .pointer("/data/cancelled")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                    {
                        return Err("cancelled".to_owned());
                    }
                    return value
                        .pointer("/data/value")
                        .cloned()
                        .ok_or_else(|| "Prompt response is missing a value".to_owned());
                }
                Ok(Message::Ping(bytes)) => socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|error| error.to_string())?,
                Ok(Message::Close(_)) | Err(_) => return Err("cancelled".to_owned()),
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| "Prompt timeout".to_owned())?
}

fn choice(label: &str, value: &str) -> serde_json::Value {
    serde_json::json!({"label": label, "value": value})
}

fn value_strings(value: &serde_json::Value) -> Result<Vec<String>, String> {
    value
        .as_array()
        .ok_or_else(|| "Prompt returned an invalid selection".to_owned())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_u64().map(|value| value.to_string()))
                .ok_or_else(|| "Prompt returned an invalid selection".to_owned())
        })
        .collect()
}

fn cancelled() -> String {
    "\x1b[90mTest execution cancelled\x1b[0m".to_owned()
}

fn all_selection(plan_id: u64) -> ExecutionSelection {
    ExecutionSelection::All {
        plan_id: plan_id.to_string(),
        plan_name: None,
        exclude: None,
    }
}

fn partial_selection(plan_id: u64, suites: Vec<ExecutionSuiteSelection>) -> ExecutionSelection {
    ExecutionSelection::Partial {
        plan_id: plan_id.to_string(),
        plan_name: None,
        suites,
    }
}

struct RunTestArgs {
    family: Option<String>,
    device_type: Option<String>,
    build_id: Option<String>,
    plan_id: Option<u64>,
    selection: Option<ExecutionSelection>,
    logs: bool,
}

fn parse_run_test_args(args: &[&str]) -> Result<RunTestArgs, String> {
    let value_flags = [
        "--suites",
        "-s",
        "--cases",
        "-c",
        "--exclude-suites",
        "--exclude-cases",
    ];
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if value_flags.contains(&args[index]) {
            index += 2;
        } else if args[index].starts_with('-') {
            index += 1;
        } else {
            positional.push(args[index]);
            index += 1;
        }
    }
    if positional.len() > 4 {
        return Err("run-test accepts at most family, type, buildId, and planId".to_owned());
    }
    let plan_id = positional
        .get(3)
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| "planId must be a positive integer".to_owned())?
        .filter(|value| *value > 0);
    let all = args.iter().any(|value| matches!(*value, "--all" | "-a"));
    let suites = flag_value(args, "--suites", "-s")?;
    let cases = flag_value(args, "--cases", "-c")?;
    if usize::from(all) + usize::from(suites.is_some()) + usize::from(cases.is_some()) > 1 {
        return Err("choose only one of --all, --suites, or --cases".to_owned());
    }
    let selection = match (plan_id, all, suites, cases) {
        (Some(plan_id), true, None, None) => Some(all_selection(plan_id)),
        (Some(plan_id), false, Some(suites), None) => Some(partial_selection(
            plan_id,
            comma_ids(&suites)?
                .into_iter()
                .map(|suite_id| ExecutionSuiteSelection {
                    suite_id,
                    select_all: true,
                    cases: None,
                    exclude_cases: None,
                })
                .collect(),
        )),
        (Some(plan_id), false, None, Some(cases)) => {
            Some(partial_selection(plan_id, case_selections(&cases)?))
        }
        (None, false, None, None) | (Some(_), false, None, None) => None,
        (None, _, _, _) => {
            return Err("planId is required when test selection flags are supplied".to_owned());
        }
        _ => unreachable!(),
    };
    Ok(RunTestArgs {
        family: positional.first().map(|value| (*value).to_owned()),
        device_type: positional.get(1).map(|value| (*value).to_owned()),
        build_id: positional.get(2).map(|value| (*value).to_owned()),
        plan_id,
        selection,
        logs: args.iter().any(|value| matches!(*value, "--logs" | "-l")),
    })
}

fn comma_ids(value: &str) -> Result<Vec<String>, String> {
    let ids = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if ids.is_empty() || ids.iter().any(|value| positive_id(Some(value)).is_none()) {
        return Err("suite ids must be positive integers".to_owned());
    }
    Ok(ids.into_iter().map(str::to_owned).collect())
}

fn case_selections(value: &str) -> Result<Vec<ExecutionSuiteSelection>, String> {
    let mut selections = Vec::<ExecutionSuiteSelection>::new();
    for token in value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some((suite_id, case_id)) = token.split_once(':') {
            if positive_id(Some(suite_id)).is_none() || positive_id(Some(case_id)).is_none() {
                return Err("cases must use suiteId:caseId,caseId format".to_owned());
            }
            selections.push(ExecutionSuiteSelection {
                suite_id: suite_id.to_owned(),
                select_all: false,
                cases: Some(vec![case_id.to_owned()]),
                exclude_cases: None,
            });
        } else {
            if positive_id(Some(token)).is_none() {
                return Err("cases must use suiteId:caseId,caseId format".to_owned());
            }
            let Some(selection) = selections.last_mut() else {
                return Err("the first case must include its suite id".to_owned());
            };
            selection
                .cases
                .get_or_insert_with(Vec::new)
                .push(token.to_owned());
        }
    }
    if selections.is_empty() {
        return Err("at least one case is required".to_owned());
    }
    Ok(selections)
}

async fn execute(state: &AppState, command: &str) -> String {
    if command.is_empty() {
        return String::new();
    }
    if command == "racer-clear" {
        return "\x1Bc".to_owned();
    }
    if command == "racer-help" || command.starts_with("racer-help ") {
        return HELP.to_owned();
    }
    let Some(database) = &state.database else {
        return "\x1b[31m Error: device persistence is unavailable\x1b[0m".to_owned();
    };
    let mut parts = command.split_whitespace();
    match parts.next() {
        Some("racer-list-devices") | Some("racer-list-devices-summary") => {
            list_devices(database.pool(), false).await
        }
        Some("racer-available-devices") => list_devices(database.pool(), true).await,
        Some("racer-device-stats") => device_stats(database.pool()).await,
        Some("racer-get-device") => match parts.next() {
            Some(device_id) => device_detail(database.pool(), device_id).await,
            None => "\x1b[31m Error: device id is required\x1b[0m".to_owned(),
        },
        Some("racer-list-builds") => list_builds(database.pool(), parts.collect()).await,
        Some("racer-list-test-plans") => list_test_plans(state, parts.collect()).await,
        Some("racer-list-test-suites") => list_test_suites(state, parts.collect()).await,
        Some("racer-list-test-cases") => list_test_cases(state, parts.collect()).await,
        Some(other) => format!("\x1b[31m Error: Unknown command: {other}\x1b[0m"),
        None => String::new(),
    }
}

async fn list_builds(pool: &sqlx::PgPool, args: Vec<&str>) -> String {
    let parsed = match parse_build_args(&args) {
        Ok(parsed) => parsed,
        Err(error) => return cli_error(error),
    };
    let query = BuildListQuery {
        device_family: parsed.family,
        device_type: parsed.device_type,
        flagged: parsed.flagged.then(|| "true".to_owned()),
        page: Some(1),
        limit: Some(parsed.limit),
        sort_by: Some("createdAt".to_owned()),
        sort_order: Some("desc".to_owned()),
        ..BuildListQuery::default()
    };
    match crate::devices::build_store::list_builds(pool, &query).await {
        Ok(builds) if builds.builds.is_empty() => "\x1b[2mNo builds found\x1b[0m".to_owned(),
        Ok(builds) => format_builds(&builds.builds),
        Err(error) => cli_error(error.to_string()),
    }
}

async fn list_test_plans(state: &AppState, args: Vec<&str>) -> String {
    let filter = match flag_value(&args, "--filter", "-f") {
        Ok(value) => value,
        Err(error) => return cli_error(error),
    };
    let Some(catalog) = &state.test_catalog else {
        return cli_error("TestRail is not configured");
    };
    match catalog.plans(filter.as_deref()).await {
        Ok(plans) if plans.is_empty() => "\x1b[2mNo test plans found\x1b[0m".to_owned(),
        Ok(plans) => {
            let mut output = "ID      NAME\r\n".to_owned();
            output.push_str(&"-".repeat(54));
            for plan in plans {
                output.push_str(&format!("\r\n{:<8} {}", plan.id, plan.name));
            }
            output
        }
        Err(error) => cli_error(error.to_string()),
    }
}

async fn list_test_suites(state: &AppState, args: Vec<&str>) -> String {
    let Some(plan_id) = positive_id(args.first().copied()) else {
        return cli_error("planId must be a positive integer");
    };
    let Some(catalog) = &state.test_catalog else {
        return cli_error("TestRail is not configured");
    };
    match catalog.suites(plan_id).await {
        Ok(suites) if suites.is_empty() => "\x1b[2mNo test suites found\x1b[0m".to_owned(),
        Ok(suites) => {
            let mut output = "ID      NAME\r\n".to_owned();
            output.push_str(&"-".repeat(54));
            for suite in suites {
                output.push_str(&format!("\r\n{:<8} {}", suite.id, suite.name));
            }
            output
        }
        Err(error) => cli_error(error.to_string()),
    }
}

async fn list_test_cases(state: &AppState, args: Vec<&str>) -> String {
    let Some(plan_id) = positive_id(args.first().copied()) else {
        return cli_error("planId must be a positive integer");
    };
    let Some(suite_id) = positive_id(args.get(1).copied()) else {
        return cli_error("suiteId must be a positive integer");
    };
    let filter = match flag_value(&args, "--filter", "-f") {
        Ok(value) => value,
        Err(error) => return cli_error(error),
    };
    let Some(catalog) = &state.test_catalog else {
        return cli_error("TestRail is not configured");
    };
    match catalog.cases(plan_id, suite_id, filter.as_deref()).await {
        Ok(cases) if cases.is_empty() => "\x1b[2mNo test cases found\x1b[0m".to_owned(),
        Ok(cases) => {
            let mut output = "ID      TITLE\r\n".to_owned();
            output.push_str(&"-".repeat(78));
            for case in cases {
                output.push_str(&format!("\r\n{:<8} {}", case.id, case.title));
            }
            output
        }
        Err(error) => cli_error(error.to_string()),
    }
}

struct BuildArgs {
    family: Option<String>,
    device_type: Option<String>,
    limit: i64,
    flagged: bool,
}

fn parse_build_args(args: &[&str]) -> Result<BuildArgs, String> {
    let positional = args
        .iter()
        .enumerate()
        .filter(|(index, value)| {
            !value.starts_with('-') && (*index == 0 || !matches!(args[index - 1], "--limit" | "-l"))
        })
        .map(|(_, value)| (*value).to_owned())
        .collect::<Vec<_>>();
    if positional.len() > 2 {
        return Err("list-builds accepts at most a family and device type".to_owned());
    }
    let limit = flag_value(args, "--limit", "-l")?
        .map(|value| value.parse::<i64>())
        .transpose()
        .map_err(|_| "limit must be a positive integer".to_owned())?
        .unwrap_or(10);
    if !(1..=1000).contains(&limit) {
        return Err("limit must be between 1 and 1000".to_owned());
    }
    Ok(BuildArgs {
        family: positional.first().cloned(),
        device_type: positional.get(1).cloned(),
        limit,
        flagged: args
            .iter()
            .any(|value| matches!(*value, "--flagged" | "-f")),
    })
}

fn flag_value(args: &[&str], long: &str, short: &str) -> Result<Option<String>, String> {
    let Some(index) = args
        .iter()
        .position(|value| *value == long || *value == short)
    else {
        return Ok(None);
    };
    args.get(index + 1)
        .filter(|value| !value.starts_with('-'))
        .map(|value| Some(value.trim_matches(['\'', '"']).to_owned()))
        .ok_or_else(|| format!("{long} requires a value"))
}

fn positive_id(value: Option<&str>) -> Option<u64> {
    value
        .and_then(|value| value.parse().ok())
        .filter(|id| *id > 0)
}

fn format_builds(builds: &[serde_json::Value]) -> String {
    let mut output =
        "ID                 VERSION       FAMILY       TYPE         TAG          STATUS\r\n"
            .to_owned();
    output.push_str(&"-".repeat(86));
    for build in builds {
        let value = |key: &str| {
            build
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        };
        let tag = match value("tag") {
            "" => "official",
            tag => tag,
        };
        let status = if build.get("isFaulty").and_then(serde_json::Value::as_bool) == Some(true) {
            "flagged"
        } else {
            "active"
        };
        output.push_str(&format!(
            "\r\n{:<18} {:<13} {:<12} {:<12} {:<12} {}",
            truncate(value("id"), 18),
            truncate(value("version"), 13),
            truncate(value("deviceFamily"), 12),
            truncate(value("deviceType"), 12),
            truncate(tag, 12),
            status
        ));
    }
    output
}

fn cli_error(error: impl std::fmt::Display) -> String {
    format!("\x1b[31m Error: {error}\x1b[0m")
}

async fn list_devices(pool: &sqlx::PgPool, available_only: bool) -> String {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String)>(
        r#"SELECT "deviceId", "deviceName", "deviceType", "deviceFamily", state::text
           FROM devices
           WHERE "deletedAt" IS NULL AND (NOT $1 OR state::text = 'free')
           ORDER BY "deviceName" ASC LIMIT 10000"#,
    )
    .bind(available_only)
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => {
            let mut output =
                "ID                 Name                 Type         Family       State\r\n"
                    .to_owned();
            output.push_str(&"-".repeat(78));
            for (id, name, device_type, family, state) in rows {
                output.push_str(&format!(
                    "\r\n{:<18} {:<20} {:<12} {:<12} {}",
                    truncate(&id, 18),
                    truncate(&name, 20),
                    truncate(device_type.as_deref().unwrap_or(""), 12),
                    truncate(family.as_deref().unwrap_or(""), 12),
                    state
                ));
            }
            output
        }
        Err(error) => format!("\x1b[31m Error: {error}\x1b[0m"),
    }
}

async fn device_stats(pool: &sqlx::PgPool) -> String {
    let rows = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
        r#"SELECT COALESCE("deviceType", 'Unknown'), count(*)::bigint,
                  count(*) FILTER (WHERE state::text = 'free')::bigint,
                  count(*) FILTER (WHERE state::text = 'busy')::bigint,
                  count(*) FILTER (WHERE state::text IN ('not_reachable', 'faulty'))::bigint
           FROM devices WHERE "deletedAt" IS NULL
           GROUP BY "deviceType" ORDER BY "deviceType""#,
    )
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => {
            let mut output = "Type         Total      Free       Busy       Offline\r\n".to_owned();
            output.push_str(&"-".repeat(56));
            for (device_type, total, free, busy, offline) in rows {
                output.push_str(&format!(
                    "\r\n{:<12} {:<10} {:<10} {:<10} {offline}",
                    truncate(&device_type, 12),
                    total,
                    free,
                    busy
                ));
            }
            output
        }
        Err(error) => format!("\x1b[31m Error: {error}\x1b[0m"),
    }
}

async fn device_detail(pool: &sqlx::PgPool, device_id: &str) -> String {
    let row = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT to_jsonb(device) || jsonb_build_object(
               'interfaces', COALESCE((
                   SELECT jsonb_agg(to_jsonb(interface) ORDER BY interface.id)
                   FROM device_interfaces interface WHERE interface."deviceId" = device."deviceId"
               ), '[]'::jsonb),
               'heartbeatData', (
                   SELECT heartbeat.data FROM heartbeats heartbeat
                   WHERE heartbeat."deviceId" = device."deviceId"
                   ORDER BY heartbeat.timestamp DESC LIMIT 1
               ))
           FROM devices device
           WHERE device."deviceId" = $1 AND device."deletedAt" IS NULL"#,
    )
    .bind(device_id)
    .fetch_optional(pool)
    .await;
    match row {
        Ok(Some(row)) => serde_json::to_string_pretty(&row).unwrap_or_default(),
        Ok(None) => format!("\x1b[31m Error: Device not found: {device_id}\x1b[0m"),
        Err(error) => format!("\x1b[31m Error: {error}\x1b[0m"),
    }
}

fn truncate(value: &str, width: usize) -> &str {
    value.get(..width).unwrap_or(value)
}

async fn send_cli(socket: &mut WebSocket, output: &str) -> Result<(), axum::Error> {
    send_json(socket, serde_json::json!({"type": "cli", "output": output})).await
}

async fn send_json(socket: &mut WebSocket, value: serde_json::Value) -> Result<(), axum::Error> {
    socket.send(Message::Text(value.to_string().into())).await
}

#[cfg(test)]
mod tests {
    use super::{case_selections, format_builds, parse_build_args, parse_run_test_args, truncate};

    #[test]
    fn table_values_are_width_bounded() {
        assert_eq!(truncate("123456", 4), "1234");
        assert_eq!(truncate("abc", 4), "abc");
    }

    #[test]
    fn build_arguments_are_bounded_and_parse_legacy_flags() {
        let parsed = parse_build_args(&["Gen4", "h3", "--limit", "20", "--flagged"]).unwrap();
        assert_eq!(parsed.family.as_deref(), Some("Gen4"));
        assert_eq!(parsed.device_type.as_deref(), Some("h3"));
        assert_eq!(parsed.limit, 20);
        assert!(parsed.flagged);
        assert!(parse_build_args(&["--limit", "0"]).is_err());
    }

    #[test]
    fn build_table_preserves_frontend_fields() {
        let output = format_builds(&[serde_json::json!({
            "id": "build-1", "version": "1.2.3", "deviceFamily": "Gen4",
            "deviceType": "h3", "tag": null, "isFaulty": true
        })]);
        assert!(output.contains("build-1"));
        assert!(output.contains("official"));
        assert!(output.contains("flagged"));
    }

    #[test]
    fn run_test_arguments_build_all_and_partial_selections() {
        let all = parse_run_test_args(&["Gen4", "h3", "build-1", "6", "--all", "--logs"]).unwrap();
        assert!(all.logs);
        assert!(matches!(
            all.selection,
            Some(crate::devices::ExecutionSelection::All { .. })
        ));

        let partial =
            parse_run_test_args(&["Gen4", "h3", "build-1", "6", "--suites", "30,31"]).unwrap();
        assert!(matches!(
            partial.selection,
            Some(crate::devices::ExecutionSelection::Partial { .. })
        ));
    }

    #[test]
    fn case_spec_groups_cases_under_their_suite() {
        let selections = case_selections("30:38,39,31:101,102").unwrap();
        assert_eq!(selections.len(), 2);
        assert_eq!(selections[0].suite_id, "30");
        assert_eq!(
            selections[0].cases.as_deref(),
            Some(["38".to_owned(), "39".to_owned()].as_slice())
        );
        assert_eq!(selections[1].suite_id, "31");
    }
}
