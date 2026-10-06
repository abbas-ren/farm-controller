//! Validation rules for public registration requests.

use crate::auth::RegistrationRequest;

pub(super) fn validate_registration(request: &RegistrationRequest) -> Result<(), &'static str> {
    let valid_username = (3..=30).contains(&request.username.len())
        && request
            .username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_'));
    if !valid_username {
        return Err(
            "Username must be 3-30 characters and contain only letters, numbers, dots, or underscores",
        );
    }
    if !request.email.contains('@')
        || request.email.starts_with('@')
        || request.email.ends_with('@')
    {
        return Err("Invalid email address");
    }
    for (value, missing, invalid) in [
        (
            request.first_name.as_str(),
            "First name is required",
            "First name must contain only letters, numbers, or underscores",
        ),
        (
            request.last_name.as_str(),
            "Last name is required",
            "Last name must contain only letters, numbers, or underscores",
        ),
    ] {
        if value.is_empty() {
            return Err(missing);
        }
        if value.len() > 50
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Err(invalid);
        }
    }
    Ok(())
}
