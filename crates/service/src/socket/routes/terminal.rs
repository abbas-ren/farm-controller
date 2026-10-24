//! Interactive device terminal WebSocket (PTY proxy to edge).

use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::extract::ws::{Message, WebSocket};
use russh::{ChannelMsg, Disconnect, client, keys::PublicKeyOrCertificate};

use crate::state::AppState;

struct SshClient {
    host: String,
    port: u16,
    known_hosts_file: PathBuf,
    accept_unknown_host_keys: bool,
}

impl client::Handler for SshClient {
    type Error = russh::Error;

    /// Check server key.
    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "check_server_key");
        if self.accept_unknown_host_keys {
            return Ok(true);
        }
        match russh::keys::check_known_hosts_path(
            &self.host,
            self.port,
            &server_public_key.public_key(),
            &self.known_hosts_file,
        ) {
            Ok(known) => Ok(known),
            Err(error) => {
                tracing::warn!(
                    host = self.host,
                    port = self.port,
                    %error,
                    "SSH server host key verification failed"
                );
                Ok(false)
            }
        }
    }
}

struct TerminalTarget {
    host: String,
    port: u16,
    username: String,
    password: String,
    startup_command: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalAuthMethod {
    None,
    Password,
}

/// Terminal auth method.
fn terminal_auth_method(password: &str) -> TerminalAuthMethod {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "terminal_auth_method");
    if password.is_empty() {
        TerminalAuthMethod::None
    } else {
        TerminalAuthMethod::Password
    }
}

/// Session.
pub(super) async fn session(state: Arc<AppState>, mut socket: WebSocket, allow_farm_host: bool) {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "session");
    let Some(target) = receive_target(&state, &mut socket, allow_farm_host).await else {
        return;
    };
    let timeout = Duration::from_secs(state.config.device.terminal_connect_timeout_seconds);
    let config = Arc::new(client::Config::default());
    let ssh_client = SshClient {
        host: target.host.clone(),
        port: target.port,
        known_hosts_file: state.config.device.terminal_known_hosts_file.clone().into(),
        accept_unknown_host_keys: state.config.device.terminal_accept_unknown_host_keys,
    };
    tracing::debug!(host = %target.host, port = target.port, username = %target.username, timeout_seconds = timeout.as_secs(), "opening SSH terminal connection");
    let connection = tokio::time::timeout(
        timeout,
        client::connect(config, (target.host.as_str(), target.port), ssh_client),
    )
    .await;
    let mut ssh = match connection {
        Ok(Ok(ssh)) => ssh,
        Ok(Err(error)) => {
            tracing::error!(host = %target.host, port = target.port, %error, "SSH connection failed");
            send_error(&mut socket, format!("SSH error: {error}")).await;
            return;
        }
        Err(_) => {
            tracing::warn!(host = %target.host, port = target.port, timeout_seconds = timeout.as_secs(), "SSH connection timed out");
            send_error(&mut socket, "SSH error: connection timed out").await;
            return;
        }
    };
    let auth_method = terminal_auth_method(&target.password);
    let authentication = match auth_method {
        TerminalAuthMethod::None => ssh.authenticate_none(&target.username).await,
        TerminalAuthMethod::Password => {
            ssh.authenticate_password(&target.username, &target.password)
                .await
        }
    };
    let authenticated = match authentication {
        Ok(result) => result.success(),
        Err(error) => {
            tracing::error!(host = %target.host, username = %target.username, ?auth_method, %error, "SSH authentication request failed");
            send_error(&mut socket, format!("SSH error: {error}")).await;
            return;
        }
    };
    if !authenticated {
        tracing::warn!(host = %target.host, username = %target.username, ?auth_method, "SSH authentication was rejected");
        send_error(&mut socket, "SSH error: authentication failed").await;
        return;
    }
    tracing::info!(host = %target.host, username = %target.username, ?auth_method, "SSH terminal authenticated");
    let mut channel = match ssh.channel_open_session().await {
        Ok(channel) => channel,
        Err(error) => {
            tracing::error!(host = %target.host, %error, "SSH session channel failed to open");
            send_error(&mut socket, format!("Shell error: {error}")).await;
            return;
        }
    };
    if let Err(error) = channel
        .request_pty(false, "xterm", 120, 40, 0, 0, &[])
        .await
    {
        tracing::error!(host = %target.host, %error, "SSH pseudo-terminal request failed");
        send_error(&mut socket, format!("Shell error: {error}")).await;
        return;
    }
    if let Err(error) = channel.request_shell(true).await {
        tracing::error!(host = %target.host, %error, "SSH interactive shell request failed");
        send_error(&mut socket, format!("Shell error: {error}")).await;
        return;
    }
    tracing::info!(host = %target.host, username = %target.username, "SSH interactive terminal opened");
    if let Some(command) = target.startup_command {
        let command = if command.ends_with('\n') {
            command
        } else {
            format!("{command}\n")
        };
        if let Err(error) = channel.data(command.as_bytes()).await {
            send_error(&mut socket, format!("Shell error: {error}")).await;
            return;
        }
    }

    loop {
        tokio::select! {
            message = socket.recv() => {
                let Some(message) = message else { break };
                match message {
                    Ok(Message::Text(text)) => {
                        let value = match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(value) => value,
                            Err(_) => {
                                send_error(&mut socket, "Invalid SSH message.").await;
                                continue;
                            }
                        };
                        match value.get("type").and_then(serde_json::Value::as_str) {
                            Some("ssh_input") => {
                                if let Some(chunk) = value.get("chunk").and_then(serde_json::Value::as_str)
                                    && channel.data(chunk.as_bytes()).await.is_err()
                                {
                                    break;
                                }
                            }
                            Some("ssh_resize") => {
                                if let Some((columns, rows)) = message_dimensions(&value)
                                    && channel.window_change(columns, rows, 0, 0).await.is_err()
                                {
                                    break;
                                }
                            }
                            _ => send_error(&mut socket, "Invalid SSH message.").await,
                        }
                    }
                    Ok(Message::Binary(bytes)) => {
                        if channel.data(&bytes[..]).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Ping(bytes)) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) | Err(_) => break,
                }
            }
            message = channel.wait() => {
                match message {
                    Some(ChannelMsg::Data { data }) => {
                        if send_output(&mut socket, "stdout", String::from_utf8_lossy(&data)).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if send_output(&mut socket, "stderr", String::from_utf8_lossy(&data)).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { .. }) | None => break,
                    _ => {}
                }
            }
        }
    }
    let _ = channel.eof().await;
    let _ = ssh
        .disconnect(Disconnect::ByApplication, "", "English")
        .await;
}

