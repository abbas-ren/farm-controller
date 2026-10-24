//! Configuration merge and validation regression tests.
//!
//! Exercises CLI enable/disable precedence, module dependency rules, database
//! URL requirements, CORS origin shape, and external URL credential rejection.

use clap::Parser;

use super::*;

/// PURPOSE: Prove `--disable` removes modules even if also listed in `--enable`.
/// USE: regression for CLI module enable/disable merge in AppConfig::load.
#[test]
fn cli_disable_wins_over_enable() {
    let cli = Cli::try_parse_from([
        "farmcontroller",
        "--enable",
        "health",
        "--disable",
        "health",
    ])
    .unwrap();
    let config = AppConfig::load(&cli).unwrap();

    assert!(!config.module_enabled(Module::Health));
    assert!(config.module_enabled(Module::Api));
}

/// PURPOSE: Device (and other domain modules) cannot run without the API module.
/// USE: guards module dependency validation in AppConfig.
#[test]
fn domain_module_requires_api() {
    let cli =
        Cli::try_parse_from(["farmcontroller", "--enable", "device", "--disable", "api"]).unwrap();

    let error = AppConfig::load(&cli).unwrap_err();
    assert!(error.to_string().contains("requires the api module"));
}

/// PURPOSE: Enabling device without `database.url` must fail validation.
/// USE: guards database_required checks for persistence-backed modules.
#[test]
fn persistence_module_requires_database_url() {
    let cli = Cli::try_parse_from(["farmcontroller", "--enable", "api,device"]).unwrap();

    let error = AppConfig::load(&cli).unwrap_err();
    assert!(error.to_string().contains("database.url is required"));
}

/// PURPOSE: Events alone must not require a database URL.
/// USE: ensures events-only deployments stay DB-optional.
#[test]
fn events_module_does_not_require_persistence() {
    let cli = Cli::try_parse_from(["farmcontroller", "--enable", "events"]).unwrap();

    let config = AppConfig::load(&cli).unwrap();
    assert!(config.module_enabled(Module::Events));
    assert!(!config.database_required());
}

/// PURPOSE: CORS entries must be bare HTTP(S) origins (no path/query).
/// USE: regression for CORS allow-list shape validation.
#[test]
fn cors_allowlist_rejects_non_origin_urls() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.server.cors_allowed_origins = vec!["https://farm.example/path".to_owned()];

    let error = config.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must contain only HTTP(S) origins without paths")
    );
}

/// PURPOSE: External base URLs must not embed userinfo credentials.
/// USE: prevents credential leakage via configured external URLs.
#[test]
fn configured_external_urls_reject_embedded_credentials() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    config.tests.gitlab_base_url = "https://user:secret@gitlab.example".to_owned();

    let error = config.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must not contain embedded credentials")
    );
}

/// PURPOSE: Rootfs extension defaults to `.tar.bz2` and rejects path-like values.
/// USE: guards device.rootfs_archive_extension validation.
#[test]
fn rootfs_archive_extension_defaults_and_rejects_paths() {
    let cli = Cli::try_parse_from(["farmcontroller"]).unwrap();
    let mut config = AppConfig::load(&cli).unwrap();
    assert_eq!(config.device.rootfs_archive_extension, ".tar.bz2");

    config.device.rootfs_archive_extension = "../rootfs".into();
    let error = config.validate().unwrap_err();
    assert!(error.to_string().contains("rootfs_archive_extension"));
}
