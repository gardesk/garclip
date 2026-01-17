use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::error::{Error, Result};
use crate::ipc::protocol::{Command, Event, Response};

/// IPC server for daemon control
pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
}

impl IpcServer {
    /// Create a new IPC server at the given path
    pub fn new<P: AsRef<Path>>(socket_path: P) -> Result<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();

        // Remove old socket if it exists
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }

        // Create parent directory if needed
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Bind to socket
        let listener = UnixListener::bind(&socket_path)?;

        // Set permissions (user only)
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;

        tracing::info!("IPC server listening on {:?}", socket_path);

        Ok(Self {
            listener,
            socket_path,
        })
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Accept a new client connection
    pub async fn accept(&self) -> Result<IpcClient> {
        let (stream, _addr) = self.listener.accept().await?;
        Ok(IpcClient::new(stream))
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        // Clean up socket file
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// A connected IPC client
pub struct IpcClient {
    stream: UnixStream,
    subscriptions: HashSet<String>,
}

impl IpcClient {
    /// Create a new client from a stream
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            subscriptions: HashSet::new(),
        }
    }

    /// Read a command from the client
    pub async fn read_command(&mut self) -> Result<Option<Command>> {
        let mut reader = BufReader::new(&mut self.stream);
        let mut line = String::new();

        match reader.read_line(&mut line).await {
            Ok(0) => Ok(None), // EOF
            Ok(_) => {
                let cmd: Command = serde_json::from_str(line.trim())?;
                Ok(Some(cmd))
            }
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Send a response to the client
    pub async fn send_response(&mut self, response: &Response) -> Result<()> {
        let json = serde_json::to_string(response)?;
        self.stream.write_all(json.as_bytes()).await?;
        self.stream.write_all(b"\n").await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Send an event to the client (if subscribed)
    pub async fn send_event(&mut self, event: &Event) -> Result<bool> {
        let event_type = match event {
            Event::ClipboardChanged { .. } => "clipboard_changed",
            Event::HistoryCleared => "history_cleared",
            Event::EntryPinned { .. } => "entry_pinned",
            Event::EntryUnpinned { .. } => "entry_unpinned",
            Event::EntryDeleted { .. } => "entry_deleted",
            Event::EntrySelected { .. } => "entry_selected",
        };

        // Check if client is subscribed to this event type or "all"
        if !self.subscriptions.contains(event_type) && !self.subscriptions.contains("all") {
            return Ok(false);
        }

        let json = serde_json::to_string(event)?;
        self.stream.write_all(json.as_bytes()).await?;
        self.stream.write_all(b"\n").await?;
        self.stream.flush().await?;
        Ok(true)
    }

    /// Subscribe to events
    pub fn subscribe(&mut self, events: Vec<String>) {
        for event in events {
            self.subscriptions.insert(event);
        }
    }

    /// Check if client has any subscriptions
    pub fn has_subscriptions(&self) -> bool {
        !self.subscriptions.is_empty()
    }

    /// Get the underlying stream for select/poll
    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }

    /// Split into read and write halves
    pub fn into_split(self) -> (tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf) {
        self.stream.into_split()
    }
}

/// Channel-based event broadcaster for multiple clients
pub struct EventBroadcaster {
    sender: mpsc::Sender<Event>,
}

impl EventBroadcaster {
    /// Create a new broadcaster
    pub fn new() -> (Self, mpsc::Receiver<Event>) {
        let (sender, receiver) = mpsc::channel(100);
        (Self { sender }, receiver)
    }

    /// Broadcast an event
    pub async fn broadcast(&self, event: Event) {
        let _ = self.sender.send(event).await;
    }
}
