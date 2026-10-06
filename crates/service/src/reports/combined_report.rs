//! Assembly of the final report HTML and boot-time summary parsing.

use std::path::Path;

use regex::Regex;
use scraper::{Html, Selector};

use super::{
    ReportRequest, TestCaseRecord,
    artifact_writers::{
        escape_html, file_name, jira_cell, parse_first_number, report_artifact_href,
    },
};

pub(crate) async fn write_combined_report(
    root: &Path,
    request: &ReportRequest,
    cases: &[TestCaseRecord],
    report_base_url: &str,
) -> Result<(), std::io::Error> {
    let sanity = cases
        .iter()
        .filter(|case| case.suite_name.eq_ignore_ascii_case("sanity"))
        .collect::<Vec<_>>();
    let passed = sanity
        .iter()
        .filter(|case| case.result.as_deref() == Some("PASS"))
        .count();
    let failed = sanity
        .iter()
        .filter(|case| case.result.as_deref() == Some("FAIL"))
        .count();
    let missing = sanity.len().saturating_sub(passed + failed);
    let pass_percentage = if sanity.is_empty() {
        0.0
    } else {
        passed as f64 / sanity.len() as f64 * 100.0
    };
    let pass_color = if pass_percentage >= 50.0 {
        "green"
    } else {
        "red"
    };
    let report_url = (!report_base_url.trim().is_empty()).then(|| {
        format!(
            "{}/{}/{}/{}",
            report_base_url.trim_end_matches('/'),
            request.device_type,
            request.build_version,
            request.test_id
        )
    });
    let boot_time = if root.join("boottime.html").is_file() {
        parse_boottime_total(&tokio::fs::read_to_string(root.join("boottime.html")).await?)
            .map(|value| format!("{value:.3}"))
            .unwrap_or_default()
    } else {
        String::new()
    };

    let performance_rows = cases
        .iter()
        .filter(|case| case.suite_name.eq_ignore_ascii_case("performance"))
        .map(|case| {
            let output_name = file_name(case.output_file_path.as_deref(), case.test_case_id);
            let output_stem = output_name
                .strip_suffix("_result.txt")
                .or_else(|| output_name.strip_suffix(".txt"))
                .unwrap_or(&output_name);
            let comparison_name = format!("{output_stem}_comp.html");
            let result_href = report_artifact_href(
                report_url.as_deref(),
                &format!("performance_commands_results/{output_name}"),
            );
            let comparison_href = report_artifact_href(
                report_url.as_deref(),
                &format!("perf_compare/{comparison_name}"),
            );
            let module = case
                .labels
                .as_ref()
                .and_then(|labels| labels.first())
                .map_or("", String::as_str);
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>{}</a></td><td></td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>{}</a></td></tr>",
                case.test_case_id,
                escape_html(module),
                escape_html(case.command.as_deref().unwrap_or("")),
                escape_html(&result_href),
                escape_html(&output_name),
                jira_cell(case.jira_defect.as_deref()),
                escape_html(case.comment.as_deref().unwrap_or("")),
                escape_html(&comparison_href),
                escape_html(&comparison_name),
            )
        })
        .collect::<String>();
    let sanity_rows = sanity
        .iter()
        .map(|case| {
            let output_name = file_name(case.output_file_path.as_deref(), case.test_case_id);
            let output_stem = output_name.strip_suffix(".txt").unwrap_or(&output_name);
            let analysis_name = format!("{output_stem}_analysis.html");
            let result_href = report_artifact_href(
                report_url.as_deref(),
                &format!("sanity_commands_results/{output_name}"),
            );
            let analysis_href = report_artifact_href(
                report_url.as_deref(),
                &format!("sanity_command_analysis/{analysis_name}"),
            );
            let result = case.result.as_deref().unwrap_or("FAIL");
            let color = if result == "PASS" { "green" } else { "red" };
            format!(
                "<tr style='color:{color};'><td>{}</td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>{}</a></td><td>{}</td><td>{}</td><td><a href='{}' target='_blank'>{}</a></td></tr>",
                case.test_case_id,
                escape_html(case.command.as_deref().unwrap_or("")),
                escape_html(result),
                escape_html(&result_href),
                escape_html(&output_name),
                jira_cell(case.jira_defect.as_deref()),
                escape_html(case.comment.as_deref().unwrap_or("")),
                escape_html(&analysis_href),
                escape_html(&analysis_name),
            )
        })
        .collect::<String>();
    let comparison_dir = root.join("perf_compare");
    let radar_section = if tokio::fs::try_exists(comparison_dir.join("radar_chart.png"))
        .await
        .unwrap_or(false)
    {
        let source = report_url.as_ref().map_or_else(
            || "perf_compare/radar_chart.png".to_owned(),
            |url| format!("{url}/perf_compare/radar_chart.png"),
        );
        format!(
            "<h2>Overall Performance Compared to the Last Release:</h2><div><img src=\"{}\" alt='radar_chart.png' data-ac-width='1000' style='max-width:1000px;'/></div>",
            escape_html(&source)
        )
    } else {
        String::new()
    };
    let mut bar_images = String::new();
    for category in ["network", "storage", "cpu", "memory", "dma", "gpu"] {
        let name = format!("{category}_bar.png");
        if tokio::fs::try_exists(comparison_dir.join(&name))
            .await
            .unwrap_or(false)
        {
            let source = report_url.as_ref().map_or_else(
                || format!("perf_compare/{name}"),
                |url| format!("{url}/perf_compare/{name}"),
            );
            bar_images.push_str(&format!(
                "<td style='padding:15px;border:2px solid #333;'><img src=\"{}\" alt='{name}' data-ac-width='275' style='max-width:275px;'/></td>",
                escape_html(&source)
            ));
        }
    }
    let bars_section = if bar_images.is_empty() {
        String::new()
    } else {
        format!(
            "<h2>Subsystem Performance Comparison:</h2><table style='border-collapse:separate;border-spacing:15px;'><tr>{bar_images}</tr></table>"
        )
    };
    let boot_report_link = report_url.as_ref().map_or_else(
        || "<ac:link><ri:attachment ri:filename='boottime_report.html'/><ac:plain-text-link-body><![CDATA[Click here for Full Boot-Time Report (boottime_report.html)]]></ac:plain-text-link-body></ac:link>".to_owned(),
        |url| format!("<a href=\"{}/boottime_report.html\" target=\"_blank\">Click here for Full Boot-Time Report (boottime_report.html)</a>", escape_html(url)),
    );
    let other_reports = report_url.as_ref().map_or_else(String::new, |url| {
        let rows = [
            ("Busmoni Dashboard", format!("{url}/busmoni_plots/busmoni_dashboard.html")),
            ("PVRTune", format!("{url}/PVRTune_plots/pvrtune_gpu_dashboard.html")),
            ("Test Data", format!("{url}/")),
            ("GitLab Repo Link", "https://rcar-env.dgn.renesas.com/gitlab/rcar-reference-sw/utils/jenkins/".to_owned()),
        ]
        .into_iter()
        .map(|(label, link)| format!("<tr><td style='padding:8px;white-space:nowrap;'>{}</td><td style='padding:8px;white-space:nowrap;'><a href=\"{}\" target=\"_blank\">&#8594; Click here</a></td></tr>", escape_html(label), escape_html(&link)))
        .collect::<String>();
        format!("<hr/><h2>Other Reports:</h2><table border='1' style='border-collapse:collapse;width:auto;table-layout:auto;'><tr><th>Items</th><th>Links</th></tr>{rows}</table><hr/><h2>References:</h2><p><a href=\"https://confluence.renesas.com/spaces/RD2022/pages/315958920/PVRTune+on+x5h\" target=\"_blank\">&#8594; Click here to analyse PVRTune data using GUI, refer this writeup (PVRTune on x5h)</a></p>")
    });

    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{} {} report</title></head><body><h1>{} {}</h1><p>Test ID: {}</p>
