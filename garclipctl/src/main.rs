use anyhow::Result;
use clap::{Parser, Subcommand};

use garclip::ipc::protocol::{Command, EntryInfo, PasteResponse, StatusInfo};
use garclip::ipc::{client::default_socket_path, send_command_blocking};

#[derive(Parser)]
#[command(name = "garclipctl")]
#[command(about = "Control the garclip clipboard daemon")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Custom socket path
    #[arg(short, long, global = true)]
    socket: Option<std::path::PathBuf>,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
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
    },

    /// Select an entry from history (makes it current)
    Select {
        /// Entry ID
        id: u64,
    },

    /// Delete an entry from history
    Delete {
        /// Entry ID
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

    /// Pin an entry (won't be removed by history limit)
    Pin {
        /// Entry ID
        id: u64,
    },

    /// Unpin an entry
    Unpin {
        /// Entry ID
        id: u64,
    },

    /// List pinned entries
    Pinned,

    /// Search history
    Search {
        /// Search query
        query: String,

        /// Maximum results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Show daemon status
    Status,

    /// Reload daemon configuration
    Reload,

    /// Stop the daemon
    Stop,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(default_socket_path);

    let cmd = match &cli.command {
        Commands::Copy { text } => {
            let text = if let Some(t) = text {
                t.clone()
            } else {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            };
            Command::Copy { text }
        }
        Commands::Paste => Command::Paste,
        Commands::History { limit } => Command::History { limit: *limit },
        Commands::Select { id } => Command::Select { id: *id },
        Commands::Delete { id } => Command::Delete { id: *id },
        Commands::Clear => Command::Clear,
        Commands::ClearHistory { keep_pinned } => Command::ClearHistory {
            keep_pinned: *keep_pinned,
        },
        Commands::Pin { id } => Command::Pin { id: *id },
        Commands::Unpin { id } => Command::Unpin { id: *id },
        Commands::Pinned => Command::ListPinned,
        Commands::Search { query, limit } => Command::Search {
            query: query.clone(),
            limit: *limit,
        },
        Commands::Status => Command::Status,
        Commands::Reload => Command::Reload,
        Commands::Stop => Command::Quit,
    };

    let response = send_command_blocking(&socket, &cmd)?;

    if !response.success {
        eprintln!("Error: {}", response.error.unwrap_or_default());
        std::process::exit(1);
    }

    // Handle output based on command
    match cli.command {
        Commands::Paste => {
            if let Some(data) = response.data {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&data)?);
                } else {
                    let paste: PasteResponse = serde_json::from_value(data)?;
                    if let Some(text) = paste.text {
                        print!("{}", text);
                    } else if paste.image_data.is_some() {
                        eprintln!("[Image - use --json for base64 data]");
                    }
                }
            }
        }
        Commands::History { .. } | Commands::Pinned | Commands::Search { .. } => {
            if let Some(data) = response.data {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&data)?);
                } else {
                    let entries: Vec<EntryInfo> = serde_json::from_value(data)?;
                    if entries.is_empty() {
                        println!("No entries");
                    } else {
                        for entry in entries {
                            let pin = if entry.pinned { "*" } else { " " };
                            println!(
                                "{}{:>4} | {:>5} | {}",
                                pin, entry.id, entry.content_type, entry.preview
                            );
                        }
                    }
                }
            }
        }
        Commands::Status => {
            if let Some(data) = response.data {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&data)?);
                } else {
                    let status: StatusInfo = serde_json::from_value(data)?;
                    println!("History:        {} entries", status.history_count);
                    println!("Pinned:         {} entries", status.pinned_count);
                    println!("Owns clipboard: {}", status.owns_clipboard);
                    println!("Watch PRIMARY:  {}", status.watching_primary);
                    println!("Uptime:         {}s", status.uptime_secs);
                    if let Some(preview) = status.current_preview {
                        println!("Current:        {}", preview);
                    }
                }
            }
        }
        _ => {
            // Commands with no output on success
        }
    }

    Ok(())
}
