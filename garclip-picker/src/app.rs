use crate::ui::Popup;
use anyhow::Result;
use garclip::config::Config;
use garclip::ipc::protocol::{Command, EntryInfo};
use gartk_core::{InputEvent, Key};
use gartk_x11::{Connection, EventLoop, EventLoopConfig, Window, WindowConfig};

/// Clipboard entry for display
#[derive(Clone)]
pub struct ClipboardItem {
    pub id: u64,
    pub preview: String,
    pub content_type: String,
    pub source: Option<String>,
    pub pinned: bool,
}

impl From<EntryInfo> for ClipboardItem {
    fn from(info: EntryInfo) -> Self {
        Self {
            id: info.id,
            preview: info.preview,
            content_type: info.content_type,
            source: info.source,
            pinned: info.pinned,
        }
    }
}

/// Application state
pub struct App {
    popup: Popup,
    config: Config,
    input: String,
    cursor: usize,
    all_items: Vec<ClipboardItem>,
    filtered_items: Vec<ClipboardItem>,
    selected: usize,
    scroll_offset: usize,
    max_visible: usize,
    selected_id: Option<u64>,
    should_quit: bool,
}

impl App {
    /// Create a new app
    pub fn new() -> Result<Self> {
        let config = Config::load_default();

        // Fetch clipboard history
        let all_items = Self::fetch_history(&config)?;

        // Connect to X11
        let conn = Connection::connect(None)?;

        // Detect monitor of active window (falls back to pointer position)
        let monitor = gartk_x11::monitor_of_active_window(&conn)?;

        // Calculate popup size and position
        let width = 600;
        let height = 420;
        let x = monitor.rect.x + (monitor.rect.width as i32 - width as i32) / 2;
        let y = monitor.rect.y + (monitor.rect.height as i32 - height as i32) / 3;

        // Create window
        let window = Window::create(
            conn.clone(),
            WindowConfig::popup()
                .title("garclip")
                .class("garclip-picker")
                .position(x, y)
                .size(width, height)
                .transparent(true),
        )?;

        // Focus and grab keyboard
        window.focus()?;
        window.grab_keyboard_with_retry(10, 50)?;

        // Create popup UI
        let popup = Popup::new(window)?;

        let max_visible = 10;

        Ok(Self {
            popup,
            config,
            input: String::new(),
            cursor: 0,
            filtered_items: all_items.clone(),
            all_items,
            selected: 0,
            scroll_offset: 0,
            max_visible,
            selected_id: None,
            should_quit: false,
        })
    }

    /// Fetch history from garclip daemon
    fn fetch_history(config: &Config) -> Result<Vec<ClipboardItem>> {
        let socket_path = config.socket_path();
        let cmd = Command::History { limit: 100 };

        // Use blocking IPC since we're not in async context
        let response = garclip::ipc::send_command_blocking(&socket_path, &cmd)?;

        if response.success {
            if let Some(data) = response.data {
                let entries: Vec<EntryInfo> = serde_json::from_value(data)?;
                return Ok(entries.into_iter().map(ClipboardItem::from).collect());
            }
        }

        Ok(vec![])
    }

    /// Run the application event loop
    pub fn run(&mut self) -> Result<()> {
        let window = self.popup.window();
        let mut event_loop = EventLoop::new(window, EventLoopConfig::default())?;

        // Initial render
        self.render()?;

        event_loop.run(|ev, event| {
            match event {
                InputEvent::Key(key_event) if key_event.pressed => {
                    self.handle_key(&key_event.key);
                    ev.request_redraw();
                }
                InputEvent::Expose => {
                    ev.request_redraw();
                }
                InputEvent::CloseRequested => {
                    self.should_quit = true;
                }
                _ => {}
            }

            if ev.needs_redraw() {
                let _ = self.render();
                ev.redraw_done();
            }

            Ok(!self.should_quit)
        })?;

        // Ungrab keyboard
        self.popup.window().ungrab_keyboard()?;

        // If an item was selected, activate it
        if let Some(id) = self.selected_id {
            self.activate_entry(id)?;
        }

        Ok(())
    }

    /// Handle a key press
    fn handle_key(&mut self, key: &Key) {
        match key {
            Key::Escape => {
                self.should_quit = true;
            }
            Key::Return => {
                self.select_current();
            }
            Key::Up => {
                self.select_prev();
            }
            Key::Down => {
                self.select_next();
            }
            Key::PageUp => {
                for _ in 0..self.max_visible {
                    self.select_prev();
                }
            }
            Key::PageDown => {
                for _ in 0..self.max_visible {
                    self.select_next();
                }
            }
            Key::Home => {
                self.cursor = 0;
            }
            Key::End => {
                self.cursor = self.input.len();
            }
            Key::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            Key::Right => {
                if self.cursor < self.input.len() {
                    self.cursor += 1;
                }
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    self.input.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.filter_items();
                }
            }
            Key::Delete => {
                // Delete selected entry
                if let Some(item) = self.filtered_items.get(self.selected) {
                    let id = item.id;
                    let _ = self.delete_entry(id);
                    // Refresh list
                    if let Ok(items) = Self::fetch_history(&self.config) {
                        self.all_items = items;
                        self.filter_items();
                    }
                }
            }
            Key::Char(c) => {
                self.input.insert(self.cursor, *c);
                self.cursor += 1;
                self.filter_items();
            }
            Key::Space => {
                self.input.insert(self.cursor, ' ');
                self.cursor += 1;
                self.filter_items();
            }
            _ => {}
        }
    }

    /// Filter items based on current input
    fn filter_items(&mut self) {
        if self.input.is_empty() {
            self.filtered_items = self.all_items.clone();
        } else {
            let query = self.input.to_lowercase();
            self.filtered_items = self
                .all_items
                .iter()
                .filter(|item| {
                    item.preview.to_lowercase().contains(&query)
                        || item
                            .source
                            .as_ref()
                            .map(|s| s.to_lowercase().contains(&query))
                            .unwrap_or(false)
                })
                .cloned()
                .collect();
        }

        // Reset selection
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Select the previous item
    fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            }
        }
    }

    /// Select the next item
    fn select_next(&mut self) {
        if self.selected + 1 < self.filtered_items.len() {
            self.selected += 1;
            if self.selected >= self.scroll_offset + self.max_visible {
                self.scroll_offset = self.selected - self.max_visible + 1;
            }
        }
    }

    /// Select the current item
    fn select_current(&mut self) {
        if let Some(item) = self.filtered_items.get(self.selected) {
            self.selected_id = Some(item.id);
            self.should_quit = true;
        }
    }

    /// Activate an entry (select it in garclip)
    fn activate_entry(&self, id: u64) -> Result<()> {
        let socket_path = self.config.socket_path();
        let cmd = Command::Select { id };
        garclip::ipc::send_command_blocking(&socket_path, &cmd)?;
        Ok(())
    }

    /// Delete an entry
    fn delete_entry(&self, id: u64) -> Result<()> {
        let socket_path = self.config.socket_path();
        let cmd = Command::Delete { id };
        garclip::ipc::send_command_blocking(&socket_path, &cmd)?;
        Ok(())
    }

    /// Render the popup
    fn render(&mut self) -> Result<()> {
        let visible_items: Vec<&ClipboardItem> = self
            .filtered_items
            .iter()
            .skip(self.scroll_offset)
            .take(self.max_visible)
            .collect();

        let selected_visible = self.selected.saturating_sub(self.scroll_offset);

        self.popup.render(
            &self.input,
            self.cursor,
            &visible_items,
            selected_visible,
            self.filtered_items.len(),
        )?;

        Ok(())
    }
}
