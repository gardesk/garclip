use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::io::AsyncWriteExt;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{mpsc, oneshot};

use garclip::config::Config;
use garclip::daemon::DaemonState;
use garclip::ipc::protocol::Command;
use garclip::ipc::{IpcClient, IpcServer, Response};

#[derive(Parser)]
#[command(name = "garclip")]
#[command(about = "X11 clipboard manager with history support")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to config file
    #[arg(short, long, global = true)]
    config: Option<std::path::PathBuf>,

    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the clipboard daemon
    Daemon {
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
    },

    /// Copy text to clipboard
    Copy {
        /// Text to copy (reads from stdin if not provided)
        text: Option<String>,
    },

    /// Get current clipboard content
    Paste,

    /// Show clipboard history
    History {
        /// Maximum entries to show
        #[arg(short, long, default_value = "20")]
        limit: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Select an entry from history
    Select {
        /// Entry ID to select
        id: u64,
    },

    /// Delete an entry from history
    Delete {
        /// Entry ID to delete
        id: u64,
    },

    /// Clear clipboard
    Clear,

    /// Clear history
    ClearHistory {
        /// Keep pinned entries
        #[arg(long)]
        keep_pinned: bool,
    },

    /// Pin an entry
    Pin {
        /// Entry ID to pin
        id: u64,
    },

    /// Unpin an entry
    Unpin {
        /// Entry ID to unpin
        id: u64,
    },

    /// List pinned entries
    ListPinned {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search history
    Search {
        /// Search query
        query: String,

        /// Maximum results
        #[arg(short, long, default_value = "20")]
        limit: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show daemon status
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Reload configuration
    Reload,

    /// Stop the daemon
    Stop,
}

/// A command request with a channel to send the response back
struct CommandRequest {
    command: Command,
    response_tx: oneshot::Sender<Response>,
}

/// Request to register a client for event subscriptions
struct SubscribeRequest {
    client_id: u64,
    events: Vec<String>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

/// Get the PID file path
fn pid_file_path() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join("garclip.pid")
}

/// Check if another instance is running
fn check_existing_instance() -> Result<()> {
    let pid_path = pid_file_path();
    if pid_path.exists() {
        let pid_str = std::fs::read_to_string(&pid_path)?;
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            // Check if process is still running
            let proc_path = format!("/proc/{}", pid);
            if std::path::Path::new(&proc_path).exists() {
                anyhow::bail!("Another garclip instance is running (PID {})", pid);
            }
        }
        // Stale PID file, remove it
        std::fs::remove_file(&pid_path)?;
    }
    Ok(())
}

/// Write the PID file
fn write_pid_file() -> Result<()> {
    let pid_path = pid_file_path();
    std::fs::write(&pid_path, std::process::id().to_string())?;
    Ok(())
}

/// Remove the PID file
fn remove_pid_file() {
    let pid_path = pid_file_path();
    let _ = std::fs::remove_file(pid_path);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up logging
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
        )
        .init();

    // Load config
    let config = if let Some(path) = &cli.config {
        Config::load(path).context("Failed to load config")?
    } else {
        Config::load_default()
    };

    match cli.command {
        Commands::Daemon { foreground: _ } => run_daemon(config).await,
        cmd => run_client_command(config, cmd).await,
    }
}

