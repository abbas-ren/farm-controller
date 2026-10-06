//! Stable messages grouped by the report boundary that emits them.

pub(super) mod http {
    pub const REPORTS_UNAVAILABLE: &str = "Reports are unavailable";
    pub const QUEUE_UNAVAILABLE: &str = "Report queue is unavailable";
    pub const QUEUE_FAILED: &str = "Failed to queue report";
    pub const CONFLUENCE_ARTIFACT_FAILED: &str = "Failed to access report artifacts";
}

pub(super) mod service {
    pub const GENERATION_STARTED: &str = "Report generation started";
    pub const GENERATION_COMPLETED: &str = "Report generation completed";
    pub const PERFORMANCE_COMPARISON_SKIPPED: &str = "Performance comparison skipped";
    pub const PERFORMANCE_GRAPHS_SKIPPED: &str = "Performance graphs skipped";
}

pub(super) mod confluence {
    pub const REPORT_HTML_MISSING: &str =
        "Report HTML not found. Create the report first before uploading to Confluence.";
    pub const PARENT_PAGE_ID_MISSING: &str = "Could not extract parent page ID from Confluence URL";
    pub const UPLOAD_STARTED: &str = "Confluence upload started";
    pub const UPLOAD_COMPLETED: &str = "Confluence upload completed";
}
