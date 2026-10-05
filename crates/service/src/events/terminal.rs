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

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
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

pub(super) async fn session(state: Arc<AppState>, mut socket: WebSocket) {
    let Some(target) = receive_target(&state, &mut socket).await else {
        return;
    };
    let timeout = Duration::from_secs(state.config.device.request_timeout_seconds);
    let config = Arc::new(client::Config::default());
    let ssh_client = SshClient {
        host: target.host.clone(),
        port: target.port,
        known_hosts_file: state.config.device.terminal_known_hosts_file.clone().into(),
        accept_unknown_host_keys: state.config.device.terminal_accept_unknown_host_keys,
    };
    let connection = tokio::time::timeout(
        timeout,
        client::connect(config, (target.host.as_str(), target.port), ssh_client),
    )
    .await;
    let mut ssh = match connection {
        Ok(Ok(ssh)) => ssh,
        Ok(Err(error)) => {
            send_error(&mut socket, format!("SSH error: {error}")).await;
            return;
        }
        Err(_) => {
            send_error(&mut socket, "SSH error: connection timed out").await;
            return;
        }
    };
    let authenticated = match ssh
        .authenticate_password(&target.username, &target.password)
        .await
    {
        Ok(result) => result.success(),
        Err(error) => {
            send_error(&mut socket, format!("SSH error: {error}")).await;
            return;
        }
    };
    if !authenticated {
        send_error(&mut socket, "SSH error: authentication failed").await;
        return;
    }
    let mut channel = match ssh.channel_open_session().await {
        Ok(channel) => channel,
        Err(error) => {
            send_error(&mut socket, format!("Shell error: {error}")).await;
            return;
        }
    };
    if let Err(error) = channel
        .request_pty(false, "xterm", 120, 40, 0, 0, &[])
        .await
    {
        send_error(&mut socket, format!("Shell error: {error}")).await;
        return;
    }
    if let Err(error) = channel.request_shell(true).await {
        send_error(&mut socket, format!("Shell error: {error}")).await;
        return;
    }
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
                        if value.get("type").and_then(serde_json::Value::as_str) == Some("ssh_input")
                            && let Some(chunk) = value.get("chunk").and_then(serde_json::Value::as_str)
                            && channel.data(chunk.as_bytes()).await.is_err()
                        {
                            break;
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

async fn receive_target(state: &AppState, socket: &mut WebSocket) -> Option<TerminalTarget> {
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
                let host = match approved_terminal_host(state, host).await {
                    Ok(host) => host,
                    Err(error) => {
                        send_error(socket, format!("SSH error: {error}")).await;
                        continue;
                    }
                };
                return Some(TerminalTarget {
                    host,
                    port: message_port(&message),
                    username: state.config.device.terminal_username.clone(),
                    password: state.config.device.terminal_password.clone(),
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

async fn approved_terminal_host(state: &AppState, requested: &str) -> Result<String, String> {
    let pool = state
        .database
        .as_ref()
        .map(|database| database.pool())
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    sqlx::query_scalar::<_, String>(
        r#"SELECT target."ipAddress"
           FROM (
               SELECT "ipAddress" FROM devices
               WHERE "deletedAt" IS NULL AND status::text = 'approved'
               UNION ALL
               SELECT "ipAddress" FROM device_controllers
               WHERE "deletedAt" IS NULL AND status::text = 'approved'
           ) target
           WHERE target."ipAddress" = $1
           LIMIT 1"#,
    )
    .bind(requested)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "target is not an approved device or controller".to_owned())
}

async fn resolve_rtos_target(
    state: &AppState,
    message: &serde_json::Value,
) -> Result<TerminalTarget, String> {
    let device_id = message
        .get("deviceId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "deviceId is required.".to_owned())?;
    let pool = state
        .database
        .as_ref()
        .map(|database| database.pool())
        .ok_or_else(|| "device persistence is unavailable".to_owned())?;
    let target = sqlx::query_as::<_, (String, Option<String>)>(
        r#"SELECT controller."ipAddress",
                  controller.mappings -> regexp_replace(lower(device."macAddress"), '[:-]', '', 'g') ->> 'uart'
           FROM devices device
           JOIN device_controllers controller
             ON controller."deletedAt" IS NULL
            AND controller."ipAddress" IS NOT NULL
            AND controller.mappings ? regexp_replace(lower(device."macAddress"), '[:-]', '', 'g')
           WHERE device."deviceId" = $1 AND device."deletedAt" IS NULL
           ORDER BY (controller."deviceControllerId" = device."controllerId") DESC,
                    controller."updatedAt" DESC
           LIMIT 1"#,
    )
    .bind(device_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?
    .ok_or_else(|| format!("No mapped device controller found for device {device_id}"))?;
    let startup_command = message
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| default_rtos_command(target.1.as_deref()));
    Ok(TerminalTarget {
        host: target.0,
        port: message_port(message),
        username: state.config.device.rtos_username.clone(),
        password: state.config.device.rtos_password.clone(),
        startup_command: Some(startup_command),
    })
}

fn message_port(message: &serde_json::Value) -> u16 {
    message
        .get("port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .unwrap_or(22)
}

fn default_rtos_command(uart: Option<&str>) -> String {
    let uart = uart.map_or_else(|| "ttyUSB1".to_owned(), adjacent_uart);
    let uart = uart.trim_start_matches("/dev/");
    format!("picocom /dev/{uart} -b 115200")
}

fn adjacent_uart(port: &str) -> String {
    let split = port
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_ascii_digit())
        .map_or(0, |(index, character)| index + character.len_utf8());
    let (prefix, suffix) = port.split_at(split);
    suffix.parse::<u32>().map_or_else(
        |_| port.trim_start_matches("/dev/").to_owned(),
        |number| format!("{}{}", prefix.trim_start_matches("/dev/"), number + 1),
    )
}

async fn send_error(socket: &mut WebSocket, error: impl Into<String>) {
    let _ = send_output(socket, "stderr", error.into()).await;
}

async fn send_output(
    socket: &mut WebSocket,
    stream: &str,
    chunk: impl Into<String>,
) -> Result<(), axum::Error> {
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
    use super::{adjacent_uart, default_rtos_command, message_port};

    #[test]
    fn rtos_command_uses_adjacent_uart() {
        assert_eq!(
            default_rtos_command(Some("/dev/ttyUSB1")),
            "picocom /dev/ttyUSB2 -b 115200"
        );
        assert_eq!(adjacent_uart("2"), "3");
    }

    #[test]
    fn terminal_port_is_bounded_to_u16() {
        assert_eq!(message_port(&serde_json::json!({"port": 2222})), 2222);
        assert_eq!(message_port(&serde_json::json!({"port": 70000})), 22);
    }
}
