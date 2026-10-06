//! Per-test comparison artifacts and parsing of benchmark result text.

use std::{collections::HashMap, path::Path};

use regex::Regex;

use super::{ReportRequest, artifact_writers::escape_html, validation::report_root};

pub(crate) async fn write_performance_comparisons(
    results_root: &Path,
    current_root: &Path,
    request: &ReportRequest,
) -> Result<(), std::io::Error> {
    let current = current_root.join("performance_commands_results");
    if !current.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "Performance results directory not found: {}",
                current.display()
            ),
        ));
    }
    let previous_request = ReportRequest {
        test_id: request
            .previous_test_id
            .clone()
            .unwrap_or_else(|| request.test_id.clone()),
        build_version: request
            .previous_build_version
            .clone()
            .unwrap_or_else(|| request.build_version.clone()),
        device_type: request.device_type.clone(),
        previous_test_id: None,
        previous_build_version: None,
        created_by: None,
    };
    let previous_root = report_root(results_root, &previous_request)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let previous = previous_root.join("performance_commands_results");

    let mut entries = tokio::fs::read_dir(&current).await?;
    let mut inputs = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_file()
            && name.ends_with(".txt")
            && !name.to_ascii_lowercase().contains("cleanup")
        {
            inputs.push(path);
        }
    }
    inputs.sort();
    if inputs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("No .txt performance files found in: {}", current.display()),
        ));
    }

    let output = current_root.join("perf_compare");
    tokio::fs::create_dir_all(&output).await?;
    let old_label = format!(
        "{}/{}",
        previous_request.build_version, previous_request.test_id
    );
    let new_label = format!("{}/{}", request.build_version, request.test_id);
    for input in inputs {
        let file_name = input
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("performance.txt");
        let current_content = tokio::fs::read_to_string(&input).await?;
        let (new_metrics, new_command) = performance_metrics(&current_content);
        let previous_path = previous.join(file_name);
        let (old_metrics, old_command) = if previous_path.is_file() {
            performance_metrics(&tokio::fs::read_to_string(previous_path).await?)
        } else {
            (new_metrics.clone(), new_command.clone())
        };
        let command = if old_command == new_command {
            new_command
        } else {
            format!("{old_command} vs {new_command}")
        };
        let mut keys = old_metrics
            .keys()
            .filter(|key| new_metrics.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        let base = file_name
            .strip_suffix("_result.txt")
            .or_else(|| file_name.strip_suffix(".txt"))
            .unwrap_or(file_name);
        let chart_name = format!("{base}_comp.png");
        if !keys.is_empty() {
            let chart_path = output.join(&chart_name);
            let chart_keys = keys.clone();
            let chart_old = chart_keys
                .iter()
                .map(|key| old_metrics[key])
                .collect::<Vec<_>>();
            let chart_new = chart_keys
                .iter()
                .map(|key| new_metrics[key])
                .collect::<Vec<_>>();
            let chart_old_label = old_label.clone();
            let chart_new_label = new_label.clone();
            tokio::task::spawn_blocking(move || {
                render_performance_comparison(
                    &chart_path,
                    &chart_keys,
                    &chart_old,
                    &chart_new,
                    &chart_old_label,
                    &chart_new_label,
                )
            })
            .await
            .map_err(std::io::Error::other)??;
        }
        let html = if keys.is_empty() {
            format!(
                "<!doctype html>\n<html><head><title>{base}_comp Skipped</title></head>\n<body>\n  <h2>&#9888; Skipped: {}</h2>\n  <p>No common performance metrics between\n     <strong>{}</strong> and <strong>{}</strong>.</p>\n  <p><strong>Command:</strong> {}</p>\n</body></html>",
                escape_html(file_name),
                escape_html(&old_label),
                escape_html(&new_label),
                escape_html(&command),
            )
        } else {
            let rows = keys
                .iter()
                .map(|key| {
                    format!(
                        "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                        escape_html(key),
                        old_metrics[key],
                        new_metrics[key]
                    )
                })
                .collect::<String>();
            format!(
                "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\" />\n  <title>{base}_comp Comparison</title>\n  <style>\n    body {{ font-family: Arial, sans-serif; margin: 20px; }}\n    h2, h3 {{ color: #2c3e50; }}\n    table {{ border-collapse: collapse; }}\n    th, td {{ border: 1px solid #ccc; padding: 6px 10px; }}\n    th {{ background: #f4f4f4; }}\n  </style>\n</head>\n<body>\n  <h2>Performance Comparison &mdash; {}</h2>\n  <p><strong>Previous:</strong> {} &nbsp;&nbsp;\n     <strong>Current:</strong> {}</p>\n  <p><img src=\"{}\" alt=\"{}\" style=\"max-width:1000px;\"/></p>\n  <h3>Detailed Metrics</h3>\n  <table>\n    <tr><th>Metric</th><th>{}</th><th>{}</th></tr>\n    {rows}\n  </table>\n</body>\n</html>",
                escape_html(&command),
                escape_html(&old_label),
                escape_html(&new_label),
                escape_html(&chart_name),
                escape_html(&chart_name),
                escape_html(&old_label),
                escape_html(&new_label),
            )
        };
        tokio::fs::write(output.join(format!("{base}_comp.html")), html).await?;
    }
    Ok(())
}

