//! Shared validation rules and protocol status values for execution handlers.

pub(super) const ALLOWED_RESULTS: &[&str] = &["PASS", "FAIL", "BLOCKED", "RETEST", "NOT EXECUTED"];
pub(super) const REPORT_STATUS_COMPLETED: &str = "completed";
pub(super) const REPORT_STATUS_FAILED: &str = "failed";
pub(super) const REPORT_STATUS_UPLOADED: &str = "uploaded";
pub(super) const REPORT_STATUS_UPLOADING: &str = "uploading";

pub(super) fn safe_segment(value: &str) -> bool {
    let mut components = std::path::Path::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}
