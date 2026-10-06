//! Confluence configuration, page operations, and report attachment upload.

use std::path::Path;

use regex::Regex;
use reqwest::{Url, multipart};
use serde::Deserialize;

use crate::config::ReportsConfig;

use super::{
    ConfluenceError, ConfluenceUploadRequest, ConfluenceUploadResponse, ReportRequest,
    ReportService, append_log, constants::confluence as messages, escape_html, report_root,
};

impl ReportService {
    pub async fn upload_confluence(
        &self,
        request: ConfluenceUploadRequest,
    ) -> Result<ConfluenceUploadResponse, ConfluenceError> {
        let report_request = ReportRequest {
            test_id: request.test_id.clone(),
            build_version: request.build_version.clone(),
            device_type: request.device_type.clone(),
            previous_test_id: None,
            previous_build_version: None,
            created_by: None,
        };
        let root = report_root(&self.config.test_results_dir, &report_request)
            .map_err(|error| ConfluenceError::Validation(error.to_string()))?;
        let report_path = root.join("report.html");
        if !report_path.is_file() {
            return Err(ConfluenceError::Validation(
                messages::REPORT_HTML_MISSING.to_owned(),
            ));
        }
        let settings = ConfluenceSettings::from_config(&self.config, &request.build_version)?;
        let mut report_html = tokio::fs::read_to_string(&report_path).await?;
        report_html = confluence_body(&report_html)?;
        append_log(&root, "INFO", messages::UPLOAD_STARTED).await?;
        tracing::info!(operation = "confluence_upload", "report upload started");

        let parent = self.lookup_page(&settings, &settings.parent_title).await?;
        let parent_page_id = match parent {
            Some(page) => page.id,
            None => {
                self.create_page(
                    &settings,
                    &settings.parent_title,
                    &settings.top_level_parent_id,
                    &format!("<h1>{}</h1>", escape_html(&settings.parent_title)),
                )
                .await?
            }
        };
        let subpage_title = format!(
            "{} - {} - {} - {}",
            request.build_version, request.device_type, request.test_id, settings.subpage_title
        );
        let page_id = match self.lookup_page(&settings, &subpage_title).await? {
            Some(page) => {
                self.update_page(
                    &settings,
                    &page.id,
                    &subpage_title,
                    &report_html,
                    page.version.number,
                )
                .await?
            }
            None => {
                self.create_page(&settings, &subpage_title, &parent_page_id, &report_html)
                    .await?
            }
        };

        let (uploaded_images, failed_images) =
            self.upload_images(&settings, &page_id, &root).await?;
        append_log(
            &root,
            "INFO",
            &format!("{}: page {page_id}", messages::UPLOAD_COMPLETED),
        )
        .await?;
        tracing::info!(
            operation = "confluence_upload",
            uploaded_images = uploaded_images.len(),
            failed_images = failed_images.len(),
            "report upload completed"
        );
        Ok(ConfluenceUploadResponse {
            status: "uploaded",
            page_id,
            subpage_title,
            parent_page_id,
            uploaded_images,
            failed_images,
        })
    }

    async fn lookup_page(
        &self,
        settings: &ConfluenceSettings,
        title: &str,
    ) -> Result<Option<ConfluencePage>, ConfluenceError> {
        let response = self
            .client
            .get(settings.endpoint("rest/api/content")?)
            .bearer_auth(&settings.pat)
            .query(&[
                ("spaceKey", settings.space.as_str()),
                ("title", title),
                ("expand", "version,ancestors"),
            ])
            .send()
            .await
            .map_err(upstream_transport)?;
        let response = upstream_response(response).await?;
        let body: ConfluenceSearch = response.json().await.map_err(upstream_transport)?;
        Ok(body.results.into_iter().next())
    }

    async fn create_page(
        &self,
        settings: &ConfluenceSettings,
        title: &str,
        parent_id: &str,
        body: &str,
    ) -> Result<String, ConfluenceError> {
        let response = self
            .client
            .post(settings.endpoint("rest/api/content")?)
            .bearer_auth(&settings.pat)
            .json(&serde_json::json!({
                "type": "page", "title": title, "ancestors": [{"id": parent_id}],
                "space": {"key": settings.space},
                "body": {"storage": {"value": body, "representation": "storage"}}
            }))
            .send()
            .await
            .map_err(upstream_transport)?;
        let response = upstream_response(response).await?;
        let page: ConfluencePageId = response.json().await.map_err(upstream_transport)?;
        Ok(page.id)
    }

    async fn update_page(
        &self,
        settings: &ConfluenceSettings,
        page_id: &str,
        title: &str,
        body: &str,
        current_version: u64,
    ) -> Result<String, ConfluenceError> {
        let response = self
            .client
            .put(settings.endpoint(&format!("rest/api/content/{page_id}"))?)
            .bearer_auth(&settings.pat)
            .json(&serde_json::json!({
                "type": "page", "title": title, "version": {"number": current_version + 1},
                "body": {"storage": {"value": body, "representation": "storage"}}
            }))
            .send()
            .await
            .map_err(upstream_transport)?;
        let response = upstream_response(response).await?;
        let page: ConfluencePageId = response.json().await.map_err(upstream_transport)?;
        Ok(page.id)
    }

