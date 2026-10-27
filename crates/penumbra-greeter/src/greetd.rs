use greetd_ipc::{codec::SyncCodec, AuthMessageType, Request, Response};
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;
use tracing::{error, info, warn};

pub enum AuthResult {
    Success,
    Failed(String),
}

pub fn authenticate_and_start(
    socket_path: Option<&str>,
    username: &str,
    password: &str,
    session_cmd: &[String],
    mock_mode: bool,
) -> AuthResult {
    if mock_mode || socket_path.is_none() {
        info!("Running in MOCK authentication mode for user '{}'", username);
        thread::sleep(Duration::from_millis(350));

        if password == "wrong" || password == "error" {
            return AuthResult::Failed("Incorrect password. Please try again.".to_string());
        }

        info!(
            "Mock authentication successful! Would start session: {:?}",
            session_cmd
        );
        return AuthResult::Success;
    }

    let socket = socket_path.unwrap();
    let mut stream = match UnixStream::connect(socket) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to connect to greetd socket at '{}': {:?}", socket, e);
            return AuthResult::Failed(format!("Greetd connection error: {}", e));
        }
    };

    // 1. Create Session for selected user
    if let Err(e) = (Request::CreateSession {
        username: username.to_string(),
    }
    .write_to(&mut stream))
    {
        return AuthResult::Failed(format!("Failed to create greetd session: {}", e));
    }

    // 2. Authentication Loop
    loop {
        match Response::read_from(&mut stream) {
            Ok(Response::AuthMessage {
                auth_message_type,
                auth_message,
            }) => {
                debug!("Greetd auth message: {:?} - {}", auth_message_type, auth_message);
                let response = match auth_message_type {
                    AuthMessageType::Info => {
                        info!("Greetd info: {}", auth_message);
                        None
                    }
                    AuthMessageType::Error => {
                        warn!("Greetd error: {}", auth_message);
                        None
                    }
                    AuthMessageType::Visible => Some(username.to_string()),
                    AuthMessageType::Secret => Some(password.to_string()),
                };

                if let Err(e) = (Request::PostAuthMessageResponse { response }.write_to(&mut stream)) {
                    return AuthResult::Failed(format!("Failed to respond to greetd: {}", e));
                }
            }
            Ok(Response::Success) => {
                info!("Authentication successful for user '{}'!", username);
                break;
            }
            Ok(Response::Error {
                description, ..
            }) => {
                warn!("Authentication failed: {}", description);
                return AuthResult::Failed(description);
            }
            Err(e) => {
                error!("Error reading from greetd stream: {:?}", e);
                return AuthResult::Failed(format!("Greetd communication error: {}", e));
            }
        }
    }

    // 3. Start Session
    let env = vec![
        "XDG_CURRENT_DESKTOP=Penumbra".to_string(),
        "XDG_SESSION_DESKTOP=Penumbra".to_string(),
        "XDG_SESSION_TYPE=wayland".to_string(),
    ];

    if let Err(e) = (Request::StartSession {
        cmd: session_cmd.to_vec(),
        env,
    }
    .write_to(&mut stream))
    {
        return AuthResult::Failed(format!("Failed to start session: {}", e));
    }

    match Response::read_from(&mut stream) {
        Ok(Response::Success) => {
            info!("Session started successfully!");
            AuthResult::Success
        }
        Ok(Response::Error { description, .. }) => {
            error!("Could not start session: {}", description);
            AuthResult::Failed(format!("Could not start session: {}", description))
        }
        Ok(Response::AuthMessage { .. }) => {
            AuthResult::Failed("Unexpected auth message when starting session".to_string())
        }
        Err(e) => AuthResult::Failed(format!("Failed to read session start response: {}", e)),
    }
}
use tracing::debug;
