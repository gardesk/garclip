use std::sync::Arc;

use x11rb::protocol::xfixes::SelectionNotifyEvent as XFixesSelectionNotifyEvent;
use x11rb::protocol::xproto::{Atom, SelectionRequestEvent, Window};
use x11rb::rust_connection::RustConnection;

use crate::clipboard::{ClipboardContent, ClipboardHistory, ContentFilter};
use crate::config::Config;
use crate::error::Result;
use crate::x11::{Atoms, SelectionManager, TransferManager};

/// Main clipboard manager that coordinates X11 selection handling and history
pub struct ClipboardManager {
    selection_mgr: SelectionManager,
    history: ClipboardHistory,
    filter: ContentFilter,

    /// Current clipboard content (what we serve when we're the owner)
    current_clipboard: Option<ClipboardContent>,

    /// Current primary content
    current_primary: Option<ClipboardContent>,

    /// Last known clipboard owner (to detect changes)
    last_clipboard_owner: Window,

    /// Last known primary owner
    last_primary_owner: Window,

    /// Whether to watch PRIMARY selection
    watch_primary: bool,

    /// Whether XFixes monitoring is active
    xfixes_active: bool,
}

impl ClipboardManager {
    /// Create a new clipboard manager
    pub fn new(
        conn: Arc<RustConnection>,
        screen_num: usize,
        history: ClipboardHistory,
        config: &Config,
    ) -> Result<Self> {
        let atoms = Atoms::intern(&*conn)?;
        let selection_mgr = SelectionManager::new(conn, screen_num, atoms)?;
        let filter = ContentFilter::new(&config.behavior, &config.filters);

        Ok(Self {
            selection_mgr,
            history,
            filter,
            current_clipboard: None,
            current_primary: None,
            last_clipboard_owner: 0,
            last_primary_owner: 0,
            watch_primary: config.behavior.watch_primary,
            xfixes_active: false,
        })
    }

    /// Reload filter from new config
    pub fn reload_filter(&mut self, config: &Config) {
        self.filter.reload(&config.behavior, &config.filters);
        self.watch_primary = config.behavior.watch_primary;
    }

    /// Start watching selections via XFixes (event-driven monitoring)
    pub fn start_watching(&mut self) -> Result<()> {
        let atoms = *self.selection_mgr.atoms();

        // Watch CLIPBOARD
        self.selection_mgr.watch_selection(atoms.clipboard)?;

        // Optionally watch PRIMARY
        if self.watch_primary {
            self.selection_mgr.watch_selection(atoms.primary)?;
        }

        self.xfixes_active = true;
        tracing::info!("XFixes selection monitoring active");

        Ok(())
    }

    /// Check if XFixes monitoring is active
    pub fn xfixes_active(&self) -> bool {
        self.xfixes_active
    }

    /// Get the XFixes event base
    pub fn xfixes_event_base(&self) -> u8 {
        self.selection_mgr.xfixes_event_base()
    }

    /// Check if an event code is an XFixes SelectionNotify
    pub fn is_xfixes_selection_notify(&self, event_code: u8) -> bool {
        self.selection_mgr.is_xfixes_selection_notify(event_code)
    }

    /// Handle XFixes SelectionNotify event
    pub fn handle_xfixes_selection_notify(
        &mut self,
        event: &XFixesSelectionNotifyEvent,
    ) -> Result<Option<u64>> {
        let atoms = *self.selection_mgr.atoms();
        let our_window = self.selection_mgr.window();

        // Ignore if we're the new owner
        if event.owner == our_window {
            return Ok(None);
        }

        tracing::debug!(
            "XFixes SelectionNotify: selection={}, owner={}, subtype={:?}",
            event.selection,
            event.owner,
            event.subtype
        );

        // Check which selection changed
        let is_clipboard = event.selection == atoms.clipboard;
        let is_primary = event.selection == atoms.primary && self.watch_primary;

        if !is_clipboard && !is_primary {
            return Ok(None);
        }

        // Owner released or window destroyed - claim ownership to preserve content
        // subtype: 0 = SetSelectionOwner, 1 = SelectionWindowDestroy, 2 = SelectionClientClose
        if event.owner == 0 {
            // Selection was cleared, try to claim it with our stored content
            if is_clipboard && self.current_clipboard.is_some() {
                tracing::debug!("Clipboard owner released, claiming ownership");
                self.selection_mgr.claim_ownership(atoms.clipboard)?;
            } else if is_primary && self.current_primary.is_some() {
                tracing::debug!("Primary owner released, claiming ownership");
                self.selection_mgr.claim_ownership(atoms.primary)?;
            }
            return Ok(None);
        }

        // New owner - request content
        let transfer = TransferManager::new(&self.selection_mgr);
        let content = transfer.request_content(event.selection)?;

        if let Some(content) = content {
            // Apply content filter
            if self.filter.should_filter(&content, None) {
                return Ok(None);
            }

            tracing::debug!(
                "Captured {} via XFixes: {}",
                if is_clipboard { "clipboard" } else { "primary" },
                content.preview(50)
            );

            // Store in history
            let id = self.history.push(content.clone(), None);

            // Store as current content
            if is_clipboard {
                self.current_clipboard = Some(content);
                self.last_clipboard_owner = event.owner;
            } else {
                self.current_primary = Some(content);
                self.last_primary_owner = event.owner;
            }

            return Ok(id);
        }

        Ok(None)
    }

    /// Get the X11 connection
    pub fn conn(&self) -> &Arc<RustConnection> {
        self.selection_mgr.conn()
    }

