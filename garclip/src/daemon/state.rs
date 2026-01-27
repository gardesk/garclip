use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine};
use tokio::sync::mpsc;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{SelectionClearEvent, SelectionRequestEvent};
use x11rb::protocol::Event as X11Event;
use x11rb::protocol::xfixes::SelectionNotifyEvent as XFixesSelectionNotifyEvent;
use x11rb::rust_connection::RustConnection;

use crate::clipboard::{ClipboardContent, ClipboardHistory, ClipboardManager};
use crate::config::Config;
use crate::error::Result;
use crate::ipc::protocol::{Command, EntryInfo, Event, PasteResponse, Response, StatusInfo};

/// Main daemon state
pub struct DaemonState {
    config: Config,
    manager: ClipboardManager,
    start_time: Instant,
    event_tx: mpsc::Sender<Event>,
}

impl DaemonState {
    /// Create a new daemon state
    pub fn new(config: Config, event_tx: mpsc::Sender<Event>) -> Result<Self> {
        // Connect to X11
        let (conn, screen_num) = RustConnection::connect(None)?;
        let conn = Arc::new(conn);

        // Load or create history
        let history_path = config.history_path();
        let history = if config.history.persist && history_path.exists() {
            tracing::info!("Loading history from {:?}", history_path);
            ClipboardHistory::load_or_new(
                &history_path,
                config.history.max_entries,
                config.behavior.deduplicate,
            )?
        } else {
            ClipboardHistory::new(config.history.max_entries, config.behavior.deduplicate)
        };

        // Create clipboard manager
        let mut manager = ClipboardManager::new(
            conn,
            screen_num,
            history,
            &config,
        )?;

        // Start XFixes monitoring
        manager.start_watching()?;

        Ok(Self {
            config,
            manager,
            start_time: Instant::now(),
            event_tx,
        })
    }

    /// Get the config
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Reload configuration
    pub fn reload_config(&mut self) -> Result<()> {
        self.config = Config::load_default();
        self.manager.reload_filter(&self.config);
        tracing::info!("Configuration reloaded");
        Ok(())
    }

    /// Poll for clipboard changes (fallback when XFixes not active)
    pub fn poll_clipboard(&mut self) -> Result<()> {
        // Skip if XFixes is active
        if self.manager.xfixes_active() {
            return Ok(());
        }

        // Poll CLIPBOARD
        if self.config.behavior.watch_clipboard {
            if let Some(id) = self.manager.poll_clipboard()? {
                self.send_clipboard_event(id);
            }
        }

        // Poll PRIMARY
        if self.config.behavior.watch_primary {
            if let Some(id) = self.manager.poll_primary()? {
                self.send_clipboard_event(id);
            }
        }

        Ok(())
    }

    /// Send clipboard change event
    fn send_clipboard_event(&self, id: u64) {
        if let Some(entry) = self.manager.history().get(id) {
            let content_type = if entry.content.is_text() {
                "text"
            } else {
                "image"
            };

            let event = Event::ClipboardChanged {
                id,
                preview: entry.preview(100),
                source: entry.source.clone(),
                content_type: content_type.to_string(),
            };

            let _ = self.event_tx.try_send(event);
        }
    }

    /// Process X11 events
    pub fn process_x11_events(&mut self) -> Result<()> {
        let conn = self.manager.conn().clone();

        while let Ok(Some(event)) = conn.poll_for_event() {
            match event {
                X11Event::XfixesSelectionNotify(ref xfixes_event) => {
                    if let Some(id) = self.handle_xfixes_selection_notify(xfixes_event)? {
                        self.send_clipboard_event(id);
                    }
                }
                X11Event::SelectionRequest(ref req) => {
                    self.handle_selection_request(req)?;
                }
                X11Event::SelectionClear(ref clear) => {
                    self.handle_selection_clear(clear);
                }
                _ => {}
            }
        }

        // Check for debounced PRIMARY content ready to commit
        if let Some(id) = self.manager.commit_pending_primary() {
            self.send_clipboard_event(id);
        }

        Ok(())
    }