/// Receive target.
async fn receive_target(
    state: &AppState,
    socket: &mut WebSocket,
    allow_farm_host: bool,
) -> Option<TerminalTarget> {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "receive_target");
    while let Some(message) = socket.recv().await {
        let text = match message {
            Ok(Message::Text(text)) => text,
            Ok(Message::Ping(bytes)) => {
                if socket.send(Message::Pong(bytes)).await.is_err() {
                    return None;
                }
                continue;
            }
            Ok(Message::Close(_)) | Err(_) => return None,
            _ => {
                send_error(socket, "Invalid SSH message.").await;
                continue;
            }
        };
        let message = match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(message) => message,
            Err(_) => {
                send_error(socket, "Invalid SSH message.").await;
                continue;
            }
        };
        match message.get("type").and_then(serde_json::Value::as_str) {
            Some("ssh_start") => {
                let Some(host) = message
                    .get("target")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|host| !host.is_empty())
                else {
                    send_error(socket, "SSH error: target is required.").await;
                    continue;
                };
                let (host, username, password) = if allow_farm_host {
                    let host = if host.eq_ignore_ascii_case("farmcontroller") {
                        "127.0.0.1".to_owned()
                    } else {
                        match approved_controller_host(state, host).await {
                            Ok(host) => host,
                            Err(error) => {
                                send_error(socket, format!("SSH error: {error}")).await;
                                continue;
                            }
                        }
                    };
                    let Some(username) = message
                        .get("username")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|username| valid_username(username))
                    else {
                        send_error(socket, "SSH error: a valid username is required.").await;
                        continue;
                    };
                    let Some(password) = message
                        .get("password")
                        .and_then(serde_json::Value::as_str)
                        .filter(|password| password.len() <= 1_024)
                    else {
                        send_error(socket, "SSH error: password is required.").await;
                        continue;
                    };
                    (host, username.to_owned(), password.to_owned())
                } else {
                    match approved_device_host(state, host).await {
                        Ok(host) => (host, "root".to_owned(), String::new()),
                        Err(error) => {
                            send_error(socket, format!("SSH error: {error}")).await;
                            continue;
                        }
                    }
                };
                return Some(TerminalTarget {
                    host,
                    port: message_port(&message),
                    username,
                    password,
                    startup_command: None,
                });
            }
            Some("rtos_ssh_start") => match resolve_rtos_target(state, &message).await {
                Ok(target) => return Some(target),
                Err(error) => send_error(socket, format!("RTOS session error: {error}")).await,
            },
            _ => send_error(socket, "Invalid SSH message.").await,
        }
    }
    None
}

