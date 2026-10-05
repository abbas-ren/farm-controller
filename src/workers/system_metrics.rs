use std::{sync::Arc, time::Duration};

use chrono::{SecondsFormat, Utc};
use sysinfo::{Disks, Networks, System};
use tokio_util::sync::CancellationToken;

use crate::events::{EventHub, ServerEvent};

const METRICS_INTERVAL: Duration = Duration::from_secs(5);

pub(super) async fn worker(event_publisher: Arc<EventHub>, cancellation: CancellationToken) {
    worker_with_interval(event_publisher, cancellation, METRICS_INTERVAL).await;
}

async fn worker_with_interval(
    event_publisher: Arc<EventHub>,
    cancellation: CancellationToken,
    interval: Duration,
) {
    let mut sampler = SystemMetricsSampler::new();
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = ticker.tick() => {
                if let Err(error) = event_publisher.publish(sampler.sample_event()) {
                    tracing::warn!(%error, worker = "system_metrics", "failed to publish system metrics");
                }
            }
        }
    }
    tracing::info!(worker = "system_metrics", "worker stopped");
}

struct SystemMetricsSampler {
    system: System,
    disks: Disks,
    networks: Networks,
}

impl SystemMetricsSampler {
    fn new() -> Self {
        Self {
            system: System::new_all(),
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
        }
    }

    fn sample_event(&mut self) -> ServerEvent {
        self.system.refresh_cpu_all();
        self.system.refresh_memory();
        self.disks.refresh(true);
        self.networks.refresh(true);

        let cpu_usage = f64::from(self.system.global_cpu_usage());
        let cpu_total_ghz = self
            .system
            .cpus()
            .iter()
            .map(sysinfo::Cpu::frequency)
            .max()
            .unwrap_or_default() as f64
            / 1000.0;
        let memory_used = self.system.used_memory();
        let memory_total = self.system.total_memory();
        let disk_total = self
            .disks
            .iter()
            .map(sysinfo::Disk::total_space)
            .sum::<u64>();
        let disk_used = self
            .disks
            .iter()
            .map(|disk| disk.total_space().saturating_sub(disk.available_space()))
            .sum::<u64>();
        let network_upload = self
            .networks
            .values()
            .map(|data| data.transmitted())
            .sum::<u64>();
        let network_download = self
            .networks
            .values()
            .map(|data| data.received())
            .sum::<u64>();

        ServerEvent {
            event: "system_metrics_update".to_owned(),
            payload: serde_json::json!({
                "cpu": {
                    "current": format!("{:.1} GHz", cpu_total_ghz * cpu_usage / 100.0),
                    "total": format!("{cpu_total_ghz:.1} GHz"),
                    "usagePercent": cpu_usage.round(),
                },
                "memory": {
                    "used": format_bytes(memory_used),
                    "total": format_bytes(memory_total),
                    "usagePercent": percentage(memory_used, memory_total),
                },
                "network": {
                    "upload": format_speed(network_upload),
                    "download": format_speed(network_download),
                },
                "disk": {
                    "used": format_bytes(disk_used),
                    "total": format_bytes(disk_total),
                    "usagePercent": percentage(disk_used, disk_total),
                },
                "timestamp": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            }),
            room: None,
        }
    }
}

fn percentage(used: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        ((used as f64 / total as f64) * 100.0).round() as u64
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    const TB: f64 = 1_000_000_000_000.0;
    let bytes = bytes as f64;
    if bytes >= TB {
        format!("{:.1} TB", bytes / TB)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

fn format_speed(bytes_per_second: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_second))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_payload_preserves_frontend_contract() {
        let event = SystemMetricsSampler::new().sample_event();
        assert_eq!(event.event, "system_metrics_update");
        assert!(event.room.is_none());
        for section in ["cpu", "memory", "network", "disk"] {
            assert!(event.payload[section].is_object(), "{section}");
        }
        assert!(event.payload["cpu"]["current"].is_string());
        assert!(event.payload["cpu"]["total"].is_string());
        assert!(event.payload["cpu"]["usagePercent"].is_number());
        assert!(event.payload["memory"]["used"].is_string());
        assert!(event.payload["memory"]["total"].is_string());
        assert!(event.payload["memory"]["usagePercent"].is_number());
        assert!(event.payload["network"]["upload"].is_string());
        assert!(event.payload["network"]["download"].is_string());
        assert!(event.payload["disk"]["used"].is_string());
        assert!(event.payload["disk"]["total"].is_string());
        assert!(event.payload["disk"]["usagePercent"].is_number());
        assert!(
            event.payload["timestamp"]
                .as_str()
                .is_some_and(|value| { chrono::DateTime::parse_from_rfc3339(value).is_ok() })
        );
    }

    #[test]
    fn metric_units_match_legacy_decimal_formatting() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(2_500_000), "2.5 MB");
        assert_eq!(format_speed(1_500), "1.5 KB/s");
        assert_eq!(percentage(1, 4), 25);
        assert_eq!(percentage(1, 0), 0);
    }

    #[tokio::test]
    async fn metrics_worker_is_bounded_and_cancellable() {
        let event_publisher = Arc::new(EventHub::default());
        let mut events = event_publisher.subscribe();
        let cancellation = CancellationToken::new();
        let handle = tokio::spawn(worker_with_interval(
            event_publisher,
            cancellation.clone(),
            Duration::from_millis(1),
        ));
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.event, "system_metrics_update");
        cancellation.cancel();
        handle.await.unwrap();
    }
}
