use std::{collections::BTreeMap, io, path::Path};

use plotters::prelude::*;
use regex::Regex;
use serde::Serialize;

use super::{ReportRequest, TestCaseRecord, file_name, performance_metrics, report_root};

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

#[derive(Clone, Copy)]
struct Benchmark {
    name: &'static str,
    target: f64,
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

fn canonical_category(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "network" => Some("Network"),
        "storage" | "file system" | "filesystem" | "ufs" | "emmc" => Some("Storage"),
        "cpu" | "system bench" | "systembench" | "sysbench" => Some("CPU"),
        "memory" => Some("Memory"),
        "dma" => Some("DMA"),
        "gpu" => Some("GPU"),
        _ => None,
    }
}

fn identify_benchmark(command: &str) -> Option<Benchmark> {
    let command = command.to_ascii_lowercase();
    [
        Benchmark {
            name: "iperf3",
            target: 1000.0,
        },
        Benchmark {
            name: "iperf",
            target: 1000.0,
        },
        Benchmark {
            name: "netperf",
            target: 1000.0,
        },
        Benchmark {
            name: "dhrystone",
            target: 14.0,
        },
        Benchmark {
            name: "mbw",
            target: 195_312.5,
        },
        Benchmark {
            name: "glmark2",
            target: 6500.0,
        },
        Benchmark {
            name: "coremark",
            target: 0.0,
        },
        Benchmark {
            name: "stress-ng",
            target: 0.0,
        },
        Benchmark {
            name: "sysbench",
            target: 0.0,
        },
        Benchmark {
            name: "fio",
            target: 0.0,
        },
        Benchmark {
            name: "iozone",
            target: 0.0,
        },
    ]
    .into_iter()
    .find(|benchmark| match benchmark.name {
        "iperf3" => command.contains("iperf3"),
        "iperf" => command.split_whitespace().any(|part| part == "iperf"),
        "dhrystone" => command.contains("dhrystone") || command.contains("dhry_per_core"),
        "glmark2" => command.contains("glmark2"),
        name => command.contains(name),
    })
}

fn derive_label(command: &str, benchmark: &str, raw_category: &str) -> String {
    let lower = command.to_ascii_lowercase();
    match benchmark {
        "sysbench" => ["cpu", "memory", "fileio", "mutex", "threads", "oltp"]
            .into_iter()
            .find(|kind| lower.contains(&format!("sysbench {kind}")))
            .map_or_else(|| "sysbench".to_owned(), |kind| format!("sysbench {kind}")),
        "fio" => {
            let device = if lower.contains("nvme") {
                Some("nvme")
            } else if lower.contains("mmc") || raw_category.eq_ignore_ascii_case("emmc") {
                Some("emmc")
            } else if lower.contains("ufs")
                || Regex::new(r"/dev/sd[a-z]")
                    .expect("static regex")
                    .is_match(&lower)
                || raw_category.eq_ignore_ascii_case("ufs")
            {
                Some("ufs")
            } else if lower.contains("shm") || lower.contains("mem") {
                Some("mem")
            } else {
                None
            };
            let size = Regex::new(r"--size(?:=|\s+)(\d+[gmkt]?)\b")
                .expect("static regex")
                .captures(&lower)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().to_ascii_uppercase());
            [Some("fio".to_owned()), device.map(str::to_owned), size]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        }
        "stress-ng" => Regex::new(r"--([a-z0-9-]+)\s+\d+")
            .expect("static regex")
            .captures_iter(&lower)
            .filter_map(|captures| captures.get(1).map(|value| value.as_str()))
            .find(|value| {
                !matches!(
                    *value,
                    "timeout" | "backoff" | "times" | "timer-slack" | "taskset"
                )
            })
            .map_or_else(
                || "stress-ng".to_owned(),
                |kind| format!("stress-ng {kind}"),
            ),
        "mbw" => {
            let method = Regex::new(r"-t\s+(memcpy|dumb|mcblock)\b")
                .expect("static regex")
                .captures(&lower)
                .and_then(|captures| captures.get(1))
                .map_or("memcpy", |value| value.as_str());
            let size = Regex::new(r"\s(\d+)\s*$")
                .expect("static regex")
                .captures(&lower)
                .and_then(|captures| captures.get(1))
                .and_then(|value| value.as_str().parse::<u32>().ok())
                .map_or_else(
                    || "larger".to_owned(),
                    |value| {
                        if value <= 5 {
                            format!("{value}MB")
                        } else {
                            "larger".to_owned()
                        }
                    },
                );
            format!("mbw {method} {size}")
        }
        "glmark2" => "gpu".to_owned(),
        _ => benchmark.to_owned(),
    }
}