    /// Get our window ID
    pub fn window(&self) -> Window {
        self.selection_mgr.window()
    }

    /// Get the atoms
    pub fn atoms(&self) -> &Atoms {
        self.selection_mgr.atoms()
    }

    /// Get the history
    pub fn history(&self) -> &ClipboardHistory {
        &self.history
    }

    /// Get mutable history
    pub fn history_mut(&mut self) -> &mut ClipboardHistory {
        &mut self.history
    }

    /// Check for clipboard changes and capture new content (polling fallback)
    pub fn poll_clipboard(&mut self) -> Result<Option<u64>> {
        // Skip polling if XFixes is active
        if self.xfixes_active {
            return Ok(None);
        }

        let atoms = *self.selection_mgr.atoms();
        let current_owner = self.selection_mgr.get_owner(atoms.clipboard)?;

        // Skip if we're the owner or if owner hasn't changed
        if current_owner == self.selection_mgr.window()
            || current_owner == self.last_clipboard_owner
        {
            return Ok(None);
        }

        self.last_clipboard_owner = current_owner;

        // No owner - clipboard was cleared
        if current_owner == 0 {
            return Ok(None);
        }

        // Request content from new owner
        let transfer = TransferManager::new(&self.selection_mgr);
        if let Some(content) = transfer.request_content(atoms.clipboard)? {
            // Apply content filter
            if self.filter.should_filter(&content, None) {
                return Ok(None);
            }

            tracing::debug!("Captured clipboard: {}", content.preview(50));

            // Store in history
            let id = self.history.push(content.clone(), None);

            // Store as current content
            self.current_clipboard = Some(content);

            return Ok(id);
        }

        Ok(None)
    }

    /// Check for PRIMARY selection changes (polling fallback)
    pub fn poll_primary(&mut self) -> Result<Option<u64>> {
        // Skip polling if XFixes is active
        if self.xfixes_active {
            return Ok(None);
        }

        if !self.watch_primary {
            return Ok(None);
        }

        let atoms = *self.selection_mgr.atoms();
        let current_owner = self.selection_mgr.get_owner(atoms.primary)?;

        if current_owner == self.selection_mgr.window()
            || current_owner == self.last_primary_owner
        {
            return Ok(None);
        }

        self.last_primary_owner = current_owner;

        if current_owner == 0 {
            return Ok(None);
        }

        // Request content
        let transfer = TransferManager::new(&self.selection_mgr);
        if let Some(content) = transfer.request_content(atoms.primary)? {
            // Apply content filter
            if self.filter.should_filter(&content, None) {
                return Ok(None);
            }

            tracing::debug!("Captured primary: {}", content.preview(50));

            // Store in history (PRIMARY shares history with CLIPBOARD)
            let id = self.history.push(content.clone(), None);

            // Store as current primary content
            self.current_primary = Some(content);

            return Ok(id);
        }

        Ok(None)
    }

    /// Take ownership of the clipboard with given content
    pub fn set_clipboard(&mut self, content: ClipboardContent) -> Result<()> {
        let atoms = *self.selection_mgr.atoms();

        self.current_clipboard = Some(content.clone());
        self.selection_mgr.claim_ownership(atoms.clipboard)?;

        // Also add to history
        self.history.push(content, None);

        Ok(())
    }

    /// Take ownership of the clipboard when the original owner closes
    /// This preserves the clipboard content
    pub fn claim_clipboard(&mut self) -> Result<()> {
        if self.current_clipboard.is_some() {
            let atoms = *self.selection_mgr.atoms();
            self.selection_mgr.claim_ownership(atoms.clipboard)?;
        }
        Ok(())
    }

    /// Set clipboard content from history entry
    pub fn select_entry(&mut self, id: u64) -> Result<bool> {
        if let Some(entry) = self.history.get(id) {
            let content = entry.content.clone();
            self.set_clipboard(content)?;
            self.history.select(id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Handle a SelectionRequest event
    pub fn handle_selection_request(&self, event: &SelectionRequestEvent) -> Result<()> {
        let atoms = *self.selection_mgr.atoms();

        let content = if event.selection == atoms.clipboard {
            self.current_clipboard.as_ref()
        } else if event.selection == atoms.primary {
            self.current_primary.as_ref()
        } else {
            None
        };

        if let Some(content) = content {
            let transfer = TransferManager::new(&self.selection_mgr);
            transfer.handle_selection_request(event, content)?;
        }

        Ok(())
    }

    /// Handle SelectionClear event
    pub fn handle_selection_clear(&mut self, selection: Atom) {
        self.selection_mgr.handle_selection_clear(selection);
    }

    /// Get current clipboard content
    pub fn current_clipboard(&self) -> Option<&ClipboardContent> {
        self.current_clipboard.as_ref()
    }

    /// Get current primary content
    pub fn current_primary(&self) -> Option<&ClipboardContent> {
        self.current_primary.as_ref()
    }

    /// Check if we own the clipboard
    pub fn owns_clipboard(&self) -> Result<bool> {
        let atoms = *self.selection_mgr.atoms();
        self.selection_mgr.is_owner(atoms.clipboard)
    }

    /// Clear the clipboard
    pub fn clear_clipboard(&mut self) -> Result<()> {
        self.current_clipboard = None;
        let atoms = *self.selection_mgr.atoms();
        self.selection_mgr.release_ownership(atoms.clipboard)?;
        Ok(())
    }

    /// Save history to disk
    pub fn save_history(&self, path: &std::path::Path) -> Result<()> {
        self.history.save(path)
    }
}
