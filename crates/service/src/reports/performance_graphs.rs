use std::{collections::BTreeMap, io, path::Path};

use serde::Serialize;

mod parsing;
mod rendering;

use super::{ReportRequest, TestCaseRecord, file_name, report_root};
use parsing::{canonical_category, derive_label, identify_benchmark, metric_value, numbered_label};
use rendering::{render_bars, render_radar};

const CATEGORY_ORDER: [&str; 6] = ["Network", "Storage", "CPU", "Memory", "DMA", "GPU"];

#[derive(Clone, Debug, Serialize)]
struct CategoryMetrics {
    category: String,
    labels: Vec<String>,
    old_values: Vec<f64>,
    new_values: Vec<f64>,
    old_score: f64,
    new_score: f64,
    old_capability: f64,
    new_capability: f64,
    verdict: String,
}

#[derive(Clone)]
struct MetricEntry {
    label: String,
    old_value: f64,
    new_value: f64,
    target: f64,
    target_name: Option<&'static str>,
}

pub(super) async fn write(
    results_root: &Path,
    current_root: &Path,
    request: &ReportRequest,
    cases: &[TestCaseRecord],
) -> io::Result<()> {
    let current_dir = current_root.join("performance_commands_results");
    if !current_dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Performance results directory not found: {}",
                current_dir.display()
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
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let previous_dir = previous_root.join("performance_commands_results");

    let performance_cases = cases
        .iter()
        .filter(|case| case.suite_name.eq_ignore_ascii_case("performance"))
        .filter_map(|case| {
            let raw_category = case
                .labels
                .as_ref()
                .and_then(|labels| labels.first())
                .map(String::as_str)?;
            let category = canonical_category(raw_category)?;
            let command = case.command.as_deref().unwrap_or("");
            let benchmark = identify_benchmark(command);
            let base_label = benchmark.map_or_else(
                || category.to_ascii_lowercase(),
                |benchmark| derive_label(command, benchmark.name, raw_category),
            );
            Some((case, category, benchmark, base_label))
        })
        .collect::<Vec<_>>();
    let mut label_counts = BTreeMap::<(String, String), usize>::new();
    for (_, category, _, base_label) in &performance_cases {
        *label_counts
            .entry(((*category).to_owned(), base_label.clone()))
            .or_default() += 1;
    }
    let mut label_sequences = BTreeMap::<(String, String), usize>::new();
    let mut grouped: BTreeMap<String, Vec<MetricEntry>> = BTreeMap::new();
    for (case, category, benchmark, base_label) in performance_cases {
        let count = label_counts
            .get(&(category.to_owned(), base_label.clone()))
            .copied()
            .unwrap_or(1);
        let label = if count > 1 {
            let sequence = label_sequences
                .entry((category.to_owned(), base_label.clone()))
                .or_insert(0);
            *sequence += 1;
            numbered_label(&base_label, *sequence, benchmark.map(|value| value.name))
        } else {
            base_label
        };
        let name = file_name(case.output_file_path.as_deref(), case.test_case_id);
        let current_path = current_dir.join(&name);
        let current_content = if current_path.is_file() {
            tokio::fs::read_to_string(&current_path).await?
        } else {
            String::new()
        };
        let current_score = metric_value(&current_content, benchmark);
        let previous_path = previous_dir.join(&name);
        let previous_score = if previous_path.is_file() {
            metric_value(&tokio::fs::read_to_string(previous_path).await?, benchmark)
        } else {
            current_score
        };
        grouped
            .entry(category.to_owned())
            .or_default()
            .push(MetricEntry {
                label,
                old_value: previous_score,
                new_value: current_score,
                target: benchmark.map_or(0.0, |value| value.target),
                target_name: benchmark.map(|value| value.name),
            });
    }
    if grouped.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "No categorized performance result files found",
        ));
    }

    let old_label = format!(
        "{}/{}",
        previous_request.build_version, previous_request.test_id
    );
    let new_label = format!("{}/{}", request.build_version, request.test_id);
    let mut categories = Vec::new();
    let mut targets = BTreeMap::new();
    for category in CATEGORY_ORDER {
        let Some(values) = grouped.remove(category) else {
            continue;
        };
        for entry in &values {
            if entry.target > 0.0
                && let Some(name) = entry.target_name
            {
                targets.insert(format!("{category}:{name}"), entry.target);
            }
        }
        let old_score = average(values.iter().map(|entry| entry.old_value));
        let new_score = average(values.iter().map(|entry| entry.new_value));
        let old_capability = average(values.iter().map(|entry| {
            normalized_capability(
                entry.old_value,
                entry.old_value,
                entry.new_value,
                entry.target,
            )
        }));
        let new_capability = average(values.iter().map(|entry| {
            normalized_capability(
                entry.new_value,
                entry.old_value,
                entry.new_value,
                entry.target,
            )
        }));
        let verdict = if old_score > new_score {
            format!("{old_label} performs better")
        } else if new_score > old_score {
            format!("{new_label} performs better")
        } else if old_score == 0.0 {
            "No data".to_owned()
        } else {
            "Tie".to_owned()
        };
        let (labels, old_values, new_values) = ordered_values(category, &values);
        categories.push(CategoryMetrics {
            category: category.to_owned(),
            labels,
            old_values,
            new_values,
            old_score,
            new_score,
            old_capability,
            new_capability,
            verdict,
        });
    }

    let output = current_root.join("perf_compare");
    tokio::fs::create_dir_all(&output).await?;
    let mut csv = "Category,OldScore,NewScore,Verdict\n".to_owned();
    for category in &categories {
        csv.push_str(&format!(
            "{},{:.6},{:.6},{}\n",
            category.category, category.old_score, category.new_score, category.verdict
        ));
    }
    tokio::fs::write(output.join("comparison.csv"), csv).await?;
    let old_metrics = categories
        .iter()
        .map(|category| (category.category.clone(), category.old_score))
        .collect::<BTreeMap<_, _>>();
    let new_metrics = categories
        .iter()
        .map(|category| (category.category.clone(), category.new_score))
        .collect::<BTreeMap<_, _>>();
    let old_per_test = categories
        .iter()
        .map(|category| {
            (
                category.category.clone(),
                category
                    .labels
                    .iter()
                    .cloned()
                    .zip(category.old_values.iter().copied())
                    .collect::<BTreeMap<_, _>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let new_per_test = categories
        .iter()
        .map(|category| {
            (
                category.category.clone(),
                category
                    .labels
                    .iter()
                    .cloned()
                    .zip(category.new_values.iter().copied())
                    .collect::<BTreeMap<_, _>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let verdicts = categories
        .iter()
        .map(|category| (category.category.clone(), category.verdict.clone()))
        .collect::<BTreeMap<_, _>>();
    tokio::fs::write(
        output.join("comparison.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "old_label": old_label,
            "new_label": new_label,
            "old_metrics": old_metrics,
            "new_metrics": new_metrics,
            "old_per_test": old_per_test,
            "new_per_test": new_per_test,
            "targets": targets,
            "verdicts": verdicts,
            "categories": categories,
            "timestamp": chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
        }))
        .map_err(io::Error::other)?,
    )
    .await?;

    let render_output = output.clone();
    let render_categories = categories.clone();
    tokio::task::spawn_blocking(move || {
        render_radar(&render_output, &render_categories, &old_label, &new_label)?;
        for category in &render_categories {
            render_bars(&render_output, category, &old_label, &new_label)?;
        }
        Ok::<(), io::Error>(())
    })
    .await
    .map_err(io::Error::other)??;
    Ok(())
}

fn normalized_capability(value: f64, old: f64, new: f64, target: f64) -> f64 {
    let target = if target > 0.0 {
        target
    } else {
        old.max(new).max(1.0)
    };
    (value / target).clamp(0.0, 1.0)
}

fn fixed_labels(category: &str) -> &'static [&'static str] {
    match category {
        "Network" => &["netperf", "iperf3"],
        "Storage" => &[
            "fio emmc 1G",
            "iozone",
            "fio ufs 1G",
            "sysbench fileio v1",
            "fio emmc 4G",
            "fio emmc 8G",
            "fio ufs 4G",
            "fio emmc 16G",
            "sysbench fileio v2",
        ],
        "CPU" => &[
            "stress-ng cpu run1",
            "sysbench cpu",
            "dhrystone",
            "stress-ng cpu run3",
            "sysbench threads",
            "stress-ng cpu run2",
            "stress-ng switch",
            "coremark",
        ],
        "Memory" => &[
            "mbw memcpy 3MB",
            "mbw memcpy 4MB",
            "mbw memcpy 2MB",
            "mbw memcpy larger",
            "sysbench memory",
            "mbw memcpy 1MB",
            "mbw memcpy 5MB",
        ],
        "DMA" => &["dma"],
        "GPU" => &["gpu"],
        _ => &[],
    }
}

fn ordered_values(category: &str, entries: &[MetricEntry]) -> (Vec<String>, Vec<f64>, Vec<f64>) {
    if matches!(category, "DMA" | "GPU") {
        return (
            vec![fixed_labels(category)[0].to_owned()],
            vec![average(entries.iter().map(|entry| entry.old_value))],
            vec![average(entries.iter().map(|entry| entry.new_value))],
        );
    }
    let mut values = entries
        .iter()
        .map(|entry| (entry.label.clone(), (entry.old_value, entry.new_value)))
        .collect::<BTreeMap<_, _>>();
    let mut labels = fixed_labels(category)
        .iter()
        .map(|label| (*label).to_owned())
        .collect::<Vec<_>>();
    let extras = values
        .keys()
        .filter(|label| !labels.contains(label))
        .cloned()
        .collect::<Vec<_>>();
    labels.extend(extras);
    let pairs = labels
        .iter()
        .map(|label| values.remove(label).unwrap_or((0.0, 0.0)))
        .collect::<Vec<_>>();
    (
        labels,
        pairs.iter().map(|pair| pair.0).collect(),
        pairs.iter().map(|pair| pair.1).collect(),
    )
}

fn average(values: impl Iterator<Item = f64>) -> f64 {
    let values = values.collect::<Vec<_>>();
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_follow_legacy_aliases() {
        assert_eq!(canonical_category("File System"), Some("Storage"));
        assert_eq!(canonical_category("systembench"), Some("CPU"));
        assert_eq!(canonical_category("custom"), None);
    }
}
