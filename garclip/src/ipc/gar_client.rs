use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Response from gar IPC
#[derive(Debug, Deserialize)]
struct GarResponse {
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<String>,
}

/// Focused window info from gar
#[derive(Debug, Deserialize)]
struct FocusedInfo {
    id: u64,
    workspace: u32,
    floating: bool,
}

/// Client for communicating with gar window manager
pub struct GarClient {
    socket_path: PathBuf,
}

impl GarClient {
    /// Create a new gar client
    pub fn new() -> Self {
        let socket_path = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
            .join("gar.sock");

        Self { socket_path }
    }

    /// Check if gar is available
    pub fn is_available(&self) -> bool {
        self.socket_path.exists()
    }

    /// Get the focused window ID
    pub fn get_focused_window_id(&self) -> Option<u64> {
        if !self.is_available() {
            return None;
        }

        let request = serde_json::json!({
            "command": "get_focused"
        });

        match self.send_request(&request) {
            Ok(response) => {
                if response.success {
                    if let Some(data) = response.data {
                        if let Ok(info) = serde_json::from_value::<FocusedInfo>(data) {
                            return Some(info.id);
                        }
                    }
                }
                None
            }
            Err(e) => {
                tracing::debug!("Failed to query gar: {}", e);
                None
            }
        }
    }

    /// Send a request to gar and get the response
    fn send_request(&self, request: &serde_json::Value) -> std::io::Result<GarResponse> {
        let mut stream = UnixStream::connect(&self.socket_path)?;
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        stream.set_write_timeout(Some(Duration::from_millis(100)))?;

        writeln!(stream, "{}", request)?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;

        serde_json::from_str(&line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

impl Default for GarClient {
    fn default() -> Self {
        Self::new()
    }
}
