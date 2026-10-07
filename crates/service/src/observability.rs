use std::{
    fs,
    io::{self, Write},
    net::UdpSocket,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::{MatchedPath, Request, State},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
    TextEncoder, core::Collector,
};
use tracing::info_span;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::{
    EnvFilter, fmt::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt,
};
use uuid::Uuid;

use crate::{
    config::{LogStream, LoggingConfig},
    error::AppError,
    state::AppState,
};

mod runtime_logging;

pub use runtime_logging::{
    RuntimeLogEntry, recent_runtime_logs, runtime_log_level, set_runtime_log_level,
};

#[derive(Clone)]
pub struct Metrics {
    registry: Registry,
    requests: IntCounterVec,
    duration: HistogramVec,
    request_size: HistogramVec,
    response_size: HistogramVec,
    active: IntGauge,
    failures: IntCounterVec,
    database_operations: IntCounterVec,
    database_duration: HistogramVec,
    worker_runs: IntCounterVec,
    worker_items: IntCounterVec,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new_custom(Some("farmcontroller".to_owned()), None)?;
        let requests = IntCounterVec::new(
            Opts::new(
                "http_requests_total",
                "HTTP requests by method, route, and status",
            ),
            &["method", "route", "status"],
        )?;
        let duration = HistogramVec::new(
            HistogramOpts::new(
                "http_request_duration_seconds",
                "HTTP request duration by method and route",
            ),
            &["method", "route"],
        )?;
        let active = IntGauge::new("http_active_requests", "Currently active HTTP requests")?;
        let request_size =
            HistogramVec::new(
                HistogramOpts::new("http_request_size_bytes", "HTTP request body size").buckets(
                    vec![1_024.0, 16_384.0, 262_144.0, 4_194_304.0, 67_108_864.0],
                ),
                &["method", "route"],
            )?;
        let response_size = HistogramVec::new(
            HistogramOpts::new("http_response_size_bytes", "HTTP response body size").buckets(
                vec![1_024.0, 16_384.0, 262_144.0, 4_194_304.0, 67_108_864.0],
            ),
            &["method", "route"],
        )?;
        let failures = IntCounterVec::new(
            Opts::new(
                "http_failures_total",
                "HTTP failures by bounded authentication, validation, authorization, and server category",
            ),
            &["kind"],
        )?;
        let database_operations = IntCounterVec::new(
            Opts::new(
                "database_operations_total",
                "Database operations by bounded operation and outcome",
            ),
            &["operation", "outcome"],
        )?;
        let database_duration = HistogramVec::new(
            HistogramOpts::new(
                "database_operation_duration_seconds",
                "Database operation duration by bounded operation",
            ),
            &["operation"],
        )?;
        let worker_runs = IntCounterVec::new(
            Opts::new(
                "worker_runs_total",
                "Worker executions by bounded worker and outcome",
            ),
            &["worker", "outcome"],
        )?;
        let worker_items = IntCounterVec::new(
            Opts::new("worker_items_total", "Items processed by bounded worker"),
            &["worker"],
        )?;
        registry.register(Box::new(requests.clone()))?;
        registry.register(Box::new(duration.clone()))?;
        registry.register(Box::new(request_size.clone()))?;
        registry.register(Box::new(response_size.clone()))?;
        registry.register(Box::new(active.clone()))?;
        registry.register(Box::new(failures.clone()))?;
        registry.register(Box::new(database_operations.clone()))?;
        registry.register(Box::new(database_duration.clone()))?;
        registry.register(Box::new(worker_runs.clone()))?;
        registry.register(Box::new(worker_items.clone()))?;
        Ok(Self {
            registry,
            requests,
            duration,
            request_size,
            response_size,
            active,
            failures,
            database_operations,
            database_duration,
            worker_runs,
            worker_items,
        })
    }

    pub fn record_worker_run(&self, worker: &'static str, outcome: &'static str) {
        self.worker_runs.with_label_values(&[worker, outcome]).inc();
    }

    pub fn record_worker_items(&self, worker: &'static str, count: u64) {
        self.worker_items.with_label_values(&[worker]).inc_by(count);
    }

    pub fn record_database_operation(
        &self,
        operation: &'static str,
        outcome: &'static str,
        elapsed: Duration,
    ) {
        self.database_operations
            .with_label_values(&[operation, outcome])
            .inc();
        self.database_duration
            .with_label_values(&[operation])
            .observe(elapsed.as_secs_f64());
    }

    pub fn encode(
        &self,
        uptime: Duration,
        database_pool: Option<DatabasePoolSnapshot>,
    ) -> Result<String, prometheus::Error> {
        let uptime_metric = prometheus::Gauge::with_opts(Opts::new(
            "process_uptime_seconds",
            "FarmController process uptime",
        ))?;
        uptime_metric.set(uptime.as_secs_f64());

        let mut families = self.registry.gather();
        families.extend(uptime_metric.collect());
        if let Some(resident_memory_bytes) = resident_memory_bytes() {
            let memory = IntGauge::with_opts(Opts::new(
                "process_resident_memory_bytes",
                "FarmController resident memory on Linux",
            ))?;
            memory.set(resident_memory_bytes);
            families.extend(memory.collect());
        }
        if let Some(pool) = database_pool {
            let connections = IntGaugeVec::new(
                Opts::new(
                    "database_pool_connections",
                    "PostgreSQL pool connections by bounded state",
                ),
                &["state"],
            )?;
            connections
                .with_label_values(&["open"])
                .set(i64::from(pool.open));
            connections
                .with_label_values(&["idle"])
                .set(i64::try_from(pool.idle).unwrap_or(i64::MAX));
            connections
                .with_label_values(&["max"])
                .set(i64::from(pool.max));
            families.extend(connections.collect());
        }
        let mut output = Vec::new();
        TextEncoder::new().encode(&families, &mut output)?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }
}

