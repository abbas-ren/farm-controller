use std::collections::BTreeMap;

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

pub(super) async fn device_satisfies(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    device_id: &str,
    test_id: &str,
) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String)>(
        r#"SELECT type, status::text FROM device_interfaces WHERE "deviceId" = $1"#,
    )
    .bind(device_id)
    .fetch_all(&mut **transaction)
    .await?;
    let mut counts = BTreeMap::new();
    for (interface_type, status) in rows {
        if matches!(
            status.as_str(),
            "disconnected" | "down" | "unknown" | "not_available"
        ) {
            continue;
        }
        *counts
            .entry(interface_type.trim().to_lowercase())
            .or_insert(0) += 1;
    }
    let preconditions = sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT "preCondition" FROM testcase WHERE "executionId" = $1"#,
    )
    .bind(test_id)
    .fetch_all(&mut **transaction)
    .await?;
    Ok(preconditions
        .into_iter()
        .flatten()
        .flat_map(|value| {
            value
                .lines()
                .map(|line| line.replace('\\', "").trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        })
        .all(|expression| evaluate(&expression, &counts)))
}

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
