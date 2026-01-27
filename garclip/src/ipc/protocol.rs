use serde::{Deserialize, Serialize};

use crate::clipboard::ClipboardEntry;

/// IPC commands sent from clients to the daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Copy text to clipboard
    Copy {
        text: String,
    },

    /// Copy image to clipboard (base64 encoded)
    CopyImage {
        data: String,
        mime_type: String,
    },

    /// Get current clipboard content
    Paste,

    /// Get clipboard history
    History {
        #[serde(default = "default_limit")]
        limit: usize,
    },

    /// Select an entry from history (makes it current)
    Select {
        id: u64,
    },

    /// Delete an entry from history
    Delete {
        id: u64,
    },

    /// Clear the clipboard
    Clear,

    /// Clear history
    ClearHistory {
        #[serde(default)]
        keep_pinned: bool,
    },

    /// Pin an entry
    Pin {
        id: u64,
    },

    /// Unpin an entry
    Unpin {
        id: u64,
    },

    /// List pinned entries
    ListPinned,

    /// Search history
    Search {
        query: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },

    /// Get daemon status
    Status,

    /// Reload configuration
    Reload,

    /// Shutdown daemon
    Quit,

    /// Subscribe to events
    Subscribe {
        events: Vec<String>,
    },
}

fn default_limit() -> usize {
    50
}

/// Response from the daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    /// Create a success response
    pub fn ok() -> Self {
        Self {
            success: true,
            data: None,
            error: None,
        }
    }

    /// Create a success response with data
    pub fn ok_with_data<T: Serialize>(data: T) -> Self {
        Self {
            success: true,
            data: Some(serde_json::to_value(data).unwrap_or(serde_json::Value::Null)),
            error: None,
        }
    }

    /// Create an error response
    pub fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

/// Events sent to subscribed clients
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Clipboard content changed
    ClipboardChanged {
        id: u64,
        preview: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        content_type: String,
    },

    /// History was cleared
    HistoryCleared,

    /// Entry was pinned
    EntryPinned {
        id: u64,
    },

    /// Entry was unpinned
    EntryUnpinned {
        id: u64,
    },

    /// Entry was deleted
    EntryDeleted {
        id: u64,
    },

    /// Entry was selected
    EntrySelected {
        id: u64,
    },
}

/// Status information returned by Status command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusInfo {
    /// Number of entries in history
    pub history_count: usize,

    /// Number of pinned entries
    pub pinned_count: usize,

    /// Whether we own the CLIPBOARD selection
    pub owns_clipboard: bool,

    /// Whether we're watching PRIMARY
    pub watching_primary: bool,

    /// Current clipboard preview
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_preview: Option<String>,

    /// Current clipboard content type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_type: Option<String>,

    /// Daemon uptime in seconds
    pub uptime_secs: u64,
}

/// Entry info returned in history listings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryInfo {
    pub id: u64,
    pub preview: String,
    pub content_type: String,
    pub size: usize,
    pub timestamp: String,
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl From<&ClipboardEntry> for EntryInfo {
    fn from(entry: &ClipboardEntry) -> Self {
        let content_type = if entry.content.is_text() {
            "text"
        } else {
            "image"
        };

        Self {
            id: entry.id,
            preview: entry.preview(100),
            content_type: content_type.to_string(),
            size: entry.content.size(),
            timestamp: entry.timestamp.to_rfc3339(),
            pinned: entry.pinned,
            source: entry.source.clone(),
        }
    }
}

/// Paste response with full content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasteResponse {
    pub id: u64,
    pub content_type: String,

    /// Text content (if text)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Image data base64 (if image)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_data: Option<String>,

    /// Image MIME type (if image)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,

    /// File URIs (if files)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_uris: Option<Vec<String>>,

    /// Whether this is a cut operation (if files)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_cut: Option<bool>,
}