<h2>Summary:</h2><table border='1' style='border-collapse:collapse;text-align:center;'><tr><th>SI.No</th><th>Category</th><th>Total Tests</th><th>PASS</th><th>FAIL</th><th>MISSING</th><th>PASS%</th></tr><tr><td>1</td><td>Sanity</td><td>{}</td><td style='color:green'>{passed}</td><td style='color:red'>{failed}</td><td style='color:grey'>{missing}</td><td style='color:{pass_color};font-weight:bold'>{pass_percentage:.1}%</td></tr></table><hr/>
<h2>Boot Time Summary:</h2><table border='1' style='border-collapse:collapse;width:auto;'><tr><th>Stage</th><th>Time (s)</th><th>Reports</th></tr><tr><td>Boot time to root login</td><td>{}</td><td>{boot_report_link}<br/><ac:link><ri:attachment ri:filename='uart.txt'/><ac:plain-text-link-body><![CDATA[Click here to Download UART Log (uart.txt)]]></ac:plain-text-link-body></ac:link></td></tr><tr><td>Boot time to display (Manual)</td><td></td><td></td></tr></table><hr/>{radar_section}{bars_section}
<h2>Performance Benchmarking Test Report</h2><table border='1' cellpadding='5' cellspacing='0'><tr><th>Test No</th><th>Module</th><th>Command</th><th>Measured Result</th><th>Expected Result</th><th>JIRA ID</th><th>Remark</th><th>Comparison With Prev SDK</th></tr>{performance_rows}</table>
<h2>Sanity Test Report</h2><table border='1' cellpadding='5' cellspacing='0'><tr><th>Test No</th><th>Commands</th><th>Test Results</th><th>Logs</th><th>JIRA ID</th><th>Remark</th><th>Analysis</th></tr>{sanity_rows}</table>{other_reports}</body></html>",
        escape_html(&request.device_type),
        escape_html(&request.build_version),
        escape_html(&request.device_type),
        escape_html(&request.build_version),
        escape_html(&request.test_id),
        sanity.len(),
        escape_html(&boot_time),
    );
    tokio::fs::write(root.join("report.html"), html).await
}

fn parse_boottime_total(content: &str) -> Option<f64> {
    let document = Html::parse_document(content);
    let table_selector = Selector::parse("table").expect("static selector");
    let header_selector = Selector::parse("th").expect("static selector");
    let row_selector = Selector::parse("tr").expect("static selector");
    let cell_selector = Selector::parse("td").expect("static selector");
    for table in document.select(&table_selector) {
        let headers = table
            .select(&header_selector)
            .map(|header| header.text().collect::<String>().to_ascii_lowercase())
            .collect::<Vec<_>>();
        if !headers.iter().any(|header| header.contains("stage"))
            || !headers.iter().any(|header| header.contains("time"))
        {
            continue;
        }
        for row in table.select(&row_selector) {
            let cells = row
                .select(&cell_selector)
                .map(|cell| cell.text().collect::<String>().trim().to_owned())
                .collect::<Vec<_>>();
            if cells.len() >= 2 && cells[0].eq_ignore_ascii_case("total") {
                return parse_first_number(&cells[1]);
            }
        }
    }
    let text = document.root_element().text().collect::<Vec<_>>().join(" ");
    Regex::new(r"(?i)\bTotal\b[^0-9]{0,20}([0-9]+(?:\.[0-9]+)?)\b")
        .expect("static regex")
        .captures(&text)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}