fn numbered_label(base: &str, sequence: usize, benchmark: Option<&str>) -> String {
    match benchmark {
        Some("stress-ng") => format!("{base} run{sequence}"),
        Some("sysbench") => format!("{base} v{sequence}"),
        _ => format!("{base} ({sequence})"),
    }
}

fn metric_value(content: &str, benchmark: Option<Benchmark>) -> f64 {
    match benchmark.map(|value| value.name) {
        Some("iperf3" | "iperf") => extract_iperf_mbps(content),
        Some("dhrystone") => extract_dhrystone(content),
        Some("mbw") => extract_mbw(content),
        Some("glmark2") => extract_glmark2(content),
        _ => performance_metrics(content).0.values().sum(),
    }
}

fn extract_iperf_mbps(content: &str) -> f64 {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content)
        && let Some(bits) = value
            .pointer("/end/sum_received/bits_per_second")
            .or_else(|| value.pointer("/end/sum/bits_per_second"))
            .and_then(serde_json::Value::as_f64)
    {
        return bits / 1_000_000.0;
    }
    for role in ["receiver", "sender"] {
        let pattern =
            format!(r"(?is)\b0\.00-\s*\d+(?:\.\d+)?\s*sec.*?(\d+(?:\.\d+)?)\s*Mbits/sec\s*{role}");
        if let Some(value) = Regex::new(&pattern)
            .expect("static role produces a valid regex")
            .captures(content)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse().ok())
        {
            return value;
        }
    }
    0.0
}

fn extract_dhrystone(content: &str) -> f64 {
    let row =
        Regex::new(r"(?m)^\s*\d+\s*\|\s*[0-9]+(?:\.[0-9]+)?\s*\|\s*([0-9]+(?:\.[0-9]+)?)\s*$")
            .expect("static regex");
    let values = row
        .captures_iter(content)
        .filter_map(|captures| captures.get(1)?.as_str().parse::<f64>().ok())
        .collect::<Vec<_>>();
    if !values.is_empty() {
        return values.iter().sum::<f64>() / values.len() as f64;
    }
    Regex::new(r"(?m)^\s*Total\s*\|\s*[0-9]+(?:\.[0-9]+)?\s*\|\s*([0-9]+(?:\.[0-9]+)?)\s*$")
        .expect("static regex")
        .captures(content)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
        .unwrap_or(0.0)
}

fn extract_mbw(content: &str) -> f64 {
    Regex::new(r"(?m)^AVG\s+Method:\s*MEMCPY.*?Copy:\s*([0-9]+(?:\.[0-9]+)?)\s*MiB/s")
        .expect("static regex")
        .captures(content)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
        .unwrap_or(0.0)
}