fn render_performance_comparison(
    path: &Path,
    labels: &[String],
    old_values: &[f64],
    new_values: &[f64],
    old_label: &str,
    new_label: &str,
) -> Result<(), std::io::Error> {
    use plotters::prelude::*;

    let path = path.to_string_lossy().into_owned();
    let root = BitMapBackend::new(&path, (1200, 650)).into_drawing_area();
    root.fill(&WHITE)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let maximum = old_values
        .iter()
        .chain(new_values)
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let slots = labels.len() * 2;
    let mut chart = ChartBuilder::on(&root)
        .caption(
            format!("Performance Comparison - {old_label} vs {new_label}"),
            ("sans-serif", 28),
        )
        .margin(20)
        .x_label_area_size(90)
        .y_label_area_size(80)
        .build_cartesian_2d(0..slots, 0.0..maximum * 1.15)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    chart
        .configure_mesh()
        .x_labels(labels.len())
        .x_label_formatter(&|slot| labels.get(*slot / 2).cloned().unwrap_or_default())
        .draw()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    chart
        .draw_series(old_values.iter().enumerate().map(|(index, value)| {
            Rectangle::new([(index * 2, 0.0), (index * 2 + 1, *value)], BLUE.filled())
        }))
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    chart
        .draw_series(new_values.iter().enumerate().map(|(index, value)| {
            Rectangle::new(
                [(index * 2 + 1, 0.0), (index * 2 + 2, *value)],
                RED.filled(),
            )
        }))
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    root.present()
        .map_err(|error| std::io::Error::other(error.to_string()))
}

pub(crate) fn performance_metrics(content: &str) -> (HashMap<String, f64>, String) {
    let lines = content.lines().collect::<Vec<_>>();
    let mut metrics = HashMap::new();
    let mut command = "Unknown".to_owned();
    for line in &lines {
        if let Some(value) = line.strip_prefix("Command:") {
            command = value.trim().to_owned();
        } else if let Some((key, value)) = line.split_once(':')
            && let Ok(value) = value.trim().parse::<f64>()
        {
            metrics.insert(key.trim().to_owned(), value);
        }
    }
    if let Some(total) = lines.iter().find(|line| line.trim().starts_with("Total")) {
        let values = total.split('|').map(str::trim).collect::<Vec<_>>();
        if let Some(value) = values.get(1).and_then(|value| value.parse::<f64>().ok()) {
            metrics.insert("Total_DMIPS".to_owned(), value);
        }
        if values.len() == 3
            && let Ok(value) = values[2].parse::<f64>()
        {
            metrics.insert("Total_DMIPS_per_MHz".to_owned(), value);
        }
    }
    if let Some(value) = lines.iter().rev().find_map(|line| {
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        (tokens.len() >= 5)
            .then(|| tokens.last()?.parse::<f64>().ok())
            .flatten()
    }) {
        metrics.insert("Throughput".to_owned(), value);
    }
    let speed = Regex::new(r"\(([\d.]+) MiB/sec\)").expect("static regex is valid");
    if let Some(value) = lines.iter().rev().find_map(|line| {
        speed
            .captures(line)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse::<f64>().ok())
    }) {
        metrics.insert("Write_Speed_MiB_per_sec".to_owned(), value);
    }
    (metrics, command)
}