#[derive(Clone, Copy)]
pub struct DatabasePoolSnapshot {
    pub open: u32,
    pub idle: usize,
    pub max: u32,
}

fn resident_memory_bytes() -> Option<i64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let kibibytes = status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")?
            .split_whitespace()
            .next()?
            .parse::<i64>()
            .ok()
    })?;
    kibibytes.checked_mul(1024)
}

struct ActiveRequestGuard(IntGauge);

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

pub async fn track_request(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |path| path.as_str().to_owned());
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned);
    let request_size = request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok());
    let started = Instant::now();
    state.metrics.active.inc();
    let _active = ActiveRequestGuard(state.metrics.active.clone());
    let span = info_span!("http.request", %request_id, %method, %route);
    let _entered = span.enter();

    let mut response = next.run(request).await;
    let status = response.status();
    let elapsed = started.elapsed().as_secs_f64();
    state
        .metrics
        .requests
        .with_label_values(&[method.as_str(), &route, status.as_str()])
        .inc();
    state
        .metrics
        .duration
        .with_label_values(&[method.as_str(), &route])
        .observe(elapsed);
    if let Some(size) = request_size {
        state
            .metrics
            .request_size
            .with_label_values(&[method.as_str(), &route])
            .observe(size);
    }
    if let Some(size) = response
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
    {
        state
            .metrics
            .response_size
            .with_label_values(&[method.as_str(), &route])
            .observe(size);
    }
    let failure = match status {
        axum::http::StatusCode::BAD_REQUEST
        | axum::http::StatusCode::UNPROCESSABLE_ENTITY
        | axum::http::StatusCode::PAYLOAD_TOO_LARGE => Some("validation"),
        axum::http::StatusCode::UNAUTHORIZED => Some("authentication"),
        axum::http::StatusCode::FORBIDDEN => Some("authorization"),
        status if status.is_server_error() => Some("server"),
        _ => None,
    };
    if let Some(failure) = failure {
        state.metrics.failures.with_label_values(&[failure]).inc();
    }
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    tracing::info!(duration_seconds = elapsed, status = %status, "request completed");
    response
}

#[derive(Clone)]
struct LogFanout {
    stream: LogStream,
    file: Option<NonBlocking>,
    network: Option<Arc<UdpSocket>>,
}

enum StreamWriter {
    Stdout(io::Stdout),
    Stderr(io::Stderr),
}

