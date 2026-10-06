use super::*;
use crate::devices::repository::UserDeviceRepository;

#[async_trait]
impl DeviceRegistrationRepository for RegistrationRepository {
    async fn test_completion_target(
        &self,
        test_id: &str,
        _device_id: &str,
    ) -> Result<Option<repository_types::TestCompletionTarget>, DeviceRepositoryError> {
        Ok(
            (test_id == "test-1").then(|| repository_types::TestCompletionTarget {
                status: "in_progress".to_owned(),
                device_family: Some("Gen3".to_owned()),
                mac_address: Some("00:11:22:33:44:55".to_owned()),
                device_type: "x3h".to_owned(),
                build_version: Some("v1.2.3".to_owned()),
                build_id: Some("build-1".to_owned()),
                created_by: Some("user-1".to_owned()),
                gen5_controller_ip: None,
                uart_port: None,
                gen4_controller_ip: None,
                relay_serial: None,
                relay_channel: None,
            }),
        )
    }

    async fn store_rtos_log_path(
        &self,
        _test_id: &str,
        _path: &str,
    ) -> Result<(), DeviceRepositoryError> {
        Ok(())
    }

    async fn mark_device_flashing(
        &self,
        device_id: &str,
    ) -> Result<Option<repository_types::DeviceFlashingResult>, DeviceRepositoryError> {
        Ok(
            (device_id == "device-1").then(|| repository_types::DeviceFlashingResult {
                device: serde_json::json!({
                    "deviceId": device_id, "state": "busy",
                    "upgrading": true, "flashing": false
                }),
                rtos_target: None,
            }),
        )
    }

    async fn export_devices(
        &self,
        _query: &DeviceCsvQuery,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "deviceId": "device-1",
            "deviceName": "Bench 1",
            "deviceType": "x5h",
            "macAddress": "00:11:22:33:44:55",
            "ipAddress": "192.0.2.10",
            "status": "approved",
            "state": "free",
            "createdAt": "2026-10-02T12:00:00Z",
            "updatedAt": "2026-10-03T12:00:00Z",
            "lastConnectedOn": "2026-10-03T12:00:00Z",
            "interfaces": [{"type": "ethernet", "interfaceId": "eth0"}]
        })])
    }

    async fn register_device(
        &self,
        registration: &DeviceRegistration,
    ) -> Result<repository_types::DeviceRegistrationResult, DeviceRepositoryError> {
        let device_id = validation::validate_device_registration(registration)?;
        Ok(repository_types::DeviceRegistrationResult {
            device: serde_json::json!({
                "deviceId": device_id,
                "macAddress": registration.mac_address,
                "ipAddress": registration.ip_address,
                "deviceName": registration.device_name,
                "state": registration.state,
                "status": registration.status,
                "heartbeatTimer": registration.timeout,
                "interfaces": validation::device_interfaces(registration)
                    .into_iter()
                    .map(|interface| serde_json::json!({
                        "deviceId": device_id,
                        "type": interface.interface_type,
                        "interfaceId": interface.interface_id
                    }))
                    .collect::<Vec<_>>()
            }),
            notify_addition: true,
            notify_approval: true,
        })
    }

    async fn register_controller(
        &self,
        registration: &ControllerRegistration,
    ) -> Result<RegistrationResult, DeviceRepositoryError> {
        let created = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let now = chrono::Utc::now();
        Ok(RegistrationResult {
            created,
            controller: DeviceController {
                device_controller_id: validation::normalized_controller_id(
                    &registration.mac_address,
                )?,
                mac_address: registration.mac_address.clone(),
                ip_address: registration.ip_address.clone(),
                name: None,
                device_family: registration.device_family.clone(),
                status: "approved".to_owned(),
                state: "active".to_owned(),
                created_at: now,
                updated_at: now,
                relays: Vec::new(),
            },
        })
    }

    async fn save_gen5_mapping(
        &self,
        caller_ip: &str,
        mac: &str,
        status: CallbackStatus,
        tty_entry: Option<&TtyEntry>,
    ) -> Result<(), DeviceRepositoryError> {
        self.callbacks.lock().unwrap().push(format!(
            "mapping:{caller_ip}:{mac}:{status:?}:{}",
            tty_entry.map_or("none", |entry| entry.uart.as_str())
        ));
        Ok(())
    }

    async fn confirm_flash(
        &self,
        device_type: &str,
        status: CallbackStatus,
    ) -> Result<FlashConfirmationResult, DeviceRepositoryError> {
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("flash:{device_type}:{status:?}"));
        Ok(if status == CallbackStatus::Failure {
            FlashConfirmationResult {
                cancelled_execution: Some(repository_types::FlashCancelledExecution {
                    test_id: "test-1".to_owned(),
                    build_id: "build-1".to_owned(),
                    created_by: "user-1".to_owned(),
                }),
                freed_device_id: Some("device-1".to_owned()),
            }
        } else {
            FlashConfirmationResult::default()
        })
    }
}

#[async_trait]
impl DeviceInventoryRepository for RegistrationRepository {
    async fn device_families(&self) -> Result<Vec<String>, DeviceRepositoryError> {
        Ok(vec!["Gen4".to_owned(), "Gen5".to_owned()])
    }

    async fn device_types(
        &self,
        device_family: Option<&str>,
    ) -> Result<Vec<String>, DeviceRepositoryError> {
        Ok(match device_family {
            Some("Gen5") => vec!["x5h".to_owned()],
            _ => vec!["v4h".to_owned(), "x5h".to_owned()],
        })
    }