fn extract_glmark2(content: &str) -> f64 {
    if let Some(score) = Regex::new(r"(?i)\bglmark2\s+Score:\s*([0-9]+(?:\.[0-9]+)?)\b")
        .expect("static regex")
        .captures(content)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
    {
        return score;
    }
    average(
        Regex::new(r"\bFPS:\s*([0-9]+(?:\.[0-9]+)?)\b")
            .expect("static regex")
            .captures_iter(content)
            .filter_map(|captures| captures.get(1)?.as_str().parse().ok()),
    )
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

fn render_radar(
    output: &Path,
    categories: &[CategoryMetrics],
    old_label: &str,
    new_label: &str,
) -> io::Result<()> {
    let path = output.join("radar_chart.png");
    let path = path.to_string_lossy().into_owned();
    let root = BitMapBackend::new(&path, (1200, 700)).into_drawing_area();
    root.fill(&WHITE).map_err(plot_error)?;
    let center = (350_i32, 350_i32);
    let radius = 240.0_f64;
    for ring in 1..=5 {
        let ring_radius = radius * f64::from(ring) / 5.0;
        root.draw(&Circle::new(
            center,
            ring_radius.round() as i32,
            ShapeStyle::from(&RGBColor(210, 210, 210)),
        ))
        .map_err(plot_error)?;
    }
    let count = categories.len();
    let points = |old: bool| {
        categories
            .iter()
            .enumerate()
            .map(|(index, category)| {
                let angle = std::f64::consts::TAU * index as f64 / count as f64
                    - std::f64::consts::FRAC_PI_2;
                let value = if old {
                    category.old_capability
                } else {
                    category.new_capability
                };
                (
                    center.0 + (angle.cos() * radius * value).round() as i32,
                    center.1 + (angle.sin() * radius * value).round() as i32,
                )
            })
            .collect::<Vec<_>>()
    };
    for (index, category) in categories.iter().enumerate() {
        let angle =
            std::f64::consts::TAU * index as f64 / count as f64 - std::f64::consts::FRAC_PI_2;
        let end = (
            center.0 + (angle.cos() * radius).round() as i32,
            center.1 + (angle.sin() * radius).round() as i32,
        );
        root.draw(&PathElement::new(vec![center, end], BLACK.mix(0.25)))
            .map_err(plot_error)?;
        root.draw(&Text::new(
            category.category.clone(),
            (
                center.0 + (angle.cos() * (radius + 40.0)).round() as i32,
                center.1 + (angle.sin() * (radius + 40.0)).round() as i32,
            ),
            ("sans-serif", 22).into_font(),
        ))
        .map_err(plot_error)?;
    }
    let mut old_points = points(true);
    old_points.push(old_points[0]);
    let mut new_points = points(false);
    new_points.push(new_points[0]);
    root.draw(&Polygon::new(old_points.clone(), BLUE.mix(0.18).filled()))
        .map_err(plot_error)?;
    root.draw(&PathElement::new(old_points, BLUE.stroke_width(3)))
        .map_err(plot_error)?;
    root.draw(&Polygon::new(new_points.clone(), RED.mix(0.18).filled()))
        .map_err(plot_error)?;
    root.draw(&PathElement::new(new_points, RED.stroke_width(3)))
        .map_err(plot_error)?;
    root.draw(&Text::new(
        format!("Blue: {old_label}    Red: {new_label}"),
        (700, 50),
        ("sans-serif", 24).into_font(),
    ))
    .map_err(plot_error)?;
    for (index, category) in categories.iter().enumerate() {
        root.draw(&Text::new(
            format!(
                "{}: {:.1}% vs {:.1}% ({})",
                category.category,
                category.old_capability * 100.0,
                category.new_capability * 100.0,
                category.verdict
            ),
            (700, 100 + index as i32 * 55),
            ("sans-serif", 19).into_font(),
        ))
        .map_err(plot_error)?;
    }
    root.present().map_err(plot_error)
}

fn render_bars(
    output: &Path,
    category: &CategoryMetrics,
    old_label: &str,
    new_label: &str,
) -> io::Result<()> {
    let name = format!(
        "{}_bar.png",
        category.category.to_ascii_lowercase().replace(' ', "_")
    );
    let path = output.join(name);
    let path = path.to_string_lossy().into_owned();
    let root = BitMapBackend::new(&path, (1000, 520)).into_drawing_area();
    root.fill(&WHITE).map_err(plot_error)?;
    let maximum = category
        .old_values
        .iter()
        .chain(&category.new_values)
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let slots = category.labels.len() * 2;
    let mut chart = ChartBuilder::on(&root)
        .caption(
            format!("{} - {old_label} vs {new_label}", category.category),
            ("sans-serif", 24),
        )
        .margin(20)
        .x_label_area_size(70)
        .y_label_area_size(70)
        .build_cartesian_2d(0..slots, 0.0..maximum * 1.15)
        .map_err(plot_error)?;
    chart
        .configure_mesh()
        .x_labels(category.labels.len())
        .x_label_formatter(&|slot| category.labels.get(*slot / 2).cloned().unwrap_or_default())
        .draw()
        .map_err(plot_error)?;
    chart
        .draw_series(
            category
                .old_values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    Rectangle::new([(index * 2, 0.0), (index * 2 + 1, *value)], BLUE.filled())
                }),
        )
        .map_err(plot_error)?;
    chart
        .draw_series(
            category
                .new_values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    Rectangle::new(
                        [(index * 2 + 1, 0.0), (index * 2 + 2, *value)],
                        RED.filled(),
                    )
                }),
        )
        .map_err(plot_error)?;
    root.present().map_err(plot_error)
}

fn plot_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
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
