//! Resolve legacy and explicit TestRail selections into persisted execution cases.

use super::*;
use crate::state::AppState;

pub(super) async fn resolve_execution(
    state: &AppState,
    request: CreateExecutionRequest,
    user_id: String,
) -> Result<ExecutionCreation, super::super::error::DeviceRepositoryError> {
    use super::super::error::DeviceRepositoryError;

    if request.device_type.trim().is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "Device type is required".to_owned(),
        ));
    }
    if request.build_id.trim().is_empty() {
        return Err(DeviceRepositoryError::Validation(
            "Build ID is required".to_owned(),
        ));
    }
    let selection = request.selection.clone();
    let mut plan_name = request.test_plan_name.clone().unwrap_or_default();
    let mut suites = request.test_suites.clone();
    let mut is_all_selected = request.is_all_selected;
    let mut cases = Vec::new();
    let plan_id = match &selection {
        Some(ExecutionSelection::All {
            plan_id,
            plan_name: selected_plan_name,
            exclude,
        }) => {
            let plan_id = encoded_id(plan_id)?;
            if plan_name.is_empty() {
                plan_name = selected_plan_name.clone().unwrap_or_default();
            }
            let catalog = state.test_catalog.as_ref().ok_or_else(|| {
                DeviceRepositoryError::Internal("TestRail is not configured".to_owned())
            })?;
            let catalog_suites = catalog
                .suites(plan_id as u64)
                .await
                .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
            let excluded_suites = exclude
                .as_ref()
                .map(|exclude| {
                    exclude
                        .suites
                        .iter()
                        .map(|value| encoded_id(value))
                        .collect::<Result<std::collections::BTreeSet<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            for suite in catalog_suites {
                let suite_id = suite.id as i64;
                if excluded_suites.contains(&suite_id) {
                    continue;
                }
                let excluded_cases = exclude
                    .as_ref()
                    .and_then(|exclude| {
                        exclude.cases_by_suite.iter().find_map(|(key, values)| {
                            (encoded_id(key).ok() == Some(suite_id)).then_some(values)
                        })
                    })
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| encoded_id(value))
                            .collect::<Result<std::collections::BTreeSet<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                cases.extend(
                    catalog
                        .cases(plan_id as u64, suite.id, None)
                        .await
                        .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?
                        .into_iter()
                        .filter(|case| !excluded_cases.contains(&(case.id as i64)))
                        .map(|case| catalog_case(case, suite.name.clone())),
                );
            }
            is_all_selected = exclude.as_ref().is_none_or(|exclude| {
                exclude.suites.is_empty() && exclude.cases_by_suite.is_empty()
            });
            suites.clear();
            plan_id
        }
        Some(ExecutionSelection::Partial {
            plan_id,
            plan_name: selected_plan_name,
            suites: selected_suites,
        }) => {
            if selected_suites.is_empty() {
                return Err(DeviceRepositoryError::Validation(
                    "At least one suite selection is required".to_owned(),
                ));
            }
            let plan_id = encoded_id(plan_id)?;
            if plan_name.is_empty() {
                plan_name = selected_plan_name.clone().unwrap_or_default();
            }
            let catalog = state.test_catalog.as_ref().ok_or_else(|| {
                DeviceRepositoryError::Internal("TestRail is not configured".to_owned())
            })?;
            let catalog_suites = catalog
                .suites(plan_id as u64)
                .await
                .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
            suites.clear();
            for selected in selected_suites {
                let suite_id = encoded_id(&selected.suite_id)?;
                if selected.select_all && selected.cases.is_some() {
                    return Err(DeviceRepositoryError::Validation(
                        "Selected cases must be omitted when selectAll is true".to_owned(),
                    ));
                }
                let selected_case_ids = if selected.select_all {
                    None
                } else {
                    let values = selected
                        .cases
                        .as_ref()
                        .filter(|values| !values.is_empty())
                        .ok_or_else(|| {
                            DeviceRepositoryError::Validation(
                                "Selected cases are required when selectAll is false".to_owned(),
                            )
                        })?;
                    Some(
                        values
                            .iter()
                            .map(|value| encoded_id(value))
                            .collect::<Result<std::collections::BTreeSet<_>, _>>()?,
                    )
                };
                let excluded = selected
                    .exclude_cases
                    .as_ref()
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| encoded_id(value))
                            .collect::<Result<std::collections::BTreeSet<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let suite = catalog_suites
                    .iter()
                    .find(|suite| suite.id as i64 == suite_id)
                    .ok_or_else(|| {
                        DeviceRepositoryError::Validation(format!("Suite {suite_id} not found"))
                    })?;
                suites.push(suite_id);
                cases.extend(
                    catalog
                        .cases(plan_id as u64, suite.id, None)
                        .await
                        .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?
                        .into_iter()
                        .filter(|case| {
                            let case_id = case.id as i64;
                            !excluded.contains(&case_id)
                                && selected_case_ids
                                    .as_ref()
                                    .is_none_or(|ids| ids.contains(&case_id))
                        })
                        .map(|case| catalog_case(case, suite.name.clone())),
                );
            }
            is_all_selected = false;
            plan_id
        }
        None => {
            let plan_id = request.test_plan_id.ok_or_else(|| {
                DeviceRepositoryError::Validation(
                    "Provide selection or a legacy testPlanId and testcase selection".to_owned(),
                )
            })?;
            if !request.is_all_selected && suites.is_empty() && request.test_cases.is_empty() {
                return Err(DeviceRepositoryError::Validation(
                    "Provide selection or a legacy testPlanId and testcase selection".to_owned(),
                ));
            }
            cases.extend(request.test_cases.values().flatten().cloned());
            if request.is_all_selected || !suites.is_empty() {
                let catalog = state.test_catalog.as_ref().ok_or_else(|| {
                    DeviceRepositoryError::Internal("TestRail is not configured".to_owned())
                })?;
                let catalog_suites = catalog
                    .suites(plan_id as u64)
                    .await
                    .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?;
                for suite in catalog_suites {
                    if request.is_all_selected || suites.contains(&(suite.id as i64)) {
                        cases.extend(
                            catalog
                                .cases(plan_id as u64, suite.id, None)
                                .await
                                .map_err(|error| {
                                    DeviceRepositoryError::Internal(error.to_string())
                                })?
                                .into_iter()
                                .map(|case| catalog_case(case, suite.name.clone())),
                        );
                    }
                }
            }
            plan_id
        }
    };
    let mut unique_cases = std::collections::BTreeMap::new();
    for case in cases {
        unique_cases.entry(case.test_case_id).or_insert(case);
    }
    let mut cases = unique_cases.into_values().collect::<Vec<_>>();
    cases.sort_by_key(|case| (case.suite_id, case.test_case_id));
    if plan_name.is_empty()
        && request
            .name
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
    {
        if let Some(catalog) = &state.test_catalog {
            plan_name = catalog
                .plans(None)
                .await
                .map_err(|error| DeviceRepositoryError::Internal(error.to_string()))?
                .into_iter()
                .find(|plan| plan.id == plan_id as u64)
                .map(|plan| plan.name)
                .unwrap_or_else(|| "Untitled".to_owned());
        } else {
            plan_name = "Untitled".to_owned();
        }
    }
    Ok(ExecutionCreation {
        name: request.name,
        device_family: request.device_family,
        device_type: request.device_type,
        build_id: request.build_id,
        plan_id,
        plan_name,
        test_suites: suites,
        is_all_selected,
        selection,
        cases,
        user_id,
    })
}

fn encoded_id(value: &str) -> Result<i64, super::super::error::DeviceRepositoryError> {
    let digits = value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse::<i64>().map_err(|_| {
        super::super::error::DeviceRepositoryError::Validation(format!(
            "Invalid ID format: {value}"
        ))
    })
}

fn catalog_case(case: crate::test_catalog::TestCase, suite_name: String) -> ExecutionCaseInput {
    ExecutionCaseInput {
        test_case_id: case.id as i64,
        name: None,
        execution_id: None,
        suite_id: case.suite_id as i64,
        script_file: case.script_file,
        suite_name,
        title: case.title,
        plan_id: case.plan_id as i64,
        order: Some(case.order),
        priority_id: Some(case.priority_id as i64),
        result: None,
        pre_condition: case.pre_condition,
        labels: case.labels,
    }
}
