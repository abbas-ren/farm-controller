//! Suite summaries, sanity-analysis artifacts, and shared HTML/file helpers.

use std::path::Path;

use chrono::Utc;
use regex::Regex;

use super::TestCaseRecord;

pub(crate) async fn append_log(
    root: &Path,
    level: &str,
    message: &str,
) -> Result<(), std::io::Error> {
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("report_generation.log"))
        .await?;
    file.write_all(format!("[{}] [{}] {}\n", Utc::now().to_rfc3339(), level, message).as_bytes())
        .await
}

pub(crate) async fn write_suite_artifacts(
    root: &Path,
    cases: &[TestCaseRecord],
) -> Result<(), std::io::Error> {
    for (suite, text_name, html_name) in [
        (
            "sanity",
            "sanity_commands_output.txt",
            "sanity_commands_report.html",
        ),
        (
            "performance",
            "performance_commands_output.txt",
            "performance_commands_report.html",
        ),
    ] {
        let selected: Vec<_> = cases
            .iter()
            .filter(|case| case.suite_name.eq_ignore_ascii_case(suite))
            .collect();
        if selected.is_empty() {
            continue;
        }
        let mut text = String::new();
        let mut rows = String::new();
        for case in selected {
            let result = case.result.as_deref().unwrap_or("FAIL");
            let command = case.command.as_deref().unwrap_or("N/A");
            let output_path = case.output_file_path.as_deref().unwrap_or("");
            let output_name = file_name(case.output_file_path.as_deref(), case.test_case_id);
            if suite == "performance" {
                let module = case
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.first())
                    .map_or("", String::as_str);
                text.push_str(&format!(
                    "Test {} - Module: {module}\nCommand: {command}\nSaved to: {output_name}\n\n{}\n",
                    case.test_case_id,
                    "-".repeat(60)
                ));
                rows.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>View Output</a></td><td></td><td>{}</td><td>{}</td></tr>",
                    case.test_case_id,
                    escape_html(module),
                    escape_html(command),
                    escape_html(output_path),
                    jira_cell(case.jira_defect.as_deref()),
                    escape_html(case.comment.as_deref().unwrap_or(""))
                ));
            } else {
                text.push_str(&format!(
                    "Test {}\nCommand: {command}\nSaved to: {output_name}\n\n{}\n",
                    case.test_case_id,
                    "-".repeat(60)
                ));
                let color = if result == "PASS" { "green" } else { "red" };
                rows.push_str(&format!(
                    "<tr style='color:{color};'><td>{}</td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>View Output</a></td><td>{}</td><td>{}</td></tr>",
                    case.test_case_id,
                    escape_html(command),
                    escape_html(result),
                    escape_html(output_path),
                    jira_cell(case.jira_defect.as_deref()),
                    escape_html(case.comment.as_deref().unwrap_or(""))
                ));
            }
        }
        let (title, header) = if suite == "performance" {
            (
                "Performance Benchmarking",
                "<tr><th>Test No</th><th>Module</th><th>Command</th><th>Measured Result</th><th>Expected Result</th><th>JIRA ID</th><th>Remark</th></tr>",
            )
        } else {
            (
                "Sanity",
                "<tr><th>Test No</th><th>Commands</th><th>Test Results</th><th>Logs</th><th>JIRA ID</th><th>Remark</th></tr>",
            )
        };
        tokio::fs::write(root.join(text_name), text).await?;
        tokio::fs::write(root.join(html_name), format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title} Test Report:</title></head><body><h2>{title} Test Report</h2><table border='1' cellpadding='5' cellspacing='0'><thead>{header}</thead><tbody>{rows}</tbody></table></body></html>"
        )).await?;
    }
    Ok(())
}

pub(crate) async fn write_sanity_analysis(root: &Path) -> Result<(), std::io::Error> {
    let results = root.join("sanity_commands_results");
    if !results.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Sanity results directory not found: {}", results.display()),
        ));
    }
    for required in [
        root.join("sanity_commands_report.html"),
        root.join("sanity_commands_output.txt"),
    ] {
        if !required.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Sanity report artifact not found: {}", required.display()),
            ));
        }
    }

    let mut entries = tokio::fs::read_dir(&results).await?;
    let mut inputs = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        {
            inputs.push(path);
        }
    }
    inputs.sort();
    if inputs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "No .txt files found in sanity results: {}",
                results.display()
            ),
        ));
    }

    let output = root.join("sanity_command_analysis");
    tokio::fs::create_dir_all(&output).await?;
    for input in inputs {
        let content = tokio::fs::read_to_string(&input).await?;
        let input_name = input
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sanity.txt");
        let lines = content.lines().collect::<Vec<_>>();
        let errors = lines
            .iter()
            .filter(|line| line.to_ascii_lowercase().contains("error"))
            .take(5)
            .copied()
            .collect::<Vec<_>>();
        let warnings = lines
            .iter()
            .filter(|line| line.to_ascii_lowercase().contains("warn"))
            .take(5)
            .copied()
            .collect::<Vec<_>>();
        let list = |items: &[&str]| {
            items
                .iter()
                .map(|item| format!("<li>{}</li>", escape_html(item)))
                .collect::<String>()
        };
        let html = format!(
            "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\" />\n  <title>Analysis Report</title>\n  <style>\n    body {{ font-family: Arial, sans-serif; margin: 20px; }}\n    h1, h2 {{ color: #2c3e50; }}\n    ul {{ padding-left: 20px; }}\n    pre {{ background: #f4f4f4; padding: 10px; border: 1px solid #ccc; white-space: pre-wrap; }}\n  </style>\n</head>\n<body>\n  <h1>Analysis Report: {}</h1>\n  <p>Total lines: {} | Errors: {} | Warnings: {}</p>\n  <h2>Top Errors</h2>\n  <ul>{}</ul>\n  <h2>Top Warnings</h2>\n  <ul>{}</ul>\n  <h2>Raw Output</h2>\n  <pre>{}</pre>\n</body>\n</html>\n",
            escape_html(input_name),
            lines.len(),
            errors.len(),
            warnings.len(),
            list(&errors),
            list(&warnings),
            escape_html(&content),
        );
        let stem = input
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("sanity");
        tokio::fs::write(output.join(format!("{stem}_analysis.html")), html).await?;
    }
    Ok(())
}

pub(crate) fn file_name(path: Option<&str>, test_case_id: i64) -> String {
    path.and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .map_or_else(|| format!("test_{test_case_id}_result.txt"), str::to_owned)
}

pub(crate) fn jira_cell(value: Option<&str>) -> String {
    let value = value.unwrap_or("").trim();
    if value.starts_with("http://") || value.starts_with("https://") {
        let escaped = escape_html(value);
        format!("<a href='{escaped}' target='_blank'>{escaped}</a>")
    } else {
        escape_html(value)
    }
}

pub(crate) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn report_artifact_href(report_url: Option<&str>, relative_path: &str) -> String {
    report_url.map_or_else(
        || relative_path.to_owned(),
        |url| format!("{}/{relative_path}", url.trim_end_matches('/')),
    )
}

pub(crate) fn parse_first_number(value: &str) -> Option<f64> {
    Regex::new(r"([0-9]+(?:\.[0-9]+)?)")
        .expect("static regex")
        .captures(value)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}
