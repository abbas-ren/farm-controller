use clap::Parser;

use super::*;

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

#[test]
fn domain_module_requires_api() {
    let cli =
        Cli::try_parse_from(["farmcontroller", "--enable", "device", "--disable", "api"]).unwrap();

    let error = AppConfig::load(&cli).unwrap_err();
    assert!(error.to_string().contains("requires the api module"));
}

#[test]
fn persistence_module_requires_database_url() {
    let cli = Cli::try_parse_from(["farmcontroller", "--enable", "api,device"]).unwrap();

    let error = AppConfig::load(&cli).unwrap_err();
    assert!(error.to_string().contains("database.url is required"));
}

#[test]
fn events_module_does_not_require_persistence() {
    let cli = Cli::try_parse_from(["farmcontroller", "--enable", "events"]).unwrap();

    let config = AppConfig::load(&cli).unwrap();
    assert!(config.module_enabled(Module::Events));
    assert!(!config.database_required());
}

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