/// Approved device host.
async fn approved_device_host(state: &AppState, requested: &str) -> Result<String, String> {
    use crate::persistence::entity::device;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let _api_trace =
        crate::observability::ApiTrace::enter("socket::routes::terminal", "approved_device_host");
    let orm = state
        .database
        .as_ref()
        .map(|database| database.orm())
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    device::Entity::find()
        .filter(device::Column::DeletedAt.is_null())
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .filter(device::Column::IpAddress.eq(requested))
        .one(orm)
        .await
        .map_err(|error| error.to_string())?
        .and_then(|d| d.ip_address)
        .ok_or_else(|| "target is not an approved device".to_owned())
}

/// Approved controller host.
async fn approved_controller_host(state: &AppState, requested: &str) -> Result<String, String> {
    use crate::persistence::entity::device_controller;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let _api_trace = crate::observability::ApiTrace::enter(
        "socket::routes::terminal",
        "approved_controller_host",
    );
    let orm = state
        .database
        .as_ref()
        .map(|database| database.orm())
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    device_controller::Entity::find()
        .filter(device_controller::Column::DeletedAt.is_null())
        .filter(
            device_controller::Column::Status
                .eq(device_controller::DeviceControllerStatus::Approved),
        )
        .filter(device_controller::Column::IpAddress.eq(requested))
        .one(orm)
        .await
        .map_err(|error| error.to_string())?
        .map(|c| c.ip_address)
        .ok_or_else(|| "target is not an approved device controller".to_owned())
}

/// Resolve rtos target.
async fn resolve_rtos_target(
    state: &AppState,
    message: &serde_json::Value,
) -> Result<TerminalTarget, String> {
    use crate::persistence::entity::{device, device_controller, device_uart_mapping};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let _api_trace =
        crate::observability::ApiTrace::enter("socket::routes::terminal", "resolve_rtos_target");
    let device_id = message
        .get("deviceId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "deviceId is required.".to_owned())?;
    let orm = state
        .database
        .as_ref()
        .map(|database| database.orm())
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    let mapping = device_uart_mapping::Entity::find()
        .filter(device_uart_mapping::Column::DeviceId.eq(device_id))
        .filter(device_uart_mapping::Column::Verified.eq(true))
        .one(orm)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("No mapped device controller found for device {device_id}"))?;
    let device_ok = device::Entity::find()
        .filter(device::Column::DeviceId.eq(device_id))
        .filter(device::Column::DeletedAt.is_null())
        .filter(device::Column::Status.eq(device::DeviceStatus::Approved))
        .one(orm)
        .await
        .map_err(|error| error.to_string())?
        .is_some();
    if !device_ok {
        return Err(format!("No mapped device controller found for device {device_id}"));
    }
    let controller = device_controller::Entity::find()
        .filter(device_controller::Column::DeviceControllerId.eq(&mapping.controller_id))
        .filter(device_controller::Column::DeletedAt.is_null())
        .filter(
            device_controller::Column::Status
                .eq(device_controller::DeviceControllerStatus::Approved),
        )
        .one(orm)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("No mapped device controller found for device {device_id}"))?;
    let startup_command = rtos_command(&mapping.tty, mapping.generation)?;
    Ok(TerminalTarget {
        host: controller.ip_address,
        port: message_port(message),
        username: state.config.device.rtos_username.clone(),
        password: state.config.device.rtos_password.clone(),
        startup_command: Some(startup_command),
    })
}

/// Message port.
fn message_port(message: &serde_json::Value) -> u16 {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "message_port");
    message
        .get("port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .unwrap_or(22)
}

