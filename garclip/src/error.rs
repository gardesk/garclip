use thiserror::Error;

/// Errors that can occur in the garclip library
#[derive(Error, Debug)]
pub enum Error {
    #[error("failed to connect to X11 display: {0}")]
    X11Connect(#[from] x11rb::errors::ConnectError),

    #[error("X11 connection error: {0}")]
    X11Connection(#[from] x11rb::errors::ConnectionError),

    #[error("X11 reply error: {0}")]
    X11Reply(#[from] x11rb::errors::ReplyError),

    #[error("X11 reply or ID error: {0}")]
    X11ReplyOrId(#[from] x11rb::errors::ReplyOrIdError),

    #[error("invalid screen number: {0}")]
    InvalidScreen(usize),

    #[error("atom not found: {0}")]
    AtomNotFound(String),

    #[error("selection conversion failed")]
    SelectionConversionFailed,

    #[error("selection timeout")]
    SelectionTimeout,

    #[error("unsupported target: {0}")]
    UnsupportedTarget(String),

    #[error("invalid image data")]
    InvalidImageData,

    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("IPC error: {0}")]
    Ipc(String),

    #[error("daemon not running")]
    DaemonNotRunning,

    #[error("entry not found: {0}")]
    EntryNotFound(u64),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