    async fn upload_images(
        &self,
        settings: &ConfluenceSettings,
        page_id: &str,
        root: &Path,
    ) -> Result<(Vec<String>, Vec<String>), ConfluenceError> {
        let directory = root.join("perf_compare");
        let mut names = vec!["radar_chart.png".to_owned()];
        names.extend(
            ["network", "storage", "cpu", "memory", "dma", "gpu"]
                .map(|category| format!("{category}_bar.png")),
        );
        if let Ok(mut entries) = tokio::fs::read_dir(&directory).await {
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("test_") && name.ends_with("_comp.png") {
                    names.push(name);
                }
            }
        }
        names.sort();
        names.dedup();
        let mut uploaded = Vec::new();
        let mut failed = Vec::new();
        for name in names {
            let path = directory.join(&name);
            if !path.is_file() {
                failed.push(name);
                continue;
            }
            let bytes = tokio::fs::read(&path).await?;
            let form = multipart::Form::new().part(
                "file",
                multipart::Part::bytes(bytes)
                    .file_name(name.clone())
                    .mime_str("image/png")
                    .map_err(|error| ConfluenceError::Upstream(error.to_string()))?,
            );
            let request = self
                .client
                .post(settings.endpoint(&format!("rest/api/content/{page_id}/child/attachment"))?)
                .bearer_auth(&settings.pat)
                .header("X-Atlassian-Token", "no-check")
                .multipart(form);
            match request.send().await {
                Ok(response) if response.status().is_success() => uploaded.push(name),
                Ok(response) => failed.push(format!("{} (HTTP {})", name, response.status())),
                Err(error) => failed.push(format!("{name} ({error})")),
            }
        }
        Ok((uploaded, failed))
    }
}

#[derive(Debug)]
pub(crate) struct ConfluenceSettings {
    api_base: Url,
    pat: String,
    space: String,
    parent_title: String,
    subpage_title: String,
    top_level_parent_id: String,
}

impl ConfluenceSettings {
    pub(crate) fn from_config(
        config: &ReportsConfig,
        build_version: &str,
    ) -> Result<Self, ConfluenceError> {
        let required = [
            ("confluence_url", config.confluence_url.trim()),
            ("confluence_pat", config.confluence_pat.trim()),
            ("confluence_space", config.confluence_space.trim()),
            (
                "confluence_subpage_title",
                config.confluence_subpage_title.trim(),
            ),
        ];
        let missing: Vec<_> = required
            .iter()
            .filter_map(|(name, value)| value.is_empty().then_some(*name))
            .collect();
        if !missing.is_empty() {
            return Err(ConfluenceError::Configuration(format!(
                "Missing Confluence configuration: {}",
                missing.join(", ")
            )));
        }
        let source = Url::parse(config.confluence_url.trim()).map_err(|error| {
            ConfluenceError::Configuration(format!("Invalid Confluence URL: {error}"))
        })?;
        let segments: Vec<_> = source.path_segments().into_iter().flatten().collect();
        let top_level_parent_id = segments
            .windows(2)
            .find_map(|parts| {
                (parts[0] == "pages"
                    && parts[1].chars().all(|character| character.is_ascii_digit()))
                .then(|| parts[1].to_owned())
            })
            .ok_or_else(|| {
                ConfluenceError::Validation(messages::PARENT_PAGE_ID_MISSING.to_owned())
            })?;
        let mut api_base = source;
        api_base.set_path("/");
        api_base.set_query(None);
        api_base.set_fragment(None);
        Ok(Self {
            api_base,
            pat: config.confluence_pat.clone(),
            space: config.confluence_space.clone(),
            parent_title: if config.confluence_parent_title.trim().is_empty() {
                build_version.to_owned()
            } else {
                config.confluence_parent_title.clone()
            },
            subpage_title: config.confluence_subpage_title.clone(),
            top_level_parent_id,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, ConfluenceError> {
        self.api_base
            .join(path)
            .map_err(|error| ConfluenceError::Configuration(error.to_string()))
    }
}

#[derive(Deserialize)]
struct ConfluenceSearch {
    results: Vec<ConfluencePage>,
}

#[derive(Deserialize)]
struct ConfluencePage {
    id: String,
    version: ConfluenceVersion,
}

#[derive(Deserialize)]
struct ConfluenceVersion {
    number: u64,
}

#[derive(Deserialize)]
struct ConfluencePageId {
    id: String,
}

fn upstream_transport(error: reqwest::Error) -> ConfluenceError {
    ConfluenceError::Upstream(error.to_string())
}

async fn upstream_response(
    response: reqwest::Response,
) -> Result<reqwest::Response, ConfluenceError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(ConfluenceError::Upstream(format!(
        "Confluence returned HTTP {status}: {}",
        body.chars().take(1000).collect::<String>()
    )))
}

pub(crate) fn confluence_body(document: &str) -> Result<String, ConfluenceError> {
    let body = Regex::new(r"(?is)<body[^>]*>(.*)</body>")
        .expect("static regex is valid")
        .captures(document)
        .and_then(|captures| captures.get(1))
        .map_or(document, |body| body.as_str());
    let images =
        Regex::new(r#"(?i)<img\b[^>]*src="([^"]*\.png)"[^>]*>"#).expect("static regex is valid");
    Ok(images
        .replace_all(body, |captures: &regex::Captures<'_>| {
            let filename = Path::new(&captures[1])
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&captures[1]);
            format!(
                r#"<ac:image ac:width="600"><ri:attachment ri:filename="{}"/></ac:image>"#,
                escape_html(filename)
            )
        })
        .into_owned())
}