async fn run_daemon(config: Config) -> Result<()> {
    tracing::info!("Starting garclip daemon");

    // Check for existing instance
    check_existing_instance()?;

    // Write PID file
    write_pid_file()?;

    // Channel for events (clipboard changes, etc.)
    let (event_tx, mut event_rx) = mpsc::channel(100);

    // Channel for commands from IPC clients
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<CommandRequest>(100);

    // Channel for subscription requests
    let (sub_tx, mut sub_rx) = mpsc::channel::<SubscribeRequest>(100);

    // Channel to signal shutdown
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // Set up signal handlers
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;

    // Create daemon state
    let mut state = DaemonState::new(config.clone(), event_tx)?;

    // Create IPC server
    let ipc_server = IpcServer::new(config.socket_path())?;

    // Track connected clients with subscriptions
    let mut subscribed_clients: HashMap<u64, (Vec<String>, tokio::net::unix::OwnedWriteHalf)> =
        HashMap::new();
    let mut next_client_id = 0u64;

    // Main event loop
    let poll_interval = state.poll_interval();
    let mut poll_timer = tokio::time::interval(poll_interval);

    // Auto-save timer (every 5 minutes)
    let mut save_timer = tokio::time::interval(Duration::from_secs(300));

    loop {
        tokio::select! {
            // Poll clipboard for changes
            _ = poll_timer.tick() => {
                if let Err(e) = state.poll_clipboard() {
                    tracing::error!("Error polling clipboard: {}", e);
                }
                if let Err(e) = state.process_x11_events() {
                    tracing::error!("Error processing X11 events: {}", e);
                }
            }

            // Accept new IPC connections
            client = ipc_server.accept() => {
                match client {
                    Ok(client) => {
                        let client_id = next_client_id;
                        next_client_id += 1;

                        let cmd_tx = cmd_tx.clone();
                        let sub_tx = sub_tx.clone();
                        let shutdown_tx = shutdown_tx.clone();

                        // Spawn task to handle this client
                        tokio::spawn(async move {
                            handle_client(client, client_id, cmd_tx, sub_tx, shutdown_tx).await
                        });
                    }
                    Err(e) => {
                        tracing::error!("Error accepting client: {}", e);
                    }
                }
            }

            // Handle commands from IPC clients
            Some(req) = cmd_rx.recv() => {
                let response = state.handle_command(req.command).await;
                let _ = req.response_tx.send(response);
            }

            // Handle subscription requests
            Some(sub_req) = sub_rx.recv() => {
                tracing::debug!("Client {} subscribed to: {:?}", sub_req.client_id, sub_req.events);
                subscribed_clients.insert(sub_req.client_id, (sub_req.events, sub_req.writer));
            }

            // Broadcast events to subscribed clients
            Some(event) = event_rx.recv() => {
                let event_type = match &event {
                    garclip::Event::ClipboardChanged { .. } => "clipboard_changed",
                    garclip::Event::HistoryCleared => "history_cleared",
                    garclip::Event::EntryPinned { .. } => "entry_pinned",
                    garclip::Event::EntryUnpinned { .. } => "entry_unpinned",
                    garclip::Event::EntryDeleted { .. } => "entry_deleted",
                    garclip::Event::EntrySelected { .. } => "entry_selected",
                };

                let json = serde_json::to_string(&event).unwrap_or_default();
                let mut to_remove = Vec::new();

                for (&id, (events, writer)) in subscribed_clients.iter_mut() {
                    // Check if client is subscribed to this event type
                    if events.contains(&"all".to_string()) || events.contains(&event_type.to_string()) {
                        if writer.write_all(format!("{}\n", json).as_bytes()).await.is_err() {
                            to_remove.push(id);
                        }
                    }
                }

                for id in to_remove {
                    subscribed_clients.remove(&id);
                }
            }

            // Auto-save history
            _ = save_timer.tick() => {
                if let Err(e) = state.save_history() {
                    tracing::error!("Error saving history: {}", e);
                }
            }

            // Handle shutdown request from client
            _ = shutdown_rx.recv() => {
                tracing::info!("Received shutdown request");
                break;
            }

            // Handle SIGTERM
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM, shutting down");
                break;
            }

            // Handle SIGINT (Ctrl+C)
            _ = sigint.recv() => {
                tracing::info!("Received SIGINT, shutting down");
                break;
            }

            // Handle SIGHUP (reload config)
            _ = sighup.recv() => {
                tracing::info!("Received SIGHUP, reloading configuration");
                if let Err(e) = state.reload_config() {
                    tracing::error!("Error reloading config: {}", e);
                }
            }
        }
    }

    // Save history on exit
    state.save_history()?;

    // Clean up PID file
    remove_pid_file();

    tracing::info!("Daemon stopped");
    Ok(())
}

