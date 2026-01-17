use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Configuration for garclip
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// History settings
    pub history: HistoryConfig,

    /// Behavior settings
    pub behavior: BehaviorConfig,

    /// Filter settings
    pub filters: FilterConfig,

    /// Daemon settings
    pub daemon: DaemonConfig,
}

/// History configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// Maximum number of entries to keep
    pub max_entries: usize,

    /// Persist history across restarts
    pub persist: bool,

    /// Custom path for history file (default: ~/.local/share/garclip/history.json)
    #[serde(default)]
    pub persist_path: Option<PathBuf>,
}

/// Behavior configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
    /// Watch PRIMARY selection (middle-click paste)
    pub watch_primary: bool,

    /// Watch CLIPBOARD selection (Ctrl+C)
    pub watch_clipboard: bool,

    /// Deduplicate entries (move to front instead of adding)
    pub deduplicate: bool,

    /// Ignore empty clipboard content
    pub ignore_empty: bool,

    /// Minimum text length to record
    pub min_length: usize,

    /// Maximum text length to record (bytes)
    pub max_length: usize,

    /// Maximum image size to record (bytes)
    pub max_image_size: usize,

    /// Poll interval in milliseconds
    pub poll_interval_ms: u64,
}

/// Filter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    /// Regex patterns to ignore (matched against text content)
    #[serde(default)]
    pub ignore_patterns: Vec<String>,

    /// Window classes to ignore
    #[serde(default)]
    pub ignore_classes: Vec<String>,
}

/// Daemon configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Custom socket path (default: $XDG_RUNTIME_DIR/garclip.sock)
    #[serde(default)]
    pub socket_path: Option<PathBuf>,

    /// Log level
    pub log_level: String,

    /// Log file path (default: none, log to stdout)
    #[serde(default)]
    pub log_file: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            history: HistoryConfig::default(),
            behavior: BehaviorConfig::default(),
            filters: FilterConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            persist: true,
            persist_path: None,
        }
    }
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            watch_primary: true,
            watch_clipboard: true,
            deduplicate: true,
            ignore_empty: true,
            min_length: 1,
            max_length: 10 * 1024 * 1024, // 10MB
            max_image_size: 50 * 1024 * 1024, // 50MB
            poll_interval_ms: 250,
        }
    }
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            ignore_patterns: vec![],
            ignore_classes: vec![],
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: None,
            log_level: "info".to_string(),
            log_file: None,
        }
    }
}

impl Config {
    /// Load configuration from a file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load from default location or return defaults
    pub fn load_default() -> Self {
        if let Some(path) = Self::default_path() {
            if path.exists() {
                match Self::load(&path) {
                    Ok(config) => return config,
                    Err(e) => {
                        tracing::warn!("Failed to load config from {:?}: {}", path, e);
                    }
                }
            }
        }
        Self::default()
    }

    /// Get the default config path
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("garclip").join("config.toml"))
    }

    /// Get the history file path
    pub fn history_path(&self) -> PathBuf {
        self.history
            .persist_path
            .clone()
            .unwrap_or_else(|| {
                dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("garclip")
                    .join("history.json")
            })
    }

    /// Get the socket path
    pub fn socket_path(&self) -> PathBuf {
        self.daemon.socket_path.clone().unwrap_or_else(|| {
            std::env::var("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
                .join("garclip.sock")
        })
    }

    /// Save config to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| crate::error::Error::Other(e.to_string()))?;

        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(path, content)?;
        Ok(())
    }

    /// Save to default location
    pub fn save_default(&self) -> Result<()> {
        if let Some(path) = Self::default_path() {
            self.save(path)
        } else {
            Err(crate::error::Error::Config("No config directory".to_string()))
        }
    }
}
