use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::io::AsyncWriteExt;
use tokio::signal;
use tokio::sync::mpsc;

use garclip::config::Config;
use garclip::daemon::DaemonState;
use garclip::ipc::protocol::Command;
use garclip::ipc::{IpcClient, IpcServer};

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

    // Create event channel
    let (event_tx, mut event_rx) = mpsc::channel(100);

    // Create daemon state
    let mut state = DaemonState::new(config.clone(), event_tx)?;

    // Create IPC server
    let ipc_server = IpcServer::new(config.socket_path())?;

    // Track connected clients with subscriptions
    let mut subscribed_clients: HashMap<u64, tokio::net::unix::OwnedWriteHalf> = HashMap::new();
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
                    Ok(mut client) => {
                        let client_id = next_client_id;
                        next_client_id += 1;

                        // Spawn task to handle this client
                        tokio::spawn(async move {
                            handle_client(&mut client, client_id).await
                        });
                    }
                    Err(e) => {
                        tracing::error!("Error accepting client: {}", e);
                    }
                }
            }

            // Broadcast events to subscribed clients
            Some(event) = event_rx.recv() => {
                let json = serde_json::to_string(&event).unwrap_or_default();
                let mut to_remove = Vec::new();

                for (&id, writer) in subscribed_clients.iter_mut() {
                    if let Err(_) = writer.write_all(format!("{}\n", json).as_bytes()).await {
                        to_remove.push(id);
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

            // Handle shutdown signals
            _ = signal::ctrl_c() => {
                tracing::info!("Received SIGINT, shutting down");
                break;
            }
        }
    }

    // Save history on exit
    state.save_history()?;

    tracing::info!("Daemon stopped");
    Ok(())
}

async fn handle_client(client: &mut IpcClient, _client_id: u64) {
    loop {
        match client.read_command().await {
            Ok(Some(cmd)) => {
                tracing::debug!("Received command: {:?}", cmd);

                // Handle subscribe specially
                if let Command::Subscribe { events } = &cmd {
                    client.subscribe(events.clone());
                    let _ = client.send_response(&garclip::Response::ok()).await;
                    continue;
                }

                // For quit, we'd need to signal the main loop
                if matches!(cmd, Command::Quit) {
                    let _ = client.send_response(&garclip::Response::ok()).await;
                    // In a real implementation, we'd signal shutdown here
                    break;
                }

                // For other commands, we need access to daemon state
                // This is a simplified version - in production, use channels
                let response = garclip::Response::err("Command handling requires daemon state");
                let _ = client.send_response(&response).await;
            }
            Ok(None) => {
                // Client disconnected
                break;
            }
            Err(e) => {
                tracing::error!("Error reading command: {}", e);
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
        Commands::History { json, .. } | Commands::ListPinned { json } | Commands::Search { json, .. } => {
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