async fn handle_client(
    client: IpcClient,
    client_id: u64,
    cmd_tx: mpsc::Sender<CommandRequest>,
    sub_tx: mpsc::Sender<SubscribeRequest>,
    shutdown_tx: mpsc::Sender<()>,
) {
    let (reader, writer) = client.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = Some(writer);

    use tokio::io::AsyncBufReadExt;

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                // EOF - client disconnected
                tracing::debug!("Client {} disconnected", client_id);
                break;
            }
            Ok(_) => {
                let cmd: Command = match serde_json::from_str(line.trim()) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("Invalid command from client {}: {}", client_id, e);
                        if let Some(ref mut w) = writer {
                            let resp = Response::err(format!("Invalid command: {}", e));
                            let json = serde_json::to_string(&resp).unwrap_or_default();
                            let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                        }
                        continue;
                    }
                };

                tracing::debug!("Client {} sent: {:?}", client_id, cmd);

                // Handle special commands
                match &cmd {
                    Command::Subscribe { events } => {
                        // Take ownership of writer for subscription streaming
                        if let Some(w) = writer.take() {
                            let _ = sub_tx
                                .send(SubscribeRequest {
                                    client_id,
                                    events: events.clone(),
                                    writer: w,
                                })
                                .await;
                        }
                        // Client is now in subscription mode, exit handler
                        // (events will be sent by the main loop)
                        break;
                    }
                    Command::Quit => {
                        // Send OK response then signal shutdown
                        if let Some(ref mut w) = writer {
                            let resp = Response::ok();
                            let json = serde_json::to_string(&resp).unwrap_or_default();
                            let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                        }
                        let _ = shutdown_tx.send(()).await;
                        break;
                    }
                    _ => {
                        // Send command to daemon and wait for response
                        let (response_tx, response_rx) = oneshot::channel();
                        let req = CommandRequest {
                            command: cmd,
                            response_tx,
                        };

                        if cmd_tx.send(req).await.is_err() {
                            tracing::error!("Failed to send command to daemon");
                            break;
                        }

                        match response_rx.await {
                            Ok(response) => {
                                if let Some(ref mut w) = writer {
                                    let json = serde_json::to_string(&response).unwrap_or_default();
                                    if w.write_all(format!("{}\n", json).as_bytes()).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(_) => {
                                tracing::error!("Failed to receive response from daemon");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Error reading from client {}: {}", client_id, e);
                break;
            }
        }
    }
}

async fn run_client_command(config: Config, cmd: Commands) -> Result<()> {
    let socket_path = config.socket_path();

    let ipc_cmd = match &cmd {
        Commands::Copy { text } => {
            let text = if let Some(t) = text {
                t.clone()
            } else {
                // Read from stdin
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            };
            Command::Copy { text }
        }
        Commands::Paste => Command::Paste,
        Commands::History { limit, .. } => Command::History { limit: *limit },
        Commands::Select { id } => Command::Select { id: *id },
        Commands::Delete { id } => Command::Delete { id: *id },
        Commands::Clear => Command::Clear,
        Commands::ClearHistory { keep_pinned } => Command::ClearHistory {
            keep_pinned: *keep_pinned,
        },
        Commands::Pin { id } => Command::Pin { id: *id },
        Commands::Unpin { id } => Command::Unpin { id: *id },
        Commands::ListPinned { .. } => Command::ListPinned,
        Commands::Search { query, limit, .. } => Command::Search {
            query: query.clone(),
            limit: *limit,
        },
        Commands::Status { .. } => Command::Status,
        Commands::Reload => Command::Reload,
        Commands::Stop => Command::Quit,
        Commands::Daemon { .. } => unreachable!(),
    };

    let response = garclip::ipc::send_command(&socket_path, &ipc_cmd).await?;

    // Handle response based on command type
    match cmd {
        Commands::Paste => {
            if response.success {
                if let Some(data) = response.data {
                    let paste: garclip::ipc::protocol::PasteResponse =
                        serde_json::from_value(data)?;
                    if let Some(text) = paste.text {
                        print!("{}", text);
                    } else if let Some(_img) = paste.image_data {
                        eprintln!("[Image content - use --json to get base64 data]");
                    }
                }
            } else {
                eprintln!("Error: {}", response.error.unwrap_or_default());
            }
        }
        Commands::History { json, .. }
        | Commands::ListPinned { json }
        | Commands::Search { json, .. } => {
            if response.success {
                if json {
                    println!("{}", serde_json::to_string_pretty(&response.data)?);
                } else if let Some(data) = response.data {
                    let entries: Vec<garclip::ipc::protocol::EntryInfo> =
                        serde_json::from_value(data)?;
                    for entry in entries {
                        let pin_marker = if entry.pinned { "*" } else { " " };
                        println!(
                            "{}{:>4} | {:>8} | {}",
                            pin_marker, entry.id, entry.content_type, entry.preview
                        );
                    }
                }
            } else {
                eprintln!("Error: {}", response.error.unwrap_or_default());
            }
        }
        Commands::Status { json } => {
            if response.success {
                if json {
                    println!("{}", serde_json::to_string_pretty(&response.data)?);
                } else if let Some(data) = response.data {
                    let status: garclip::ipc::protocol::StatusInfo =
                        serde_json::from_value(data)?;
                    println!("History entries: {}", status.history_count);
                    println!("Pinned entries:  {}", status.pinned_count);
                    println!("Owns clipboard:  {}", status.owns_clipboard);
                    println!("Watch PRIMARY:   {}", status.watching_primary);
                    println!("Uptime:          {}s", status.uptime_secs);
                    if let Some(preview) = status.current_preview {
                        println!("Current:         {}", preview);
                    }
                }
            } else {
                eprintln!("Error: {}", response.error.unwrap_or_default());
            }
        }
        _ => {
            if !response.success {
                eprintln!("Error: {}", response.error.unwrap_or_default());
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
