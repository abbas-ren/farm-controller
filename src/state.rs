use std::{sync::Arc, time::Instant};

use crate::{
    auth::{IdentityProvider, KeycloakIdentityProvider},
    config::{AppConfig, Module},
    devices::{DeviceRepository, PostgresDeviceRepository},
    error::AppError,
    events::EventHub,
    jira::JiraClient,
    observability::Metrics,
    persistence::Database,
    qmetry_catalog::{HttpQmetryCatalog, QmetryCatalog},
    reports::{PostgresReportDataSource, ReportService},
    test_catalog::{TestCatalog, TestRailCatalog},
    workers::RelaySyncService,
};

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub metrics: Metrics,
    pub identity_provider: Option<Arc<dyn IdentityProvider>>,
    pub database: Option<Database>,
    pub(crate) device_repository: Option<Arc<dyn DeviceRepository>>,
    pub(crate) event_publisher: Arc<EventHub>,
    pub(crate) relay_sync: RelaySyncService,
    pub reports: Option<ReportService>,
    pub test_catalog: Option<Arc<dyn TestCatalog>>,
    pub qmetry_catalog: Option<Arc<dyn QmetryCatalog>>,
    pub jira: Option<Arc<JiraClient>>,
    pub started_at: Instant,
}

impl AppState {
    pub async fn bootstrap(config: AppConfig, metrics: Metrics) -> Result<Self, AppError> {
        let database = if config.database_required() {
            Some(Database::connect(&config.database, metrics.clone()).await?)
        } else {
            None
        };
        let device_repository = database.as_ref().and_then(|database| {
            config.module_enabled(Module::Device).then(|| {
                Arc::new(PostgresDeviceRepository::new(database.pool().clone()))
                    as Arc<dyn DeviceRepository>
            })
        });
        let event_publisher = Arc::new(EventHub::default());
        let relay_sync = RelaySyncService::new(config.workers.relay_sync_queue_capacity);
        let reports = database.as_ref().and_then(|database| {
            config.module_enabled(Module::Reports).then(|| {
                ReportService::new(
                    config.reports.clone(),
                    Arc::new(PostgresReportDataSource::new(database.pool().clone())),
                    metrics.clone(),
                )
                .with_event_publisher(event_publisher.clone())
            })
        });
        let identity_provider = if config.module_enabled(Module::Auth) {
            Some(
                Arc::new(KeycloakIdentityProvider::new(config.auth.clone())?)
                    as Arc<dyn IdentityProvider>,
            )
        } else {
            None
        };
        let test_catalog = TestRailCatalog::configured(config.tests.clone())
            .map_err(|error| AppError::Configuration(error.to_string()))?;
        let qmetry_catalog = HttpQmetryCatalog::configured(config.tests.clone())
            .map_err(|error| AppError::Configuration(error.to_string()))?;
        let jira = JiraClient::configured(&config.tests)
            .map_err(|error| AppError::Configuration(error.to_string()))?
            .map(Arc::new);
        Ok(Self {
            config,
            metrics,
            identity_provider,
            database,
            device_repository,
            event_publisher,
            relay_sync,
            reports,
            test_catalog,
            qmetry_catalog,
            jira,
            started_at: Instant::now(),
        })
    }

    #[cfg(test)]
    pub fn without_dependencies(config: AppConfig, metrics: Metrics) -> Self {
        let relay_sync = RelaySyncService::new(config.workers.relay_sync_queue_capacity);
        Self {
            config,
            metrics,
            identity_provider: None,
            database: None,
            device_repository: None,
            event_publisher: Arc::new(EventHub::default()),
            relay_sync,
            reports: None,
            test_catalog: None,
            qmetry_catalog: None,
            jira: None,
            started_at: Instant::now(),
        }
    }

    #[cfg(test)]
    pub fn with_identity_provider(
        config: AppConfig,
        metrics: Metrics,
        identity_provider: Arc<dyn IdentityProvider>,
    ) -> Self {
        let relay_sync = RelaySyncService::new(config.workers.relay_sync_queue_capacity);
        Self {
            config,
            metrics,
            identity_provider: Some(identity_provider),
            database: None,
            device_repository: None,
            event_publisher: Arc::new(EventHub::default()),
            relay_sync,
            reports: None,
            test_catalog: None,
            qmetry_catalog: None,
            jira: None,
            started_at: Instant::now(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_device_repository(
        mut self,
        device_repository: Arc<dyn DeviceRepository>,
    ) -> Self {
        self.config.modules.enabled.insert(Module::Device);
        self.device_repository = Some(device_repository);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_catalog(mut self, test_catalog: Arc<dyn TestCatalog>) -> Self {
        self.test_catalog = Some(test_catalog);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_qmetry_catalog(mut self, qmetry_catalog: Arc<dyn QmetryCatalog>) -> Self {
        self.qmetry_catalog = Some(qmetry_catalog);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_reports(mut self, reports: ReportService) -> Self {
        self.reports = Some(reports.with_event_publisher(self.event_publisher.clone()));
        self
    }
}