    async fn device_by_id(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((device_id == "device-1").then(|| {
            serde_json::json!({
                "deviceId": "device-1",
                "deviceFamily": "Gen5",
                "interfaces": [],
                "testExecutions": []
            })
        }))
    }

    async fn latest_heartbeat(
        &self,
        device_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok((device_id == "device-1").then(|| {
            serde_json::json!({
                "timestamp": "2026-10-02T00:00:00Z",
                "data": {"cpuUsagePercent": 12.5},
                "timeout": 30
            })
        }))
    }

    async fn topology(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(vec![serde_json::json!({
            "deviceId": "device-1",
            "deviceName": "Bench 1",
            "deviceType": "x5h",
            "deviceFamily": "Gen5",
            "macAddress": "001122334455",
            "ipAddress": "192.0.2.20",
            "state": "free",
            "status": "approved",
            "controllerId": "aabbccddeeff",
            "heartbeatTimer": 5
        })])
    }

    async fn list_devices(
        &self,
        query: &DeviceListQuery,
        is_admin: bool,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        Ok(DeviceList {
            data: vec![
                serde_json::json!({"deviceId": "device-1", "status": if is_admin { "requested" } else { "approved" }}),
            ],
            total_pages: 1,
            current_page: query.page.unwrap_or(1),
            total_devices: 1,
            requested_count: u64::from(is_admin),
            device_timers: BTreeMap::from([("device-1".to_owned(), 30)]),
            device_timeouts: BTreeMap::from([("device-1".to_owned(), 5)]),
            state_count: BTreeMap::new(),
        })
    }
}

#[async_trait]
impl ControllerRepository for RegistrationRepository {
    async fn device_action_target(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceActionTarget>, DeviceRepositoryError> {
        Ok(
            matches!(device_id, "device-1" | "heartbeat-device").then(|| DeviceActionTarget {
                device_id: device_id.to_owned(),
                ip_address: if device_id == "heartbeat-device" {
                    "127.0.0.1".to_owned()
                } else {
                    "192.0.2.20".to_owned()
                },
                status: "requested".to_owned(),
            }),
        )
    }

    async fn apply_device_action(
        &self,
        device_id: &str,
        action: DeviceAction,
        _user_id: &str,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        let status = match action {
            DeviceAction::Approved => "approved",
            DeviceAction::Declined => "declined",
        };
        Ok((device_id == "device-1")
            .then(|| serde_json::json!({"deviceId": device_id, "status": status})))
    }

    async fn list_controllers(
        &self,
        query: &ControllerListQuery,
    ) -> Result<ControllerList, DeviceRepositoryError> {
        Ok(ControllerList {
            data: Vec::new(),
            total_pages: 0,
            current_page: query.page.unwrap_or(1),
            total_count: 0,
            summary: serde_json::json!({"controllers": {"total": 0, "active": 0, "notReachable": 0}, "relays": {"total": 0, "connected": 0, "disconnected": 0}}),
        })
    }

    async fn edit_controller(
        &self,
        _controller_id: &str,
        _request: &ControllerEditRequest,
    ) -> Result<Option<serde_json::Value>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn controller_delete_target(
        &self,
        _controller_id: &str,
    ) -> Result<Option<ControllerDeleteTarget>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn delete_controller(&self, _controller_id: &str) -> Result<bool, DeviceRepositoryError> {
        Ok(false)
    }

    async fn device_delete_target(
        &self,
        _device_id: &str,
    ) -> Result<Option<DeviceDeleteTarget>, DeviceRepositoryError> {
        Ok(None)
    }

    async fn delete_device(&self, _device_id: &str) -> Result<bool, DeviceRepositoryError> {
        Ok(false)
    }
}

#[async_trait]
impl UserDeviceRepository for RegistrationRepository {
    async fn list_user_devices(
        &self,
        query: &UserDeviceListQuery,
    ) -> Result<DeviceList, DeviceRepositoryError> {
        Ok(DeviceList {
            data: Vec::new(),
            total_pages: 0,
            current_page: query.page.unwrap_or(1),
            total_devices: 0,
            requested_count: 0,
            device_timers: BTreeMap::new(),
            device_timeouts: BTreeMap::new(),
            state_count: BTreeMap::new(),
        })
    }

    async fn active_devices(&self) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(Vec::new())
    }

    async fn update_heartbeat_timeout(
        &self,
        device_id: &str,
        value: i64,
    ) -> Result<bool, DeviceRepositoryError> {
        self.callbacks
            .lock()
            .unwrap()
            .push(format!("heartbeat:{device_id}:{value}"));
        Ok(true)
    }
}

#[async_trait]
impl DeviceArtifactRepository for RegistrationRepository {
    async fn builds_for_device_type(
        &self,
        _device_type: &str,
    ) -> Result<Vec<serde_json::Value>, DeviceRepositoryError> {
        Ok(Vec::new())
    }

    async fn configure_artifacts(
        &self,
        entries: &[DeviceTypeFolder],
    ) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        Ok(entries.to_vec())
    }

    async fn artifact_folders(&self) -> Result<Vec<DeviceTypeFolder>, DeviceRepositoryError> {
        Ok(Vec::new())
    }

    async fn artifact_folder_for_device(
        &self,
        _device_id: &str,
    ) -> Result<Option<String>, DeviceRepositoryError> {
        Ok(None)
    }
}
