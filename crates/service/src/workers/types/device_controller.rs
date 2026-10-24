//! DTOs for device-controller heartbeat timeout transitions.

/// Result of marking a controller unreachable and raising an alert.
pub struct ControllerStateTransition {
    pub controller_id: String,
    /// Alert payload shape expected by the frontend alert stream.
    pub alert: serde_json::Value,
}
