use serde::{Deserialize, Serialize};

/// Content stored in the clipboard
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ClipboardContent {
    /// Plain text content
    Text(String),

    /// Image content with MIME type
    Image {
        /// Raw image data (PNG, JPEG, etc.)
        #[serde(with = "base64_serde")]
        data: Vec<u8>,
        /// MIME type (e.g., "image/png")
        mime_type: String,
    },

    /// File URIs (for file manager copy/paste)
    Files {
        /// File URIs (e.g., "file:///home/user/file.txt")
        uris: Vec<String>,
        /// Whether this is a cut operation (move vs copy)
        is_cut: bool,
    },
}

impl ClipboardContent {
    /// Get a preview string for display
    pub fn preview(&self, max_len: usize) -> String {
        match self {
            ClipboardContent::Text(text) => {
                let preview: String = text
                    .chars()
                    .take(max_len)
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect();
                if text.len() > max_len {
                    format!("{}...", preview)
                } else {
                    preview
                }
            }
            ClipboardContent::Image { data, mime_type } => {
                format!("[Image: {}, {} bytes]", mime_type, data.len())
            }
            ClipboardContent::Files { uris, is_cut } => {
                let action = if *is_cut { "Cut" } else { "Copy" };
                let count = uris.len();
                if count == 1 {
                    // Show the filename for single file
                    let path = uris[0].strip_prefix("file://").unwrap_or(&uris[0]);
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_else(|| path.into());
                    format!("[{}: {}]", action, name)
                } else {
                    format!("[{}: {} files]", action, count)
                }
            }
        }
    }

    /// Check if this content is text
    pub fn is_text(&self) -> bool {
        matches!(self, ClipboardContent::Text(_))
    }

    /// Check if this content is an image
    pub fn is_image(&self) -> bool {
        matches!(self, ClipboardContent::Image { .. })
    }

    /// Check if this content is file URIs
    pub fn is_files(&self) -> bool {
        matches!(self, ClipboardContent::Files { .. })
    }

    /// Get text content if this is text
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ClipboardContent::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Get image data if this is an image
    pub fn as_image(&self) -> Option<(&[u8], &str)> {
        match self {
            ClipboardContent::Image { data, mime_type } => Some((data, mime_type)),
            _ => None,
        }
    }

    /// Get file URIs if this is files
    pub fn as_files(&self) -> Option<(&[String], bool)> {
        match self {
            ClipboardContent::Files { uris, is_cut } => Some((uris, *is_cut)),
            _ => None,
        }
    }

    /// Get the content hash for deduplication
    pub fn hash(&self) -> String {
        match self {
            ClipboardContent::Text(text) => {
                let hash = blake3::hash(text.as_bytes());
                hash.to_hex().to_string()
            }
            ClipboardContent::Image { data, .. } => {
                let hash = blake3::hash(data);
                hash.to_hex().to_string()
            }
            ClipboardContent::Files { uris, is_cut } => {
                // Hash URIs and cut flag together
                let mut hasher = blake3::Hasher::new();
                for uri in uris {
                    hasher.update(uri.as_bytes());
                    hasher.update(b"\n");
                }
                hasher.update(if *is_cut { b"cut" } else { b"copy" });
                hasher.finalize().to_hex().to_string()
            }
        }
    }

    /// Get the size in bytes
    pub fn size(&self) -> usize {
        match self {
            ClipboardContent::Text(text) => text.len(),
            ClipboardContent::Image { data, .. } => data.len(),
            ClipboardContent::Files { uris, .. } => {
                uris.iter().map(|u| u.len()).sum()
            }
        }
    }
}

impl PartialEq for ClipboardContent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ClipboardContent::Text(a), ClipboardContent::Text(b)) => a == b,
            (
                ClipboardContent::Image {
                    data: a,
                    mime_type: ma,
                },
                ClipboardContent::Image {
                    data: b,
                    mime_type: mb,
                },
            ) => a == b && ma == mb,
            (
                ClipboardContent::Files {
                    uris: a,
                    is_cut: ca,
                },
                ClipboardContent::Files {
                    uris: b,
                    is_cut: cb,
                },
            ) => a == b && ca == cb,
            _ => false,
        }
    }
}

impl Eq for ClipboardContent {}

/// A clipboard history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardEntry {
    /// Unique ID for this entry
    pub id: u64,

    /// The clipboard content
    pub content: ClipboardContent,

    /// Source application (window class) if known
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// When the entry was captured
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// Whether this entry is pinned (won't be removed by history limit)
    #[serde(default)]
    pub pinned: bool,

    /// Content hash for deduplication
    pub hash: String,
}

impl ClipboardEntry {
    /// Create a new clipboard entry
    pub fn new(id: u64, content: ClipboardContent, source: Option<String>) -> Self {
        let hash = content.hash();
        Self {
            id,
            content,
            source,
            timestamp: chrono::Utc::now(),
            pinned: false,
            hash,
        }
    }

    /// Get a preview of the content
    pub fn preview(&self, max_len: usize) -> String {
        self.content.preview(max_len)
    }
}

/// Serde module for base64 encoding of binary data
mod base64_serde {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(data: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        STANDARD.encode(data).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        STANDARD.decode(&s).map_err(serde::de::Error::custom)
    }
}
