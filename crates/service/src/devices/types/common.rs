//! Shared device inventory projections and list/detail wrappers.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::ErrorResponse;

/// Request/response body type `DeviceRegistrationResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceRegistrationResponse {
    /// Whether the legacy API considers the operation successful.
    pub success: bool,
    /// Payload body preserved for frontend compatibility.
    pub data: serde_json::Value,
}

/// Request/response body type `DeviceDataResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceDataResponse {
    /// Whether the legacy API considers the operation successful.
    pub success: bool,
    /// Payload body preserved for frontend compatibility.
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

/// Request/response body type `DeviceHeartbeatResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceHeartbeatResponse {
    /// Whether the legacy API considers the operation successful.
    pub success: bool,
    /// Payload body preserved for frontend compatibility.
    #[schema(value_type = Option<Object>, nullable = true)]
    pub data: Option<serde_json::Value>,
}

/// Request/response body type `DeviceTopologyResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceTopologyResponse {
    /// Whether the legacy API considers the operation successful.
    pub success: bool,
    /// Payload body preserved for frontend compatibility.
    #[schema(value_type = Vec<Object>)]
    pub data: Vec<serde_json::Value>,
}

/// Enumerated wire value `CallbackStatus` for device HTTP handlers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CallbackStatus {
    Success,
    Failure,
}

/// Request/response body type `IplFlashingStatus` for device HTTP handlers.
#[derive(Debug, Deserialize, ToSchema)]
pub struct IplFlashingStatus {
    /// Outcome or lifecycle status for this record.
    pub status: CallbackStatus,
    /// Progress percent (0–100) for long-running operations.
    pub percentage: u8,
    /// Whether the operation finished successfully.
    pub completed: bool,
    /// Device or interface MAC / UID identifier.
    pub mac: String,
    /// Hardware generation (e.g. 4 or 5) for IPL/flash paths.
    pub generation: u8,
    /// Human-readable status or error text for the client.
    pub message: String,
}

/// EdgeController Gen5 mapping callback body for `POST /mapping-gen5`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MappingCallback {
    /// Device or interface MAC / UID identifier.
    #[schema(example = "aabbccddeeff")]
    pub mac: String,
    /// Outcome or lifecycle status for this record.
    pub status: CallbackStatus,
    /// Optional UART/power mapping payload from EdgeController.
    #[schema(example = json!({"uart": "/dev/ttyUSB0", "power": "1"}))]
    pub tty_entry: Option<serde_json::Value>,
    /// Caller or device IPv4 address.
    #[schema(example = "192.0.2.10")]
    pub ip: Option<String>,
}

/// Request/response body type `TtyEntry` for device HTTP handlers.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct TtyEntry {
    /// UART device path observed on the controller.
    pub uart: String,
    /// Power port / channel identifier paired with the UART mapping.
    pub power: String,
}

/// Request/response body type `FlashConfirmationQuery` for device HTTP handlers.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct FlashConfirmationQuery {
    /// Outcome or lifecycle status for this record.
    pub status: CallbackStatus,
}

/// Request/response body type `TestCompletionQuery` for device HTTP handlers.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestCompletionQuery {
    /// Test execution identifier.
    pub test_id: String,
}

/// Request/response body type `TestCompletionResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct TestCompletionResponse {
    /// Whether the legacy API considers the operation successful.
    #[schema(example = true)]
    pub success: bool,
    /// Human-readable status or error text for the client.
    #[schema(example = "Test completion handled for device aabbccddeeff")]
    pub message: String,
}

/// Request/response body type `DeviceFlashingResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceFlashingResponse {
    /// Whether the legacy API considers the operation successful.
    #[schema(example = true)]
    pub success: bool,
    /// Human-readable status or error text for the client.
    #[schema(example = "Device aabbccddeeff is now marked as upgrading")]
    pub message: String,
}

/// Request/response body type `EmptyObjectResponse` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EmptyObjectResponse {}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    /// Human-readable status or error text for the client.
    pub message: String,
}

/// Enumerated wire value `CompatibilityErrorResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum CompatibilityErrorResponse {
    Structured(ErrorResponse),
    Message(MessageResponse),
}

/// Request/response body type `SuccessResponse` for device HTTP handlers.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SuccessResponse {
    /// Whether the legacy API considers the operation successful.
    pub success: bool,
}

/// Request/response body type `CallbackResponse` for device HTTP handlers.
#[derive(Debug, Serialize, ToSchema)]
pub struct CallbackResponse {
    /// Whether the legacy API considers the operation successful.
    pub success: bool,
    /// Human-readable status or error text for the client.
    pub message: String,
}
