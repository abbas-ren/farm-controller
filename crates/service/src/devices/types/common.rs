use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::ErrorResponse;

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceRegistrationResponse {
    pub success: bool,
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceDataResponse {
    pub success: bool,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceHeartbeatResponse {
    pub success: bool,
    #[schema(value_type = Option<Object>, nullable = true)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceTopologyResponse {
    pub success: bool,
    #[schema(value_type = Vec<Object>)]
    pub data: Vec<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CallbackStatus {
    Success,
    Failure,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MappingCallback {
    #[schema(example = "aabbccddeeff")]
    pub mac: String,
    pub status: CallbackStatus,
    #[schema(example = json!({"uart": "/dev/ttyUSB0", "power": "1"}))]
    pub tty_entry: Option<serde_json::Value>,
    #[schema(example = "192.0.2.10")]
    pub ip: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct TtyEntry {
    pub uart: String,
    pub power: String,
}

#[derive(Debug, Deserialize)]
pub struct FlashConfirmationQuery {
    pub status: CallbackStatus,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestCompletionQuery {
    pub test_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestCompletionResponse {
    #[schema(example = true)]
    pub success: bool,
    #[schema(example = "Test completion handled for device aabbccddeeff")]
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceFlashingResponse {
    #[schema(example = true)]
    pub success: bool,
    #[schema(example = "Device aabbccddeeff is now marked as upgrading")]
    pub message: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EmptyObjectResponse {}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum CompatibilityErrorResponse {
    Structured(ErrorResponse),
    Message(MessageResponse),
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SuccessResponse {
    pub success: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CallbackResponse {
    pub success: bool,
    pub message: &'static str,
}
