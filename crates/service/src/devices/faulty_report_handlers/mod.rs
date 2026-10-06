//! Faulty-report HTTP handlers grouped by write, record, and download behavior.

use std::{
    path::{Path as FilePath, PathBuf},
    sync::Arc,
};

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{auth, error::ErrorResponse, events::ServerEvent, state::AppState};

use super::{
    CompatibilityErrorResponse, FaultyReportCreateRequest, FaultyReportDetail, FaultyReportRecord,
    FaultyReportStatusRequest, MessageResponse, SuccessResponse, error_response,
    repository::DeviceRepository, repository_error, repository_types::FaultyReportCreate,
};

mod create_upload;
mod downloads;
mod records;
mod shared;

use shared::{context, remove_pending, report_id, safe_file_name, safe_segment};

pub(crate) use create_upload::{__path_create, create};
pub(crate) use downloads::{
    __path_download_file, __path_download_logs, download_file, download_logs,
};
pub(crate) use records::{
    __path_by_id, __path_delete, __path_list, __path_update_status, by_id, delete, list,
    update_status,
};
