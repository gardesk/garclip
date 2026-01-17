use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::error::{Error, Result};
use crate::ipc::protocol::{Command, Response};

/// Send a command to the daemon and get the response
pub async fn send_command<P: AsRef<Path>>(socket_path: P, cmd: &Command) -> Result<Response> {
    let mut stream = UnixStream::connect(socket_path.as_ref())
        .await
        .map_err(|_| Error::DaemonNotRunning)?;

    // Send command
    let json = serde_json::to_string(cmd)?;
    stream.write_all(json.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;

    // Read response
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let response: Response = serde_json::from_str(line.trim())?;
    Ok(response)
}

/// Send a command synchronously (blocking)
pub fn send_command_blocking<P: AsRef<Path>>(socket_path: P, cmd: &Command) -> Result<Response> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Other(e.to_string()))?;

    rt.block_on(send_command(socket_path, cmd))
}

/// Get the default socket path
pub fn default_socket_path() -> std::path::PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        .join("garclip.sock")
}
