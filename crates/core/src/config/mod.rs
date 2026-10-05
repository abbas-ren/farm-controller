#[cfg(test)]
pub mod tests;

use std::{collections::BTreeSet, net::SocketAddr, path::PathBuf};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{cli::Cli, error::AppError};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Module {
    Api,
    Auth,
    Device,
    Reports,
    Workers,
    Events,
    Metrics,
    Swagger,
    Health,
}

impl Module {
    pub const ALL: [Self; 9] = [
        Self::Api,
        Self::Auth,
        Self::Device,
        Self::Reports,
        Self::Workers,
        Self::Events,
        Self::Metrics,
        Self::Swagger,
        Self::Health,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ModuleSelection {
    All,
    Api,
    Auth,
    Device,
    Reports,
    Workers,
    Events,
    Metrics,
    Swagger,
    Health,
}

impl ModuleSelection {
    fn modules(self) -> Vec<Module> {
        match self {
            Self::All => Module::ALL.to_vec(),
            Self::Api => vec![Module::Api],
            Self::Auth => vec![Module::Auth],
            Self::Device => vec![Module::Device],
            Self::Reports => vec![Module::Reports],
            Self::Workers => vec![Module::Workers],
            Self::Events => vec![Module::Events],
            Self::Metrics => vec![Module::Metrics],
            Self::Swagger => vec![Module::Swagger],
            Self::Health => vec![Module::Health],
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub modules: ModulesConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub device: DeviceConfig,
    #[serde(default)]
    pub workers: WorkersConfig,
    #[serde(default)]
    pub reports: ReportsConfig,
    #[serde(default)]
    pub tests: TestsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TestsConfig {
    #[serde(default)]
    pub test_rail_base_url: String,
    #[serde(default)]
    pub test_rail_api_version: String,
    #[serde(default)]
    pub test_rail_username: String,
    #[serde(default)]
    pub test_rail_api_key: String,
    #[serde(default)]
    pub test_rail_project_id: u64,
    #[serde(default = "default_test_rail_timeout")]
    pub test_rail_timeout_seconds: u64,
    #[serde(default)]
    pub qmetry_base_url: String,
    #[serde(default)]
    pub qmetry_api_key: String,
    #[serde(default)]
    pub jira_project_id: String,
    #[serde(default)]
    pub jira_base_url: String,
    #[serde(default = "default_jira_endpoint")]
    pub jira_endpoint: String,
    #[serde(default)]
    pub jira_project_key: String,
    #[serde(default)]
    pub jira_api_token: String,
    #[serde(default = "default_test_rail_timeout")]
    pub jira_timeout_seconds: u64,
    #[serde(default)]
    pub jira_script_base_url: String,
    #[serde(default = "default_test_rail_timeout")]
    pub qmetry_timeout_seconds: u64,
    #[serde(default = "default_test_logs_dir")]
    pub test_logs_dir: String,
    #[serde(default = "default_test_script_server")]
    pub test_script_server: String,
    #[serde(default = "default_local_test_script_url")]
    pub local_test_script_server_url: String,
    #[serde(default)]
    pub gitlab_base_url: String,
    #[serde(default)]
    pub gitlab_project_id: String,
    #[serde(default)]
    pub gitlab_access_token: String,
    #[serde(default = "default_gitlab_branch")]
    pub gitlab_branch: String,
    #[serde(default = "default_test_script_max_bytes")]
    pub test_script_max_bytes: usize,
}

impl Default for TestsConfig {
    fn default() -> Self {
        Self {
            test_rail_base_url: String::new(),
            test_rail_api_version: String::new(),
            test_rail_username: String::new(),
            test_rail_api_key: String::new(),
            test_rail_project_id: 0,
            test_rail_timeout_seconds: default_test_rail_timeout(),
            qmetry_base_url: String::new(),
            qmetry_api_key: String::new(),
            jira_project_id: String::new(),
            jira_base_url: String::new(),
            jira_endpoint: default_jira_endpoint(),
            jira_project_key: String::new(),
            jira_api_token: String::new(),
            jira_timeout_seconds: default_test_rail_timeout(),
            jira_script_base_url: String::new(),
            qmetry_timeout_seconds: default_test_rail_timeout(),
            test_logs_dir: default_test_logs_dir(),
            test_script_server: default_test_script_server(),
            local_test_script_server_url: default_local_test_script_url(),
            gitlab_base_url: String::new(),
            gitlab_project_id: String::new(),
            gitlab_access_token: String::new(),
            gitlab_branch: default_gitlab_branch(),
            test_script_max_bytes: default_test_script_max_bytes(),
        }
    }
}

fn default_test_rail_timeout() -> u64 {
    30
}

fn default_jira_endpoint() -> String {
    "rest/api/2".to_owned()
}

fn default_test_logs_dir() -> String {
    "../log_data/test/logs".to_owned()
}

fn default_test_script_server() -> String {
    "gitlab".to_owned()
}

fn default_local_test_script_url() -> String {
    "http://127.0.0.1:8081".to_owned()
}

fn default_gitlab_branch() -> String {
    "main".to_owned()
}

fn default_test_script_max_bytes() -> usize {
    10 * 1024 * 1024
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceConfig {
    #[serde(default)]
    pub artifacts_base_url: String,
    #[serde(default = "default_artifacts_temp_dir")]
    pub artifacts_temp_dir: String,
    #[serde(default = "default_nfs_host_path")]
    pub nfs_host_path: String,
    #[serde(default = "default_tftp_output_dir")]
    pub tftp_output_dir: String,
    #[serde(default = "default_build_upload_dir")]
    pub build_upload_dir: String,
    #[serde(default = "default_build_upload_max_bytes")]
    pub build_upload_max_bytes: u64,
    #[serde(default = "default_build_artifacts_dir")]
    pub build_artifacts_dir: String,
    #[serde(default = "default_faulty_report_upload_dir")]
    pub faulty_report_upload_dir: String,
    #[serde(default = "default_faulty_report_max_bytes")]
    pub faulty_report_max_bytes: u64,
    #[serde(default = "default_device_server_ip")]
    pub server_ip: String,
    #[serde(default = "default_device_request_timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default = "default_terminal_username")]
    pub terminal_username: String,
    #[serde(default)]
    pub terminal_password: String,
    #[serde(default = "default_rtos_username")]
    pub rtos_username: String,
    #[serde(default)]
    pub rtos_password: String,
    #[serde(default)]
    pub terminal_known_hosts_file: String,
    #[serde(default = "default_accept_unknown_host_keys")]
    pub terminal_accept_unknown_host_keys: bool,
    #[serde(default = "default_ipl_artifacts_dir")]
    pub ipl_artifacts_dir: String,
    #[serde(default = "default_gen5_ipl_script")]
    pub gen5_ipl_script: String,
    #[serde(default = "default_ipl_transfer_timeout")]
    pub ipl_transfer_timeout_seconds: u64,
    #[serde(default)]
    pub gen4_ipl_username: String,
    #[serde(default)]
    pub gen4_ipl_password: String,
    #[serde(default = "default_ssh_port")]
    pub gen4_ipl_port: u16,
    #[serde(default = "default_gen4_ipl_remote_path")]
    pub gen4_ipl_remote_path: String,
    #[serde(default)]
    pub gen5_ipl_username: String,
    #[serde(default)]
    pub gen5_ipl_password: String,
    #[serde(default = "default_ssh_port")]
    pub gen5_ipl_port: u16,
    #[serde(default = "default_gen5_ipl_remote_path")]
    pub gen5_ipl_remote_path: String,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            artifacts_base_url: String::new(),
            artifacts_temp_dir: default_artifacts_temp_dir(),
            nfs_host_path: default_nfs_host_path(),
            tftp_output_dir: default_tftp_output_dir(),
            build_upload_dir: default_build_upload_dir(),
            build_upload_max_bytes: default_build_upload_max_bytes(),
            build_artifacts_dir: default_build_artifacts_dir(),
            faulty_report_upload_dir: default_faulty_report_upload_dir(),
            faulty_report_max_bytes: default_faulty_report_max_bytes(),
            server_ip: default_device_server_ip(),
            request_timeout_seconds: default_device_request_timeout(),
            terminal_username: default_terminal_username(),
            terminal_password: String::new(),
            rtos_username: default_rtos_username(),
            rtos_password: String::new(),
            terminal_known_hosts_file: String::new(),
            terminal_accept_unknown_host_keys: default_accept_unknown_host_keys(),
            ipl_artifacts_dir: default_ipl_artifacts_dir(),
            gen5_ipl_script: default_gen5_ipl_script(),
            ipl_transfer_timeout_seconds: default_ipl_transfer_timeout(),
            gen4_ipl_username: String::new(),
            gen4_ipl_password: String::new(),
            gen4_ipl_port: default_ssh_port(),
            gen4_ipl_remote_path: default_gen4_ipl_remote_path(),
            gen5_ipl_username: String::new(),
            gen5_ipl_password: String::new(),
            gen5_ipl_port: default_ssh_port(),
            gen5_ipl_remote_path: default_gen5_ipl_remote_path(),
        }
    }
}

fn default_artifacts_temp_dir() -> String {
    "./tmp/artifacts".to_owned()
}

fn default_nfs_host_path() -> String {
    "/nfs_share".to_owned()
}

fn default_tftp_output_dir() -> String {
    "./tftp_data".to_owned()
}

fn default_build_upload_dir() -> String {
    "./tmp/uploads".to_owned()
}

fn default_build_upload_max_bytes() -> u64 {
    500 * 1024 * 1024
}

fn default_build_artifacts_dir() -> String {
    "/artifacts".to_owned()
}

fn default_faulty_report_upload_dir() -> String {
    "./uploads/faulty-reports".to_owned()
}

fn default_faulty_report_max_bytes() -> u64 {
    10 * 1024 * 1024
}

fn default_device_server_ip() -> String {
    "127.0.0.1".to_owned()
}

fn default_device_request_timeout() -> u64 {
    2
}

fn default_terminal_username() -> String {
    "root".to_owned()
}

fn default_rtos_username() -> String {
    "hpcbdc".to_owned()
}

fn default_accept_unknown_host_keys() -> bool {
    true
}

fn default_ipl_artifacts_dir() -> String {
    "/Racer_IPL_flash".to_owned()
}

fn default_gen5_ipl_script() -> String {
    "/Racer_IPL_flash/gen5_ipl.sh".to_owned()
}

fn default_ipl_transfer_timeout() -> u64 {
    120
}

fn default_ssh_port() -> u16 {
    22
}

fn default_gen4_ipl_remote_path() -> String {
    "/home/hpcbdc/RACER_IPL".to_owned()
}

fn default_gen5_ipl_remote_path() -> String {
    "/home/racer/RACER_IPL".to_owned()
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub request_body_limit_bytes: usize,
    pub cors_allowed_origins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:3000"
                .parse()
                .expect("static socket address is valid"),
            request_body_limit_bytes: 10 * 1024 * 1024,
            cors_allowed_origins: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModulesConfig {
    #[serde(default = "all_modules", rename = "enable")]
    pub enabled: BTreeSet<Module>,
    #[serde(default, rename = "disable")]
    disabled: BTreeSet<Module>,
}

fn all_modules() -> BTreeSet<Module> {
    Module::ALL.into_iter().collect()
}

fn default_modules() -> BTreeSet<Module> {
    [
        Module::Api,
        Module::Metrics,
        Module::Swagger,
        Module::Health,
    ]
    .into_iter()
    .collect()
}

impl Default for ModulesConfig {
    fn default() -> Self {
        Self {
            enabled: default_modules(),
            disabled: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthConfig {
    pub keycloak_url: String,
    pub realm: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub user_role: String,
    pub default_user_password: String,
    pub request_timeout_seconds: u64,
    pub secure_cookies: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            keycloak_url: "http://127.0.0.1:9000".to_owned(),
            realm: "dev-realm".to_owned(),
            client_id: "dev-auth".to_owned(),
            client_secret: None,
            user_role: "user".to_owned(),
            default_user_password: "Renesas123".to_owned(),
            request_timeout_seconds: 10,
            secure_cookies: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseConfig {
    pub url: Option<String>,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub max_lifetime_seconds: u64,
    pub run_migrations: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: None,
            max_connections: 20,
            min_connections: 2,
            acquire_timeout_seconds: 10,
            idle_timeout_seconds: 600,
            max_lifetime_seconds: 1800,
            run_migrations: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkersConfig {
    pub artifact_scan_poll_seconds: u64,
    pub artifact_scan_request_timeout_seconds: u64,
    pub artifact_scan_compute_sha256: bool,
    pub artifact_scan_cleanup_removed: ArtifactCleanup,
    pub retention_interval_seconds: u64,
    pub retention_max_batches_per_policy: u32,
    pub relay_sync_queue_capacity: usize,
    pub relay_config_timeout_seconds: u64,
    pub relay_config_poll_seconds: u64,
    pub relay_config_batch_size: u32,
    pub controller_heartbeat_timeout_seconds: u64,
    pub controller_heartbeat_poll_seconds: u64,
    pub controller_heartbeat_batch_size: u32,
    pub device_heartbeat_poll_seconds: u64,
    pub device_heartbeat_batch_size: u32,
    pub device_action_stale_timeout_minutes: u64,
    pub device_action_watchdog_poll_seconds: u64,
    pub device_action_watchdog_batch_size: u32,
    pub test_preparation_poll_seconds: u64,
    pub test_preparation_lease_seconds: u64,
    pub test_preparation_batch_size: u32,
    pub device_action_poll_seconds: u64,
    pub device_action_batch_size: u32,
    pub gen5_mapping_poll_seconds: u64,
    pub gen5_mapping_batch_size: u32,
    pub gen5_mapping_request_timeout_seconds: u64,
    pub gen5_mapping_max_window_minutes: u64,
    pub gen5_reboot_poll_seconds: u64,
    pub gen5_reboot_timeout_seconds: u64,
    pub gen5_reboot_request_timeout_seconds: u64,
    pub gen5_reboot_batch_size: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactCleanup {
    Off,
    Soft,
    #[default]
    Delete,
}

impl Default for WorkersConfig {
    fn default() -> Self {
        Self {
            artifact_scan_poll_seconds: 60,
            artifact_scan_request_timeout_seconds: 10,
            artifact_scan_compute_sha256: true,
            artifact_scan_cleanup_removed: ArtifactCleanup::Delete,
            retention_interval_seconds: 3600,
            retention_max_batches_per_policy: 10,
            relay_sync_queue_capacity: 128,
            relay_config_timeout_seconds: 300,
            relay_config_poll_seconds: 10,
            relay_config_batch_size: 100,
            controller_heartbeat_timeout_seconds: 30,
            controller_heartbeat_poll_seconds: 5,
            controller_heartbeat_batch_size: 100,
            device_heartbeat_poll_seconds: 1,
            device_heartbeat_batch_size: 100,
            device_action_stale_timeout_minutes: 120,
            device_action_watchdog_poll_seconds: 60,
            device_action_watchdog_batch_size: 100,
            test_preparation_poll_seconds: 5,
            test_preparation_lease_seconds: 600,
            test_preparation_batch_size: 4,
            device_action_poll_seconds: 5,
            device_action_batch_size: 8,
            gen5_mapping_poll_seconds: 10,
            gen5_mapping_batch_size: 10,
            gen5_mapping_request_timeout_seconds: 10,
            gen5_mapping_max_window_minutes: 30,
            gen5_reboot_poll_seconds: 30,
            gen5_reboot_timeout_seconds: 120,
            gen5_reboot_request_timeout_seconds: 10,
            gen5_reboot_batch_size: 20,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReportsConfig {
    pub test_results_dir: PathBuf,
    pub queue_capacity: usize,
    #[serde(default)]
    pub report_base_url: String,
    pub confluence_url: String,
    pub confluence_pat: String,
    pub confluence_space: String,
    pub confluence_parent_title: String,
    pub confluence_subpage_title: String,
    pub confluence_timeout_seconds: u64,
}

impl Default for ReportsConfig {
    fn default() -> Self {
        Self {
            test_results_dir: PathBuf::from("/test-results"),
            queue_capacity: 64,
            report_base_url: String::new(),
            confluence_url: String::new(),
            confluence_pat: String::new(),
            confluence_space: String::new(),
            confluence_parent_title: String::new(),
            confluence_subpage_title: String::new(),
            confluence_timeout_seconds: 60,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub json: bool,
    #[serde(default)]
    pub file: Option<PathBuf>,
    #[serde(default)]
    pub stream: LogStream,
    #[serde(default)]
    pub network: Option<SocketAddr>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    #[default]
    Stdout,
    Stderr,
    Off,
}

fn default_log_level() -> String {
    "info".to_owned()
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            json: false,
            file: None,
            stream: LogStream::Stdout,
            network: None,
        }
    }
}

impl AppConfig {
    pub fn load(cli: &Cli) -> Result<Self, AppError> {
        let mut builder = config::Config::builder();
        if let Some(path) = &cli.config {
            if !path.is_file() {
                return Err(AppError::Configuration(format!(
                    "configuration file does not exist: {}",
                    path.display()
                )));
            }
            builder = builder.add_source(config::File::from(path.clone()).required(true));
        }

        builder = builder.add_source(
            config::Environment::with_prefix("FARMCONTROLLER")
                .prefix_separator("__")
                .separator("__")
                .try_parsing(true)
                .list_separator(",")
                .with_list_parse_key("modules.enable")
                .with_list_parse_key("modules.disable")
                .with_list_parse_key("server.cors_allowed_origins"),
        );

        let mut loaded: Self = builder
            .build()
            .map_err(|error| AppError::Configuration(error.to_string()))?
            .try_deserialize()
            .map_err(|error| AppError::Configuration(error.to_string()))?;

        loaded
            .modules
            .enabled
            .retain(|module| !loaded.modules.disabled.contains(module));
        for selection in &cli.enable {
            loaded.modules.enabled.extend(selection.modules());
        }
        for selection in &cli.disable {
            for module in selection.modules() {
                loaded.modules.enabled.remove(&module);
            }
        }
        if let Some(bind) = cli.bind {
            loaded.server.bind = bind;
        }
        if let Some(level) = &cli.log_level {
            loaded.logging.level.clone_from(level);
        }
        if let Some(file) = &cli.log_file {
            loaded.logging.file = Some(file.clone());
        }
        if cli.log_json {
            loaded.logging.json = true;
        }
        if let Some(stream) = cli.log_stream {
            loaded.logging.stream = stream;
        }
        if let Some(network) = cli.log_network {
            loaded.logging.network = Some(network);
        }

        loaded.validate()?;
        Ok(loaded)
    }

    pub fn module_enabled(&self, module: Module) -> bool {
        self.modules.enabled.contains(&module)
    }

    pub fn database_required(&self) -> bool {
        [Module::Device, Module::Reports, Module::Workers]
            .into_iter()
            .any(|module| self.module_enabled(module))
    }

    fn validate(&self) -> Result<(), AppError> {
        if self.logging.stream == LogStream::Off
            && self.logging.file.is_none()
            && self.logging.network.is_none()
        {
            return Err(AppError::Configuration(
                "at least one logging stream, file, or network sink must be enabled".to_owned(),
            ));
        }
        if self.server.request_body_limit_bytes == 0 {
            return Err(AppError::Configuration(
                "server.request_body_limit_bytes must be greater than zero".to_owned(),
            ));
        }
        for origin in &self.server.cors_allowed_origins {
            let uri = origin.parse::<axum::http::Uri>().map_err(|_| {
                AppError::Configuration(format!(
                    "server.cors_allowed_origins contains invalid origin {origin:?}"
                ))
            })?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none()
                || uri.path() != "/"
                || uri.query().is_some()
            {
                return Err(AppError::Configuration(format!(
                    "server.cors_allowed_origins must contain only HTTP(S) origins without paths: {origin:?}"
                )));
            }
        }
        for (name, value) in [
            ("auth.keycloak_url", self.auth.keycloak_url.as_str()),
            (
                "device.artifacts_base_url",
                self.device.artifacts_base_url.as_str(),
            ),
            (
                "tests.test_rail_base_url",
                self.tests.test_rail_base_url.as_str(),
            ),
            ("tests.qmetry_base_url", self.tests.qmetry_base_url.as_str()),
            ("tests.jira_base_url", self.tests.jira_base_url.as_str()),
            (
                "tests.jira_script_base_url",
                self.tests.jira_script_base_url.as_str(),
            ),
            (
                "tests.local_test_script_server_url",
                self.tests.local_test_script_server_url.as_str(),
            ),
            ("tests.gitlab_base_url", self.tests.gitlab_base_url.as_str()),
            (
                "reports.confluence_url",
                self.reports.confluence_url.as_str(),
            ),
        ] {
            crate::external_http::validate_configured_url(name, value)
                .map_err(AppError::Configuration)?;
        }
        if self.device.faulty_report_max_bytes == 0 {
            return Err(AppError::Configuration(
                "device.faulty_report_max_bytes must be greater than zero".to_owned(),
            ));
        }
        if self.device.ipl_transfer_timeout_seconds == 0
            || self.device.gen4_ipl_port == 0
            || self.device.gen5_ipl_port == 0
        {
            return Err(AppError::Configuration(
                "device IPL transfer timeout and ports must be greater than zero".to_owned(),
            ));
        }
        for (family, username, password, remote_path) in [
            (
                "Gen4",
                &self.device.gen4_ipl_username,
                &self.device.gen4_ipl_password,
                &self.device.gen4_ipl_remote_path,
            ),
            (
                "Gen5",
                &self.device.gen5_ipl_username,
                &self.device.gen5_ipl_password,
                &self.device.gen5_ipl_remote_path,
            ),
        ] {
            if !username.trim().is_empty()
                && (password.is_empty() || !remote_path.trim().starts_with('/'))
            {
                return Err(AppError::Configuration(format!(
                    "device {family} IPL password and absolute remote path are required when its username is configured"
                )));
            }
        }
        if !self.device.terminal_accept_unknown_host_keys
            && (!self.device.gen4_ipl_username.trim().is_empty()
                || !self.device.gen5_ipl_username.trim().is_empty())
            && self.device.terminal_known_hosts_file.trim().is_empty()
        {
            return Err(AppError::Configuration(
                "device.terminal_known_hosts_file is required for IPL distribution when unknown SSH host keys are rejected"
                    .to_owned(),
            ));
        }
        if self.module_enabled(Module::Events) {
            if self.device.terminal_username.trim().is_empty()
                || self.device.rtos_username.trim().is_empty()
            {
                return Err(AppError::Configuration(
                    "device terminal usernames must not be empty when events are enabled"
                        .to_owned(),
                ));
            }
            if !self.device.terminal_accept_unknown_host_keys
                && self.device.terminal_known_hosts_file.trim().is_empty()
            {
                return Err(AppError::Configuration(
                    "device.terminal_known_hosts_file is required when unknown SSH host keys are rejected"
                        .to_owned(),
                ));
            }
        }
        if self.module_enabled(Module::Auth) {
            if self.auth.keycloak_url.trim().is_empty()
                || self.auth.realm.trim().is_empty()
                || self.auth.client_id.trim().is_empty()
            {
                return Err(AppError::Configuration(
                    "auth keycloak_url, realm, and client_id must not be empty".to_owned(),
                ));
            }
            if self.auth.client_secret.as_deref().is_none_or(str::is_empty) {
                return Err(AppError::Configuration(
                    "auth.client_secret is required when auth is enabled".to_owned(),
                ));
            }
            if self.auth.request_timeout_seconds == 0 {
                return Err(AppError::Configuration(
                    "auth.request_timeout_seconds must be greater than zero".to_owned(),
                ));
            }
        }
        if self.database.min_connections > self.database.max_connections {
            return Err(AppError::Configuration(
                "database.min_connections must not exceed database.max_connections".to_owned(),
            ));
        }
        if self.database.max_connections == 0
            || self.database.acquire_timeout_seconds == 0
            || self.database.idle_timeout_seconds == 0
            || self.database.max_lifetime_seconds == 0
        {
            return Err(AppError::Configuration(
                "database pool sizes and timeouts must be greater than zero".to_owned(),
            ));
        }
        if self.workers.artifact_scan_poll_seconds == 0
            || self.workers.artifact_scan_request_timeout_seconds == 0
            || self.workers.retention_interval_seconds == 0
            || self.workers.retention_max_batches_per_policy == 0
            || self.workers.relay_sync_queue_capacity == 0
            || self.workers.relay_config_timeout_seconds == 0
            || self.workers.relay_config_poll_seconds == 0
            || self.workers.relay_config_batch_size == 0
            || self.workers.controller_heartbeat_timeout_seconds == 0
            || self.workers.controller_heartbeat_poll_seconds == 0
            || self.workers.controller_heartbeat_batch_size == 0
            || self.workers.device_heartbeat_poll_seconds == 0
            || self.workers.device_heartbeat_batch_size == 0
            || self.workers.device_action_stale_timeout_minutes == 0
            || self.workers.device_action_watchdog_poll_seconds == 0
            || self.workers.device_action_watchdog_batch_size == 0
            || self.workers.test_preparation_poll_seconds == 0
            || self.workers.test_preparation_lease_seconds == 0
            || self.workers.test_preparation_batch_size == 0
            || self.workers.device_action_poll_seconds == 0
            || self.workers.device_action_batch_size == 0
            || self.workers.gen5_mapping_poll_seconds == 0
            || self.workers.gen5_mapping_batch_size == 0
            || self.workers.gen5_mapping_request_timeout_seconds == 0
            || self.workers.gen5_mapping_max_window_minutes == 0
            || self.workers.gen5_reboot_poll_seconds == 0
            || self.workers.gen5_reboot_timeout_seconds == 0
            || self.workers.gen5_reboot_request_timeout_seconds == 0
            || self.workers.gen5_reboot_batch_size == 0
        {
            return Err(AppError::Configuration(
                "worker intervals, batch limits, and queue capacities must be greater than zero"
                    .to_owned(),
            ));
        }
        if self.reports.queue_capacity == 0 {
            return Err(AppError::Configuration(
                "reports.queue_capacity must be greater than zero".to_owned(),
            ));
        }
        if self.reports.confluence_timeout_seconds == 0 {
            return Err(AppError::Configuration(
                "reports.confluence_timeout_seconds must be greater than zero".to_owned(),
            ));
        }
        let jira_values = [
            self.tests.jira_base_url.trim(),
            self.tests.jira_project_key.trim(),
            self.tests.jira_api_token.trim(),
        ];
        if jira_values.iter().any(|value| !value.is_empty())
            && jira_values.iter().any(|value| value.is_empty())
        {
            return Err(AppError::Configuration(
                "tests jira_base_url, jira_project_key, and jira_api_token must be configured together"
                    .to_owned(),
            ));
        }
        if self.tests.jira_timeout_seconds == 0 {
            return Err(AppError::Configuration(
                "tests.jira_timeout_seconds must be greater than zero".to_owned(),
            ));
        }
        for dependent in [Module::Auth, Module::Device, Module::Reports] {
            if self.module_enabled(dependent) && !self.module_enabled(Module::Api) {
                return Err(AppError::Configuration(format!(
                    "{dependent:?} requires the api module"
                )));
            }
        }
        if self.module_enabled(Module::Workers)
            && !self.module_enabled(Module::Device)
            && !self.module_enabled(Module::Reports)
        {
            return Err(AppError::Configuration(
                "workers require the device or reports module".to_owned(),
            ));
        }
        if self.module_enabled(Module::Reports) && !self.module_enabled(Module::Workers) {
            return Err(AppError::Configuration(
                "reports require the workers module".to_owned(),
            ));
        }
        if self.database_required()
            && self
                .database
                .url
                .as_deref()
                .is_none_or(|url| url.trim().is_empty())
        {
            return Err(AppError::Configuration(
                "database.url is required when device, reports, or workers are enabled".to_owned(),
            ));
        }
        Ok(())
    }
}