    /// Handle XFixes SelectionNotify event
    fn handle_xfixes_selection_notify(
        &mut self,
        event: &XFixesSelectionNotifyEvent,
    ) -> Result<Option<u64>> {
        self.manager.handle_xfixes_selection_notify(event)
    }

    /// Handle a selection request
    fn handle_selection_request(&self, event: &SelectionRequestEvent) -> Result<()> {
        tracing::trace!(
            "SelectionRequest: selection={}, target={}, requestor={}",
            event.selection,
            event.target,
            event.requestor
        );
        self.manager.handle_selection_request(event)?;
        Ok(())
    }

    /// Handle selection clear
    fn handle_selection_clear(&mut self, event: &SelectionClearEvent) {
        tracing::trace!("SelectionClear: selection={}", event.selection);
        self.manager.handle_selection_clear(event.selection);
    }

    /// Handle an IPC command
    pub async fn handle_command(&mut self, cmd: Command) -> Response {
        match cmd {
            Command::Copy { text } => self.cmd_copy(text),
            Command::CopyImage { data, mime_type } => self.cmd_copy_image(data, mime_type),
            Command::Paste => self.cmd_paste(),
            Command::History { limit } => self.cmd_history(limit),
            Command::Select { id } => self.cmd_select(id),
            Command::Delete { id } => self.cmd_delete(id),
            Command::Clear => self.cmd_clear(),
            Command::ClearHistory { keep_pinned } => self.cmd_clear_history(keep_pinned),
            Command::Pin { id } => self.cmd_pin(id),
            Command::Unpin { id } => self.cmd_unpin(id),
            Command::ListPinned => self.cmd_list_pinned(),
            Command::Search { query, limit } => self.cmd_search(query, limit),
            Command::Status => self.cmd_status(),
            Command::Reload => self.cmd_reload(),
            Command::Quit => Response::ok(), // Handled by caller
            Command::Subscribe { .. } => Response::ok(), // Handled by caller
        }
    }

    fn cmd_copy(&mut self, text: String) -> Response {
        let content = ClipboardContent::Text(text);
        match self.manager.set_clipboard(content) {
            Ok(_) => Response::ok(),
            Err(e) => Response::err(e.to_string()),
        }
    }

    fn cmd_copy_image(&mut self, data: String, mime_type: String) -> Response {
        match STANDARD.decode(&data) {
            Ok(bytes) => {
                let content = ClipboardContent::Image {
                    data: bytes,
                    mime_type,
                };
                match self.manager.set_clipboard(content) {
                    Ok(_) => Response::ok(),
                    Err(e) => Response::err(e.to_string()),
                }
            }
            Err(e) => Response::err(format!("Invalid base64: {}", e)),
        }
    }

    fn cmd_paste(&self) -> Response {
        if let Some(entry) = self.manager.history().current() {
            let response = match &entry.content {
                ClipboardContent::Text(text) => PasteResponse {
                    id: entry.id,
                    content_type: "text".to_string(),
                    text: Some(text.clone()),
                    image_data: None,
                    mime_type: None,
                    file_uris: None,
                    is_cut: None,
                },
                ClipboardContent::Image { data, mime_type } => PasteResponse {
                    id: entry.id,
                    content_type: "image".to_string(),
                    text: None,
                    image_data: Some(STANDARD.encode(data)),
                    mime_type: Some(mime_type.clone()),
                    file_uris: None,
                    is_cut: None,
                },
                ClipboardContent::Files { uris, is_cut } => PasteResponse {
                    id: entry.id,
                    content_type: "files".to_string(),
                    text: None,
                    image_data: None,
                    mime_type: None,
                    file_uris: Some(uris.clone()),
                    is_cut: Some(*is_cut),
                },
            };
            Response::ok_with_data(response)
        } else {
            Response::err("Clipboard is empty")
        }
    }