struct FanoutWriter {
    stream: Option<StreamWriter>,
    file: Option<NonBlocking>,
    network: Option<Arc<UdpSocket>>,
    network_buffer: Vec<u8>,
}

impl<'a> MakeWriter<'a> for LogFanout {
    type Writer = FanoutWriter;

    fn make_writer(&'a self) -> Self::Writer {
        let stream = match self.stream {
            LogStream::Stdout => Some(StreamWriter::Stdout(io::stdout())),
            LogStream::Stderr => Some(StreamWriter::Stderr(io::stderr())),
            LogStream::Off => None,
        };
        FanoutWriter {
            stream,
            file: self.file.clone(),
            network: self.network.clone(),
            network_buffer: Vec::new(),
        }
    }
}

impl Write for FanoutWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match &mut self.stream {
            Some(StreamWriter::Stdout(stream)) => stream.write_all(bytes)?,
            Some(StreamWriter::Stderr(stream)) => stream.write_all(bytes)?,
            None => {}
        }
        if let Some(file) = &mut self.file {
            file.write_all(bytes)?;
        }
        if self.network.is_some() {
            self.network_buffer.extend_from_slice(bytes);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.stream {
            Some(StreamWriter::Stdout(stream)) => stream.flush()?,
            Some(StreamWriter::Stderr(stream)) => stream.flush()?,
            None => {}
        }
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }
}

impl Drop for FanoutWriter {
    fn drop(&mut self) {
        if !self.network_buffer.is_empty()
            && let Some(network) = &self.network
        {
            let _ = network.send(&self.network_buffer);
        }
    }
}

fn network_writer(address: std::net::SocketAddr) -> Result<Arc<UdpSocket>, AppError> {
    let bind = if address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind)
        .map_err(|error| AppError::Telemetry(format!("cannot bind UDP log socket: {error}")))?;
    socket.connect(address).map_err(|error| {
        AppError::Telemetry(format!("cannot connect UDP log sink {address}: {error}"))
    })?;
    socket.set_nonblocking(true).map_err(|error| {
        AppError::Telemetry(format!("cannot make UDP log sink nonblocking: {error}"))
    })?;
    Ok(Arc::new(socket))
}

pub fn init_logging(config: &LoggingConfig) -> Result<Option<WorkerGuard>, AppError> {
    let filter = EnvFilter::try_new(&config.level)
        .map_err(|error| AppError::Telemetry(format!("invalid log filter: {error}")))?;
    let (filter, capture, runtime_control) = runtime_logging::prepare(filter, &config.level);
    let (file, guard) = match &config.file {
        Some(path) => {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            fs::create_dir_all(parent).map_err(|error| {
                AppError::Telemetry(format!(
                    "cannot create log directory {}: {error}",
                    parent.display()
                ))
            })?;
            let file_name = path.file_name().ok_or_else(|| {
                AppError::Telemetry(format!("log file has no file name: {}", path.display()))
            })?;
            let appender = tracing_appender::rolling::never(parent, file_name);
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (Some(writer), Some(guard))
        }
        None => (None, None),
    };
    let writer = LogFanout {
        stream: config.stream,
        file,
        network: config.network.map(network_writer).transpose()?,
    };
    if config.json {
        tracing_subscriber::registry()
            .with(filter)
            .with(capture)
            .with(tracing_subscriber::fmt::layer().json().with_writer(writer))
            .try_init()
            .map_err(|error| AppError::Telemetry(error.to_string()))?;
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(capture)
            .with(tracing_subscriber::fmt::layer().with_writer(writer))
            .try_init()
            .map_err(|error| AppError::Telemetry(error.to_string()))?;
    }
    runtime_logging::install(runtime_control)?;
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn network_writer_emits_one_nonblocking_datagram_per_event() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let fanout = LogFanout {
            stream: LogStream::Off,
            file: None,
            network: Some(network_writer(receiver.local_addr().unwrap()).unwrap()),
        };
        {
            let mut writer = fanout.make_writer();
            writer.write_all(b"{\"message\":\"ready\"}\n").unwrap();
        }
        let mut buffer = [0_u8; 128];
        let length = receiver.recv(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"{\"message\":\"ready\"}\n");
    }
}
