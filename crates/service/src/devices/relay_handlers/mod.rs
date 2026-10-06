//! Relay endpoints grouped by identity, configuration, and retained legacy behavior.
//!
//! Handlers authenticate and validate requests, then coordinate persistence and
//! controller synchronization. Modern channel state is owned by
//! `relay_configuration_store`; legacy CRUD and conflict detection remain in
//! `legacy_relay_store` to preserve their distinct wire and data contracts.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{auth, error::ErrorResponse, events::ServerEvent, state::AppState};

use super::{
    CallbackStatus, LegacyRelayChannelListQuery, LegacyRelayConfiguration,
    LegacyRelayConfigurationResponse, LegacyRelayConflictQuery, LegacyRelayListQuery,
    LegacyRelayRemapRequest, MessageResponse, RelayChannelRecord, RelayConfigurationAccepted,
    RelayConfigurationConfirmation, RelayConfigurationRequest, RelayConfirmationResponse,
    RelayConflictResponse, RelayIdentityUpdateRequest, RelayIdentityUpdateResponse, RelayRecord,
    constants::{DEVICE_CALLBACK_TIMEOUT, EDGE_CONTROLLER_PORT, RELAY_IDENTITY_TIMEOUT},
    error_response,
    repository::DeviceRepository,
    repository_error,
    validation::relay_configurations,
};

mod configuration;
mod identity;
mod legacy;
mod shared;

#[cfg(test)]
pub(crate) use configuration::confirmation_events;
pub(crate) use configuration::{
    __path_configure_relay_channels, __path_confirm_relay_configuration, configure_relay_channels,
    confirm_relay_configuration,
};
pub(crate) use identity::{
    __path_relay_device_state, __path_update_relay_identity, relay_device_state,
    update_relay_identity,
};
pub(crate) use legacy::{
    __path_configure_legacy_relay, __path_delete_legacy_relay, __path_delete_legacy_relay_channel,
    __path_fresh_legacy_relay, __path_legacy_relay_by_id, __path_legacy_relay_channel_by_id,
    __path_legacy_relay_channels, __path_legacy_relay_conflicts, __path_legacy_relays,
    __path_remap_legacy_relay, configure_legacy_relay, delete_legacy_relay,
    delete_legacy_relay_channel, fresh_legacy_relay, legacy_relay_by_id,
    legacy_relay_channel_by_id, legacy_relay_channels, legacy_relay_conflicts, legacy_relays,
    remap_legacy_relay,
};

use shared::legacy_repository;
