//! Device interface precondition evaluation for queued test actions.
//!
//! Used by the device-action worker to skip devices that lack required interfaces
//! (USB, HDMI, etc.) before assignment.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};

use crate::persistence::entity::{device_interface, test_case};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
    AtLeast,
}

#[derive(Debug, PartialEq, Eq)]
struct Condition {
    interface_type: String,
    operator: Operator,
    value: i64,
}

/// Return true if the device's interface counts satisfy the precondition expression.
pub(super) async fn device_satisfies(
    transaction: &DatabaseTransaction,
    device_id: &str,
    test_id: &str,
) -> Result<bool, sea_orm::DbErr> {
    let interfaces = device_interface::Entity::find()
        .filter(device_interface::Column::DeviceId.eq(device_id))
        .all(transaction)
        .await?;

    let mut counts = BTreeMap::new();

    for interface in interfaces {
        // Preserve the existing behavior:
        //
        // disconnected
        // down
        // unknown
        // not_available
        //
        // do not contribute to the interface count.
        if matches!(
            interface.status.unwrap_or_default(),
            device_interface::DeviceInterfaceStatus::Disconnected
                | device_interface::DeviceInterfaceStatus::Down
                | device_interface::DeviceInterfaceStatus::Unknown
                | device_interface::DeviceInterfaceStatus::NotAvailable
        ) {
            continue;
        }

        let Some(interface_type) = interface.r#type.as_deref() else {
            continue;
        };
        *counts
            .entry(interface_type.trim().to_lowercase())
            .or_insert(0) += 1;
    }

    let test_cases = test_case::Entity::find()
        .filter(test_case::Column::ExecutionId.eq(test_id))
        .all(transaction)
        .await?;

    Ok(test_cases
        .into_iter()
        .filter_map(|test_case| test_case.pre_condition)
        .flat_map(|value| {
            value
                .lines()
                .map(|line| line.replace('\\', "").trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        })
        .all(|expression| evaluate(&expression, &counts)))
}

/// Evaluate a boolean expression over interface-type counts.
fn evaluate(expression: &str, counts: &BTreeMap<String, i64>) -> bool {
    let tokens = expression
        .split(':')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if tokens.len() < 2 || tokens.len() % 3 == 0 {
        return false;
    }

    let Some(first) = condition(tokens[0], tokens[1]) else {
        return false;
    };

    let mut result = matches_condition(&first, counts);
    let mut index = 2;

    while index < tokens.len() {
        let connector = tokens[index].to_ascii_lowercase();

        if !matches!(connector.as_str(), "&" | "or") || index + 2 >= tokens.len() {
            return false;
        }

        let Some(next) = condition(tokens[index + 1], tokens[index + 2]) else {
            return false;
        };

        let current = matches_condition(&next, counts);

        result = if connector == "&" {
            result && current
        } else {
            result || current
        };

        index += 3;
    }

    result
}

/// Parse a single interface requirement token into a Condition.
fn condition(interface_type: &str, requirement: &str) -> Option<Condition> {
    let interface_type = interface_type.trim().to_lowercase();

    if interface_type.is_empty() {
        return None;
    }

    let requirement = requirement.trim();

    let (operator, number) = if let Some(value) = requirement.strip_prefix(">=") {
        (Operator::GreaterOrEqual, value)
    } else if let Some(value) = requirement.strip_prefix("<=") {
        (Operator::LessOrEqual, value)
    } else if let Some(value) = requirement.strip_prefix('>') {
        (Operator::Greater, value)
    } else if let Some(value) = requirement.strip_prefix('<') {
        (Operator::Less, value)
    } else {
        (Operator::AtLeast, requirement)
    };

    Some(Condition {
        interface_type,
        operator,
        value: number.trim().parse().ok()?,
    })
}

/// Check one Condition against aggregated interface counts.
fn matches_condition(condition: &Condition, counts: &BTreeMap<String, i64>) -> bool {
    let count = counts
        .get(&condition.interface_type)
        .copied()
        .unwrap_or_default();

    match condition.operator {
        Operator::Greater => count > condition.value,
        Operator::Less => count < condition.value,
        Operator::GreaterOrEqual | Operator::AtLeast => count >= condition.value,
        Operator::LessOrEqual => count <= condition.value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Unit test: precondition expression evaluation parity with legacy.
    fn evaluates_legacy_interface_count_rules() {
        let counts = BTreeMap::from([("uart".to_owned(), 2), ("usb".to_owned(), 1)]);

        assert!(evaluate("uart:2", &counts));
        assert!(evaluate("uart:>=2:&:usb:1", &counts));
        assert!(evaluate("i2c:1:or:usb:1", &counts));
        assert!(!evaluate("uart:>2", &counts));
        assert!(!evaluate("uart:2:xor:usb:1", &counts));
        assert!(!evaluate("invalid", &counts));
    }
}
