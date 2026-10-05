use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;

use crate::config::{LogStream, ModuleSelection};

#[derive(Debug, Parser)]
#[command(name = "farmcontroller", version, about)]
pub struct Cli {
    /// TOML configuration file.
    #[arg(long, env = "FARMCONTROLLER_CONFIG")]
    pub config: Option<PathBuf>,

    /// Enable modules. May be repeated or comma-separated; `all` enables every module.
    #[arg(long, value_delimiter = ',', value_enum)]
    pub enable: Vec<ModuleSelection>,

    /// Disable modules after defaults, file, and environment configuration.
    #[arg(long, value_delimiter = ',', value_enum)]
    pub disable: Vec<ModuleSelection>,

    /// HTTP listener address.
    #[arg(long, env = "FARMCONTROLLER_BIND")]
    pub bind: Option<SocketAddr>,

    /// Tracing filter, such as `info` or `farmcontroller=debug,tower_http=info`.
    #[arg(long, env = "FARMCONTROLLER_LOG_LEVEL")]
    pub log_level: Option<String>,

    /// Optional log file. Parent directories are created at startup.
    #[arg(long, env = "FARMCONTROLLER_LOG_FILE")]
    pub log_file: Option<PathBuf>,

    /// Emit JSON logs.
    #[arg(long, env = "FARMCONTROLLER_LOG_JSON")]
    pub log_json: bool,

    /// Console stream used for logs.
    #[arg(long, env = "FARMCONTROLLER_LOG_STREAM", value_enum)]
    pub log_stream: Option<LogStream>,

    /// Optional UDP collector address for best-effort network logs.
    #[arg(long, env = "FARMCONTROLLER_LOG_NETWORK")]
    pub log_network: Option<SocketAddr>,
}
