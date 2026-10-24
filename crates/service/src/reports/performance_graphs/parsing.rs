//! Benchmark classification and metric extraction for performance graphing.

use regex::Regex;

use super::super::performance_metrics;

#[derive(Clone, Copy)]
pub(super) struct Benchmark {
    pub(super) name: &'static str,
    pub(super) target: f64,
}

/// Canonical category.
pub(super) fn canonical_category(value: &str) -> Option<&'static str> {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "canonical_category");
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

/// Identify benchmark.
pub(super) fn identify_benchmark(command: &str) -> Option<Benchmark> {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "identify_benchmark");
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

/// Derive label.
pub(super) fn derive_label(command: &str, benchmark: &str, raw_category: &str) -> String {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "derive_label");
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

/// Numbered label.
pub(super) fn numbered_label(base: &str, sequence: usize, benchmark: Option<&str>) -> String {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "numbered_label");
    match benchmark {
        Some("stress-ng") => format!("{base} run{sequence}"),
        Some("sysbench") => format!("{base} v{sequence}"),
        _ => format!("{base} ({sequence})"),
    }
}

/// Metric value.
pub(super) fn metric_value(content: &str, benchmark: Option<Benchmark>) -> f64 {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "metric_value");
    match benchmark.map(|value| value.name) {
        Some("iperf3" | "iperf") => extract_iperf_mbps(content),
        Some("dhrystone") => extract_dhrystone(content),
        Some("mbw") => extract_mbw(content),
        Some("glmark2") => extract_glmark2(content),
        _ => performance_metrics(content).0.values().sum(),
    }
}

/// Extract iperf mbps.
fn extract_iperf_mbps(content: &str) -> f64 {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "extract_iperf_mbps");
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

/// Extract dhrystone.
fn extract_dhrystone(content: &str) -> f64 {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "extract_dhrystone");
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

/// Extract mbw.
fn extract_mbw(content: &str) -> f64 {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "extract_mbw");
    Regex::new(r"(?m)^AVG\s+Method:\s*MEMCPY.*?Copy:\s*([0-9]+(?:\.[0-9]+)?)\s*MiB/s")
        .expect("static regex")
        .captures(content)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
        .unwrap_or(0.0)
}

/// Extract glmark2.
fn extract_glmark2(content: &str) -> f64 {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "extract_glmark2");
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

/// Average.
fn average(values: impl Iterator<Item = f64>) -> f64 {
    let _api_trace = crate::observability::ApiTrace::enter("reports::performance_graphs::parsing", "average");
    let values = values.collect::<Vec<_>>();
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}
