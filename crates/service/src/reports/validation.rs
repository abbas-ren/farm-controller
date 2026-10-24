//! Report identifier validation and root-contained artifact path construction.

use std::path::{Component, Path, PathBuf};

use super::{ReportError, ReportRequest};

/// Reject report requests missing required ids/paths.
pub(crate) fn validate_request(request: &ReportRequest) -> Result<(), ReportError> {
    let _api_trace = crate::observability::ApiTrace::enter("reports::validation", "validate_request");
    for (name, value) in [
        ("testId", request.test_id.as_str()),
        ("buildVersion", request.build_version.as_str()),
        ("deviceType", request.device_type.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(ReportError::Validation(format!(
                "{name} is required and cannot be empty"
            )));
        }
        let mut components = Path::new(value).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(ReportError::Validation(format!(
                "invalid path component: {name}"
            )));
        }
    }
    Ok(())
}

/// Resolve the on-disk root directory for a report job.
pub(crate) fn report_root(base: &Path, request: &ReportRequest) -> Result<PathBuf, ReportError> {
    let _api_trace = crate::observability::ApiTrace::enter("reports::validation", "report_root");
    validate_request(request)?;
    Ok(base
        .join(request.device_type.trim())
        .join(request.build_version.trim())
        .join(request.test_id.trim()))
}
