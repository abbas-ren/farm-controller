//! In-memory TestRail/Qmetry/report/identity provider fakes for device route tests.
//!
//! Stands in for production catalog and auth providers so handlers avoid upstream HTTP.

use super::*;

#[async_trait]
impl ReportDataSource for FakeReportDataSource {
    /// PURPOSE: In-memory stand-in for `ReportDataSource::test_cases`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn test_cases(&self, _test_id: &str) -> Result<Vec<TestCaseRecord>, ReportError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl QmetryCatalog for FakeQmetryCatalog {
    /// PURPOSE: In-memory stand-in for `QmetryCatalog::cases_by_plan`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn cases_by_plan(&self, _plan_id: u64) -> Result<serde_json::Value, QmetryError> {
        Ok(
            serde_json::json!({"8": [{"id": "case-1", "name": "Boot", "script_file": "run.sh", "labels": ["smoke"]}]}),
        )
    }

    /// PURPOSE: In-memory stand-in for `QmetryCatalog::cases_by_folder`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn cases_by_folder(&self, _folder_id: u64) -> Result<serde_json::Value, QmetryError> {
        Ok(serde_json::json!([{"id": "case-1", "name": "Boot", "script_file": "run.sh"}]))
    }
}

#[async_trait]
impl TestCatalog for FakeTestCatalog {
    /// PURPOSE: In-memory stand-in for `TestCatalog::plans`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn plans(&self, filter: Option<&str>) -> Result<Vec<TestPlan>, TestCatalogError> {
        Ok(vec![TestPlan {
            id: 7,
            name: filter.unwrap_or("Plan").to_owned(),
            description: Some("Device smoke plan".to_owned()),
            test_suits: Vec::new(),
        }])
    }

    /// PURPOSE: In-memory stand-in for `TestCatalog::suites`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn suites(&self, plan_id: u64) -> Result<Vec<TestSuite>, TestCatalogError> {
        Ok(vec![TestSuite {
            id: 8,
            name: "Core".to_owned(),
            plan_id,
            order: 1,
            description: None,
        }])
    }

    /// PURPOSE: In-memory stand-in for `TestCatalog::cases`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn cases(
        &self,
        plan_id: u64,
        suite_id: u64,
        _filter: Option<&str>,
    ) -> Result<Vec<TestCase>, TestCatalogError> {
        Ok(vec![TestCase {
            id: 9,
            title: "Boot".to_owned(),
            plan_id,
            suite_id,
            order: 1,
            priority_id: 2,
            script_file: "run.sh".to_owned(),
            pre_condition: Some("Ready".to_owned()),
            labels: vec!["smoke".to_owned()],
        }])
    }

    /// PURPOSE: In-memory stand-in for `TestCatalog::update_result`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn update_result(
        &self,
        _update: &crate::test_catalog::TestResultUpdate,
    ) -> Result<(), TestCatalogError> {
        Ok(())
    }
}

#[async_trait]
impl IdentityProvider for AcceptingIdentityProvider {
    /// PURPOSE: In-memory stand-in for `IdentityProvider::signin`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn signin(
        &self,
        _username: &str,
        _password: &str,
    ) -> Result<SigninResult, IdentityError> {
        Ok(SigninResult {
            access_token: "access".to_owned(),
            refresh_token: "refresh".to_owned(),
            user: AuthenticatedUser {
                id: "user-1".to_owned(),
                username: "farm.user".to_owned(),
                email: "farm.user@example.com".to_owned(),
                first_name: None,
                last_name: None,
                realm_roles: vec!["user".to_owned()],
            },
        })
    }

    /// PURPOSE: In-memory stand-in for `IdentityProvider::validate`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn validate(
        &self,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _required_role: Option<&str>,
    ) -> Result<ValidationResult, IdentityError> {
        Ok(ValidationResult {
            refreshed_tokens: None,
        })
    }

    /// PURPOSE: In-memory stand-in for `IdentityProvider::request_registration`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn request_registration(
        &self,
        request: &RegistrationRequest,
    ) -> Result<serde_json::Value, IdentityError> {
        Ok(serde_json::json!({
            "id": "pending-user-1",
            "username": request.username,
            "firstName": request.first_name,
            "lastName": request.last_name,
            "email": request.email,
            "emailVerified": false,
            "createdTimestamp": 1790899200000_i64,
            "enabled": true,
            "totp": false,
            "disableableCredentialTypes": [],
            "requiredActions": ["UPDATE_PASSWORD"],
            "notBefore": 0,
            "access": {},
            "realmRoles": []
        }))
    }

    /// PURPOSE: In-memory stand-in for `IdentityProvider::user_by_id`.
    /// USE: device route tests that need catalog/report/identity doubles without external services.
    async fn user_by_id(&self, user_id: &str) -> Result<ManagedUser, IdentityError> {
        Ok(ManagedUser {
            id: user_id.to_owned(),
            username: "farm.user".to_owned(),
            email: Some("farm.user@example.com".to_owned()),
            first_name: Some("Farm".to_owned()),
            last_name: Some("User".to_owned()),
            realm_roles: vec!["user".to_owned()],
        })
    }
}