    fn cmd_history(&self, limit: usize) -> Response {
        let entries: Vec<EntryInfo> = self
            .manager
            .history()
            .list(limit)
            .into_iter()
            .map(EntryInfo::from)
            .collect();
        Response::ok_with_data(entries)
    }

    fn cmd_select(&mut self, id: u64) -> Response {
        match self.manager.select_entry(id) {
            Ok(true) => {
                let _ = self.event_tx.try_send(Event::EntrySelected { id });
                Response::ok()
            }
            Ok(false) => Response::err(format!("Entry {} not found", id)),
            Err(e) => Response::err(e.to_string()),
        }
    }

    fn cmd_delete(&mut self, id: u64) -> Response {
        if self.manager.history_mut().remove(id) {
            let _ = self.event_tx.try_send(Event::EntryDeleted { id });
            Response::ok()
        } else {
            Response::err(format!("Entry {} not found", id))
        }
    }

    fn cmd_clear(&mut self) -> Response {
        match self.manager.clear_clipboard() {
            Ok(_) => Response::ok(),
            Err(e) => Response::err(e.to_string()),
        }
    }

    fn cmd_clear_history(&mut self, keep_pinned: bool) -> Response {
        self.manager.history_mut().clear(keep_pinned);
        let _ = self.event_tx.try_send(Event::HistoryCleared);
        Response::ok()
    }

    fn cmd_pin(&mut self, id: u64) -> Response {
        if self.manager.history_mut().pin(id) {
            let _ = self.event_tx.try_send(Event::EntryPinned { id });
            Response::ok()
        } else {
            Response::err(format!("Entry {} not found", id))
        }
    }

    fn cmd_unpin(&mut self, id: u64) -> Response {
        if self.manager.history_mut().unpin(id) {
            let _ = self.event_tx.try_send(Event::EntryUnpinned { id });
            Response::ok()
        } else {
            Response::err(format!("Entry {} not found", id))
        }
    }

    fn cmd_list_pinned(&self) -> Response {
        let entries: Vec<EntryInfo> = self
            .manager
            .history()
            .list_pinned()
            .into_iter()
            .map(EntryInfo::from)
            .collect();
        Response::ok_with_data(entries)
    }

    fn cmd_search(&self, query: String, limit: usize) -> Response {
        let entries: Vec<EntryInfo> = self
            .manager
            .history()
            .search(&query, limit)
            .into_iter()
            .map(EntryInfo::from)
            .collect();
        Response::ok_with_data(entries)
    }

    fn cmd_status(&self) -> Response {
        let history = self.manager.history();
        let owns_clipboard = self.manager.owns_clipboard().unwrap_or(false);

        let (current_preview, current_type) = if let Some(entry) = history.current() {
            let ctype = if entry.content.is_text() {
                "text"
            } else {
                "image"
            };
            (Some(entry.preview(100)), Some(ctype.to_string()))
        } else {
            (None, None)
        };

        let status = StatusInfo {
            history_count: history.len(),
            pinned_count: history.list_pinned().len(),
            owns_clipboard,
            watching_primary: self.config.behavior.watch_primary,
            current_preview,
            current_type,
            uptime_secs: self.start_time.elapsed().as_secs(),
        };

        Response::ok_with_data(status)
    }

    fn cmd_reload(&mut self) -> Response {
        match self.reload_config() {
            Ok(_) => Response::ok(),
            Err(e) => Response::err(e.to_string()),
        }
    }

    /// Save history to disk
    pub fn save_history(&self) -> Result<()> {
        if self.config.history.persist {
            let path = self.config.history_path();
            tracing::info!("Saving history to {:?}", path);
            self.manager.save_history(&path)?;
        }
        Ok(())
    }

    /// Get the poll interval
    pub fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.config.behavior.poll_interval_ms)
    }
}
