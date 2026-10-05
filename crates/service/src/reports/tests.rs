use tempfile::TempDir;

use super::*;

struct FixtureDataSource;

type TerminalUpdate = (Uuid, ReportStatus, Option<String>);

#[async_trait]
impl ReportDataSource for FixtureDataSource {
    async fn test_cases(&self, _test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError> {
        Ok(vec![
            TestCaseRecord {
                test_case_id: 42,
                suite_name: "sanity".to_owned(),
                result: Some("PASS".to_owned()),
                comment: Some("<verified>".to_owned()),
                command: Some("uname -a".to_owned()),
                jira_defect: None,
                output_file_path: Some("/results/test_42_result.txt".to_owned()),
                labels: None,
            },
            TestCaseRecord {
                test_case_id: 43,
                suite_name: "performance".to_owned(),
                result: Some("PASS".to_owned()),
                comment: Some("stable".to_owned()),
                command: Some("iperf3 -c 192.0.2.1 -t 10".to_owned()),
                jira_defect: Some("https://jira.example/TEST-43".to_owned()),
                output_file_path: Some("/results/test_43_result.txt".to_owned()),
                labels: Some(vec!["Network".to_owned()]),
            },
        ])
    }
}

struct RecordingDataSource {
    terminal: Arc<Mutex<Vec<TerminalUpdate>>>,
}

#[async_trait]
impl ReportDataSource for RecordingDataSource {
    async fn test_cases(&self, _test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError> {
        FixtureDataSource.test_cases("test-1").await
    }

    async fn update_report_status(
        &self,
        report_id: Uuid,
        status: ReportStatus,
        error: Option<&str>,
    ) -> Result<(), ReportError> {
        self.terminal
            .lock()
            .await
            .push((report_id, status, error.map(str::to_owned)));
        Ok(())
    }
}

fn fixture_service(temp: &TempDir) -> ReportService {
    ReportService::new(
        ReportsConfig {
            test_results_dir: temp.path().to_owned(),
            queue_capacity: 4,
            report_base_url: "https://reports.example".to_owned(),
            ..ReportsConfig::default()
        },
        Arc::new(FixtureDataSource),
        Metrics::new().unwrap(),
    )
}

#[tokio::test]
async fn report_worker_generates_escaped_artifacts_and_completes() {
    let temp = TempDir::new().unwrap();
    let sanity_results = temp.path().join("x5h/1.0.0/test-1/sanity_commands_results");
    tokio::fs::create_dir_all(&sanity_results).await.unwrap();
    tokio::fs::write(
        sanity_results.join("test_42_result.txt"),
        "ready\nWARN low voltage\nERROR <unsafe>\n",
    )
    .await
    .unwrap();
    let performance_results = temp
        .path()
        .join("x5h/1.0.0/test-1/performance_commands_results");
    tokio::fs::create_dir_all(&performance_results)
        .await
        .unwrap();
    tokio::fs::write(
        performance_results.join("test_43_result.txt"),
        "Command: iperf3 -c 192.0.2.1 -t 10\nBandwidth: 95.5\n0.00-10.00 sec 114 MBytes 95.5 Mbits/sec receiver\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        temp.path().join("x5h/1.0.0/test-1/boottime.html"),
        "<table><tr><th>Stage</th><th>Time (s)</th></tr><tr><td>Total</td><td>12.345 s</td></tr></table>",
    )
    .await
    .unwrap();
    let event_publisher = Arc::new(EventHub::default());
    let mut events = event_publisher.subscribe();
    let service = fixture_service(&temp).with_event_publisher(event_publisher);
    let cancellation = CancellationToken::new();
    let handle = service.start_worker(cancellation.clone()).unwrap();
    let request = ReportRequest {
        test_id: "test-1".to_owned(),
        build_version: "1.0.0".to_owned(),
        device_type: "x5h".to_owned(),
        previous_test_id: None,
        previous_build_version: None,
        created_by: Some("user-1".to_owned()),
    };
    let job_id = service.enqueue(request).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if service.status(job_id).await.is_some_and(|job| {
                matches!(job.status, ReportStatus::Completed | ReportStatus::Failed)
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let status = service.status(job_id).await.unwrap();
    assert_eq!(status.status, ReportStatus::Completed, "{:?}", status.error);
    let completed = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.payload["status"] == "completed" {
                break event;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(completed.event, "execution_report_update");
    assert_eq!(completed.payload["reportId"], job_id.to_string());
    assert_eq!(completed.payload["testExecutionId"], "test-1");
    assert_eq!(completed.payload["createdBy"], "user-1");
    assert_eq!(completed.room.as_deref(), Some("user:user-1"));
    let report = tokio::fs::read_to_string(
        temp.path()
            .join("x5h/1.0.0/test-1/sanity_commands_report.html"),
    )
    .await
    .unwrap();
    assert!(report.contains("&lt;verified&gt;"));
    let performance_summary = tokio::fs::read_to_string(
        temp.path()
            .join("x5h/1.0.0/test-1/performance_commands_output.txt"),
    )
    .await
    .unwrap();
    assert!(performance_summary.contains("Test 43 - Module: Network"));
    assert!(performance_summary.contains("Saved to: test_43_result.txt"));
    let performance_report = tokio::fs::read_to_string(
        temp.path()
            .join("x5h/1.0.0/test-1/performance_commands_report.html"),
    )
    .await
    .unwrap();
    assert!(performance_report.contains("<th>Measured Result</th>"));
    assert!(performance_report.contains("https://jira.example/TEST-43"));
    let sanity_analysis = tokio::fs::read_to_string(
        temp.path()
            .join("x5h/1.0.0/test-1/sanity_command_analysis/test_42_result_analysis.html"),
    )
    .await
    .unwrap();
    assert!(sanity_analysis.contains("Total lines: 3 | Errors: 1 | Warnings: 1"));
    assert!(sanity_analysis.contains("ERROR &lt;unsafe&gt;"));
    assert!(!sanity_analysis.contains("ERROR <unsafe>"));
    let performance_comparison = tokio::fs::read_to_string(
        temp.path()
            .join("x5h/1.0.0/test-1/perf_compare/test_43_comp.html"),
    )
    .await
    .unwrap();
    assert!(performance_comparison.contains("Performance Comparison"));
    assert!(performance_comparison.contains("<td>Bandwidth</td><td>95.5</td><td>95.5</td>"));
    for graph in [
        "radar_chart.png",
        "network_bar.png",
        "comparison.csv",
        "comparison.json",
    ] {
        let path = temp
            .path()
            .join("x5h/1.0.0/test-1/perf_compare")
            .join(graph);
        assert!(path.is_file(), "missing graph artifact {graph}");
        assert!(std::fs::metadata(path).unwrap().len() > 8);
    }
    let comparison: serde_json::Value = serde_json::from_slice(
        &tokio::fs::read(
            temp.path()
                .join("x5h/1.0.0/test-1/perf_compare/comparison.json"),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(comparison["old_metrics"]["Network"], 95.5);
    assert_eq!(comparison["new_per_test"]["Network"]["iperf3"], 95.5);
    assert_eq!(comparison["targets"]["Network:iperf3"], 1000.0);
    assert_eq!(comparison["verdicts"]["Network"], "Tie");
    let combined_report =
        tokio::fs::read_to_string(temp.path().join("x5h/1.0.0/test-1/report.html"))
            .await
            .unwrap();
    assert!(combined_report.contains("<h2>Summary:</h2>"));
    assert!(combined_report.contains("100.0%"));
    assert!(combined_report.contains("<td>12.345</td>"));
    assert!(
        combined_report
            .contains("https://reports.example/x5h/1.0.0/test-1/perf_compare/radar_chart.png")
    );
    assert!(
        combined_report
            .contains("https://reports.example/x5h/1.0.0/test-1/perf_compare/network_bar.png")
    );
    assert!(
        combined_report
            .contains("https://reports.example/x5h/1.0.0/test-1/perf_compare/test_43_comp.html")
    );
    assert!(combined_report.contains(
        "https://reports.example/x5h/1.0.0/test-1/sanity_command_analysis/test_42_result_analysis.html"
    ));
    assert!(combined_report.contains("<h2>Other Reports:</h2>"));
    cancellation.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn report_worker_persists_and_publishes_fatal_artifact_failure() {
    let temp = TempDir::new().unwrap();
    let terminal = Arc::new(Mutex::new(Vec::new()));
    let event_publisher = Arc::new(EventHub::default());
    let mut events = event_publisher.subscribe();
    let service = ReportService::new(
        ReportsConfig {
            test_results_dir: temp.path().to_owned(),
            queue_capacity: 1,
            ..ReportsConfig::default()
        },
        Arc::new(RecordingDataSource {
            terminal: terminal.clone(),
        }),
        Metrics::new().unwrap(),
    )
    .with_event_publisher(event_publisher);
    let cancellation = CancellationToken::new();
    let handle = service.start_worker(cancellation.clone()).unwrap();
    let job_id = service
        .enqueue(ReportRequest {
            test_id: "test-1".to_owned(),
            build_version: "1.0.0".to_owned(),
            device_type: "x5h".to_owned(),
            previous_test_id: None,
            previous_build_version: None,
            created_by: Some("user-1".to_owned()),
        })
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if service
                .status(job_id)
                .await
                .is_some_and(|job| job.status == ReportStatus::Failed)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();

    let persisted = terminal.lock().await;
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].0, job_id);
    assert_eq!(persisted[0].1, ReportStatus::Failed);
    assert!(
        persisted[0]
            .2
            .as_deref()
            .is_some_and(|error| { error.contains("Sanity results directory not found") })
    );
    drop(persisted);

    let mut failure_rooms = Vec::new();
    while failure_rooms.len() < 2 {
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.payload["status"] == "failed" {
            assert_eq!(event.payload["reportId"], job_id.to_string());
            assert!(
                event.payload["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("Sanity results directory not found"))
            );
            failure_rooms.push(event.room.unwrap());
        }
    }
    failure_rooms.sort();
    assert_eq!(failure_rooms, ["test:test-1", "user:user-1"]);
    assert!(!temp.path().join("x5h/1.0.0/test-1/report.html").exists());

    cancellation.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn report_paths_reject_traversal() {
    let request = ReportRequest {
        test_id: "../secret".to_owned(),
        build_version: "1".to_owned(),
        device_type: "x5h".to_owned(),
        previous_test_id: None,
        previous_build_version: None,
        created_by: None,
    };
    assert!(matches!(
        validate_request(&request),
        Err(ReportError::Validation(_))
    ));
}

#[tokio::test]
async fn full_queue_rejects_job_without_leaking_status() {
    let temp = TempDir::new().unwrap();
    let service = ReportService::new(
        ReportsConfig {
            test_results_dir: temp.path().to_owned(),
            queue_capacity: 1,
            ..ReportsConfig::default()
        },
        Arc::new(FixtureDataSource),
        Metrics::new().unwrap(),
    );
    let request = ReportRequest {
        test_id: "test-1".to_owned(),
        build_version: "1".to_owned(),
        device_type: "x5h".to_owned(),
        previous_test_id: None,
        previous_build_version: None,
        created_by: None,
    };
    service.enqueue(request.clone()).await.unwrap();
    assert!(matches!(
        service.enqueue(request).await,
        Err(ReportError::QueueFull)
    ));
    assert_eq!(service.jobs.read().await.len(), 1);
}

#[test]
fn confluence_settings_require_page_url_and_credentials() {
    let config = ReportsConfig::default();
    assert!(matches!(
        ConfluenceSettings::from_config(&config, "1.0"),
        Err(ConfluenceError::Configuration(_))
    ));
}

#[test]
fn confluence_html_uses_storage_image_macros() {
    let body =
        confluence_body(r#"<html><body><img src="perf_compare/radar_chart.png"></body></html>"#)
            .unwrap();
    assert_eq!(
        body,
        r#"<ac:image ac:width="600"><ri:attachment ri:filename="radar_chart.png"/></ac:image>"#
    );
}
