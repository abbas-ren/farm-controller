# Observability

Structured tracing supports text or JSON output, selectable `stdout`, `stderr`, or `off` streams, an optional bounded nonblocking file, and an optional best-effort nonblocking UDP collector. Configure `logging.level`, `logging.json`, `logging.stream`, `logging.file`, and `logging.network`; never place credentials in log messages. CLI aliases are `--log-level`, `--log-json`, `--log-stream`, `--log-file`, and `--log-network`.

Every HTTP response carries a request ID. Request traces include method, matched route, status, latency, and error context. Worker failures identify the worker and operation without using device/user IDs as metric labels.

HTTP metrics use matched route templates, never raw URLs. They cover counts, status, duration, active requests, known request/response sizes, and bounded validation/authentication/authorization/server failure categories. Worker and database counters cover outcomes and durations. Linux scrapes also include resident memory and uptime; database-enabled processes expose open, idle, and maximum PostgreSQL pool connections.

Prometheus scrapes the shared listener:

```yaml
scrape_configs:
  - job_name: farmcontroller
    static_configs:
      - targets: ['farmcontroller:3000']
    metrics_path: /metrics
```

Readiness includes configured dependency checks and should gate traffic. Health reports process liveness only. Alert on sustained non-ready state, HTTP 5xx rate, worker failure counters, queue saturation, and database acquisition failures. Raw IDs, tokens, request bodies, and arbitrary URLs must not be metric labels.

Import `dashboards/farmcontroller.json` directly or provision it through Grafana's dashboard provider. Select the Prometheus datasource through the dashboard variable; no query edits are required.

Production logging should use structured JSON on stdout/stderr or the nonblocking file sink and let systemd-journald, Fluent Bit, Vector, or Promtail forward it. Where direct delivery is required, set `logging.network` to a UDP collector address such as `127.0.0.1:5514`; one formatted event is sent per datagram and delivery is deliberately best-effort so collector backpressure cannot block application work.