/// Valid username.
fn valid_username(username: &str) -> bool {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "valid_username");
    !username.is_empty()
        && username.len() <= 64
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Message dimensions.
fn message_dimensions(message: &serde_json::Value) -> Option<(u32, u32)> {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "message_dimensions");
    let columns = u32::try_from(message.get("cols")?.as_u64()?).ok()?;
    let rows = u32::try_from(message.get("rows")?.as_u64()?).ok()?;
    ((1..=1_000).contains(&columns) && (1..=1_000).contains(&rows)).then_some((columns, rows))
}

/// Rtos command.
fn rtos_command(tty: &str, generation: i16) -> Result<String, String> {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "rtos_command");
    let tty = tty.trim();
    let suffix = tty
        .strip_prefix("/dev/ttyUSB")
        .or_else(|| tty.strip_prefix("/dev/ttyACM"))
        .ok_or_else(|| "verified UART path is unsupported".to_owned())?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("verified UART path is invalid".to_owned());
    }
    let baud = match generation {
        3 | 5 => 115_200,
        4 => 921_600,
        _ => return Err("verified UART generation is invalid".to_owned()),
    };
    Ok(format!("picocom {tty} -b {baud}"))
}

/// Send error.
async fn send_error(socket: &mut WebSocket, error: impl Into<String>) {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "send_error");
    let _ = send_output(socket, "stderr", error.into()).await;
}

/// Send output.
async fn send_output(
    socket: &mut WebSocket,
    stream: &str,
    chunk: impl Into<String>,
) -> Result<(), axum::Error> {
    let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "send_output");
    socket
        .send(Message::Text(
            serde_json::json!({"type": "ssh", "stream": stream, "chunk": chunk.into()})
                .to_string()
                .into(),
        ))
        .await
}

#[cfg(test)]
mod tests {
    use super::{
        TerminalAuthMethod, message_dimensions, message_port, rtos_command, terminal_auth_method,
        valid_username,
    };

    #[test]
    /// Empty terminal password uses ssh none authentication.
    fn empty_terminal_password_uses_ssh_none_authentication() {
        let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "empty_terminal_password_uses_ssh_none_authentication");
        assert_eq!(terminal_auth_method(""), TerminalAuthMethod::None);
        assert_eq!(
            terminal_auth_method("configured-secret"),
            TerminalAuthMethod::Password
        );
    }

    #[test]
    /// Rtos command uses verified uart and generation baud.
    fn rtos_command_uses_verified_uart_and_generation_baud() {
        let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "rtos_command_uses_verified_uart_and_generation_baud");
        assert_eq!(
            rtos_command("/dev/ttyUSB1", 3).unwrap(),
            "picocom /dev/ttyUSB1 -b 115200"
        );
        assert_eq!(
            rtos_command("/dev/ttyACM2", 4).unwrap(),
            "picocom /dev/ttyACM2 -b 921600"
        );
        assert!(rtos_command("/dev/ttyUSB1;reboot", 5).is_err());
    }

    #[test]
    /// Host terminal username is bounded and shell safe.
    fn host_terminal_username_is_bounded_and_shell_safe() {
        let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "host_terminal_username_is_bounded_and_shell_safe");
        assert!(valid_username("farm-admin"));
        assert!(!valid_username(""));
        assert!(!valid_username("root;reboot"));
    }

    #[test]
    /// Terminal port is bounded to u16.
    fn terminal_port_is_bounded_to_u16() {
        let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "terminal_port_is_bounded_to_u16");
        assert_eq!(message_port(&serde_json::json!({"port": 2222})), 2222);
        assert_eq!(message_port(&serde_json::json!({"port": 70000})), 22);
    }

    #[test]
    /// Terminal dimensions are positive and bounded.
    fn terminal_dimensions_are_positive_and_bounded() {
        let _api_trace = crate::observability::ApiTrace::enter("socket::routes::terminal", "terminal_dimensions_are_positive_and_bounded");
        assert_eq!(
            message_dimensions(&serde_json::json!({"cols": 120, "rows": 40})),
            Some((120, 40))
        );
        assert_eq!(
            message_dimensions(&serde_json::json!({"cols": 0, "rows": 40})),
            None
        );
        assert_eq!(
            message_dimensions(&serde_json::json!({"cols": 120, "rows": 1001})),
            None
        );
    }
}
