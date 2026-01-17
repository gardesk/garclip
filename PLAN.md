# garclip Implementation Plan

## Overview

A bespoke X11 clipboard manager for the gardesk suite. Implements the X11 selection protocol directly using x11rb without any third-party clipboard crates. Supports clipboard history, PRIMARY/CLIPBOARD selections, and integrates with the gardesk ecosystem.

---

## X11 Clipboard Protocol (The Bespoke Implementation)

### How X11 Clipboard Works

X11 uses a **selection ownership model** - the clipboard doesn't store data. Instead:

1. When an app copies text, it becomes the **selection owner**
2. When another app wants to paste, it sends a **SelectionRequest** to the owner
3. The owner responds with **SelectionNotify** containing the data
4. Problem: When the owner app closes, clipboard data is lost

### Clipboard Manager's Role

A clipboard manager solves this by:
1. **Monitoring** selection ownership changes
2. **Grabbing** clipboard contents before the owner releases
3. **Becoming** the new owner to persist the data
4. **Responding** to selection requests from other apps

### Key X11 Atoms

```
CLIPBOARD          - Standard copy/paste selection
PRIMARY            - Middle-click paste (highlighted text)
TARGETS            - Query supported data formats
UTF8_STRING        - UTF-8 encoded text
STRING             - Latin-1 text (fallback)
TEXT               - Generic text
ATOM               - List of atoms
TIMESTAMP          - Selection timestamp
MULTIPLE           - Multiple target request
INCR               - Incremental transfer (large data)
CLIPBOARD_MANAGER  - Clipboard manager registration
SAVE_TARGETS       - Request to save clipboard
```

### X11 Event Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                    CLIPBOARD MONITORING                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. App copies text → SetSelectionOwner(CLIPBOARD)              │
│                       ↓                                         │
│  2. garclip receives SelectionClear (lost ownership)            │
│     OR detects new owner via polling                            │
│                       ↓                                         │
│  3. garclip requests content: ConvertSelection(CLIPBOARD)       │
│                       ↓                                         │
│  4. Owner sends SelectionNotify with data                       │
│                       ↓                                         │
│  5. garclip stores in history                                   │
│                       ↓                                         │
│  6. When owner closes → garclip becomes new owner               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                    SERVING PASTE REQUESTS                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. App requests paste: ConvertSelection(CLIPBOARD, target)     │
│                       ↓                                         │
│  2. garclip receives SelectionRequest event                     │
│                       ↓                                         │
│  3. Check requested target (TARGETS, UTF8_STRING, etc.)         │
│                       ↓                                         │
│  4. Set property on requestor window with data                  │
│                       ↓                                         │
│  5. Send SelectionNotify to requestor                           │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## Architecture

### Project Structure

```
garclip/
├── Cargo.toml                    # Workspace definition
├── PLAN.md                       # This file
├── garclip/                      # Main daemon crate
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs               # Entry point, CLI parsing
│       ├── lib.rs                # Library exports
│       ├── error.rs              # Error types (thiserror)
│       │
│       ├── x11/                  # X11 clipboard implementation
│       │   ├── mod.rs
│       │   ├── atoms.rs          # Atom interning and caching
│       │   ├── selection.rs      # Selection ownership management
│       │   └── transfer.rs       # Data transfer (request/response)
│       │
│       ├── clipboard/            # Clipboard logic
│       │   ├── mod.rs
│       │   ├── manager.rs        # Main clipboard manager
│       │   ├── entry.rs          # Clipboard entry type
│       │   └── history.rs        # History storage
│       │
│       ├── daemon/               # Daemon infrastructure
│       │   ├── mod.rs
│       │   └── state.rs          # Daemon state machine
│       │
│       ├── ipc/                  # IPC layer
│       │   ├── mod.rs
│       │   ├── protocol.rs       # Command/Response/Event types
│       │   ├── server.rs         # Unix socket server
│       │   └── client.rs         # Client helpers
│       │
│       └── config/               # Configuration
│           ├── mod.rs
│           └── types.rs          # Config structs
│
└── garclipctl/                   # CLI control tool
    ├── Cargo.toml
    └── src/
        └── main.rs               # CLI commands
```

### Core Types

```rust
// clipboard/entry.rs
pub struct ClipboardEntry {
    pub id: u64,                          // Unique ID
    pub content: ClipboardContent,        // The actual data
    pub source: Option<String>,           // Source application (if known)
    pub timestamp: SystemTime,            // When captured
    pub pinned: bool,                      // Pinned entries persist
}

pub enum ClipboardContent {
    Text(String),                         // UTF-8 text
    // Future: Image, Html, Files, etc.
}

// clipboard/history.rs
pub struct ClipboardHistory {
    entries: VecDeque<ClipboardEntry>,
    max_entries: usize,
    next_id: u64,
}

impl ClipboardHistory {
    pub fn push(&mut self, content: ClipboardContent, source: Option<String>);
    pub fn get(&self, id: u64) -> Option<&ClipboardEntry>;
    pub fn current(&self) -> Option<&ClipboardEntry>;
    pub fn list(&self, limit: usize) -> Vec<&ClipboardEntry>;
    pub fn remove(&mut self, id: u64) -> bool;
    pub fn clear(&mut self);
    pub fn pin(&mut self, id: u64) -> bool;
    pub fn unpin(&mut self, id: u64) -> bool;
}
```

### X11 Selection Manager

```rust
// x11/selection.rs
pub struct SelectionManager {
    conn: Arc<RustConnection>,
    window: Window,                       // Our window for selection ownership
    atoms: Atoms,                         // Cached atoms
    owned_selections: HashSet<Atom>,      // Currently owned selections
}

impl SelectionManager {
    pub fn new(conn: Arc<RustConnection>) -> Result<Self>;

    // Ownership
    pub fn claim_ownership(&mut self, selection: Atom) -> Result<()>;
    pub fn release_ownership(&mut self, selection: Atom) -> Result<()>;
    pub fn is_owner(&self, selection: Atom) -> Result<bool>;

    // Requesting data from current owner
    pub fn request_targets(&self, selection: Atom) -> Result<Vec<Atom>>;
    pub fn request_text(&self, selection: Atom) -> Result<Option<String>>;

    // Handling incoming requests (when we're the owner)
    pub fn handle_selection_request(&self, event: SelectionRequestEvent, data: &[u8]) -> Result<()>;
    pub fn handle_selection_clear(&mut self, event: SelectionClearEvent);
}

// x11/atoms.rs
pub struct Atoms {
    pub clipboard: Atom,
    pub primary: Atom,
    pub targets: Atom,
    pub utf8_string: Atom,
    pub string: Atom,
    pub text: Atom,
    pub atom: Atom,
    pub timestamp: Atom,
    pub multiple: Atom,
    pub incr: Atom,
    pub clipboard_manager: Atom,
    pub save_targets: Atom,
    // Property atoms
    pub garclip_data: Atom,               // Our property for storing data
}

impl Atoms {
    pub fn intern(conn: &RustConnection) -> Result<Self>;
}
```

### IPC Protocol

```rust
// ipc/protocol.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    // Clipboard operations
    Copy { text: String },                 // Programmatic copy
    Paste,                                 // Get current clipboard

    // History operations
    History {
        limit: Option<usize>,              // Max entries to return (default: 50)
        #[serde(default)]
        include_pinned: bool,              // Include pinned entries
    },
    Select { id: u64 },                    // Select entry from history (makes it current)
    Delete { id: u64 },                    // Delete entry from history
    Clear,                                 // Clear clipboard
    ClearHistory { keep_pinned: bool },    // Clear all history

    // Pin management
    Pin { id: u64 },                       // Pin an entry
    Unpin { id: u64 },                     // Unpin an entry
    ListPinned,                            // List pinned entries

    // Search
    Search {
        query: String,
        limit: Option<usize>,
    },

    // Daemon control
    Status,                                // Get daemon status
    Reload,                                // Reload configuration
    Quit,                                  // Shutdown daemon

    // Subscriptions
    Subscribe { events: Vec<String> },     // Subscribe to events
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    ClipboardChanged {
        id: u64,
        preview: String,                   // First ~100 chars
        source: Option<String>,
    },
    HistoryCleared,
    EntryPinned { id: u64 },
    EntryUnpinned { id: u64 },
    EntryDeleted { id: u64 },
}
```

### Configuration

```toml
# ~/.config/garclip/config.toml

[history]
max_entries = 1000                # Maximum history entries
persist = true                    # Persist history across restarts
persist_path = ""                 # Custom path (default: ~/.local/share/garclip/history.json)

[behavior]
watch_primary = true              # Also track PRIMARY selection (middle-click)
watch_clipboard = true            # Track CLIPBOARD selection (Ctrl+C)
deduplicate = true                # Don't add duplicate entries
ignore_empty = true               # Ignore empty clipboard
min_length = 1                    # Minimum text length to record
max_length = 1048576              # Maximum text length (1MB)

[filters]
# Regex patterns to ignore
ignore_patterns = [
    "^\\s*$",                     # Whitespace only
]
# Window classes to ignore
ignore_classes = [
    "keepassxc",                  # Password managers
    "1password",
]

[daemon]
socket_path = ""                  # Custom socket path (default: $XDG_RUNTIME_DIR/garclip.sock)
log_level = "info"                # Logging level
```

---

## Implementation Phases

### Phase 1: Foundation (Core X11 + Basic Daemon)

**Goal**: Get the daemon running and monitoring clipboard

1. **Project scaffolding**
   - Create workspace structure
   - Set up Cargo.toml with dependencies
   - Basic error types

2. **X11 atom management** (`x11/atoms.rs`)
   - Intern all required atoms
   - Atom caching

3. **Selection ownership** (`x11/selection.rs`)
   - Create hidden window for selection
   - Claim/release ownership
   - Check current owner

4. **Basic clipboard read** (`x11/transfer.rs`)
   - Request TARGETS from owner
   - Request UTF8_STRING/STRING data
   - Handle SelectionNotify response

5. **Daemon skeleton** (`daemon/state.rs`)
   - X11 event loop
   - Detect clipboard changes
   - Log clipboard content

**Deliverable**: Daemon that prints clipboard changes to stdout

### Phase 2: Clipboard Manager (Persistence + History)

**Goal**: Persist clipboard data and maintain history

1. **Clipboard entry types** (`clipboard/entry.rs`)
   - ClipboardEntry struct
   - ClipboardContent enum

2. **History storage** (`clipboard/history.rs`)
   - In-memory VecDeque
   - Push/get/list/remove operations
   - Deduplication

3. **Selection serving** (`x11/transfer.rs`)
   - Handle SelectionRequest events
   - Respond with stored data
   - TARGETS response

4. **Become clipboard owner**
   - Take ownership when original owner releases
   - Serve clipboard content to requestors

5. **Persistence** (`clipboard/history.rs`)
   - Save history to JSON file
   - Load on startup

**Deliverable**: Daemon that persists clipboard across app closes

### Phase 3: IPC Layer

**Goal**: Control daemon via Unix socket

1. **Protocol types** (`ipc/protocol.rs`)
   - Command, Response, Event enums
   - Serde serialization

2. **IPC server** (`ipc/server.rs`)
   - Unix socket listener
   - Accept connections
   - JSON message handling

3. **Command handlers** (`daemon/state.rs`)
   - Implement all commands
   - Wire to clipboard manager

4. **Event subscriptions**
   - Track subscribed clients
   - Broadcast events

**Deliverable**: Daemon controllable via JSON over socket

### Phase 4: CLI Tool (garclipctl)

**Goal**: User-friendly CLI interface

1. **CLI structure**
   - clap argument parsing
   - Subcommands for each operation

2. **Commands**
   - `garclipctl paste` - Print current clipboard
   - `garclipctl copy <text>` - Copy text
   - `garclipctl history` - List history
   - `garclipctl select <id>` - Select from history
   - `garclipctl clear` - Clear clipboard
   - `garclipctl search <query>` - Search history
   - `garclipctl status` - Show daemon status

3. **Output formatting**
   - Human-readable default
   - `--json` flag for scripting

**Deliverable**: Full CLI control tool

### Phase 5: Configuration + Polish

**Goal**: Configurable and robust

1. **Configuration loading**
   - Parse TOML config
   - Defaults for all values
   - Config reload via IPC

2. **Filtering**
   - Ignore patterns
   - Ignore window classes

3. **PRIMARY selection**
   - Monitor PRIMARY in addition to CLIPBOARD
   - Configurable

4. **Signal handling**
   - SIGTERM graceful shutdown
   - SIGHUP config reload

5. **Logging**
   - tracing integration
   - Log file support

**Deliverable**: Production-ready clipboard manager

---

## Dependencies

```toml
[dependencies]
# X11
x11rb = { version = "0.13", features = ["allow-unsafe-code"] }

# Async runtime
tokio = { version = "1", features = ["full", "signal"] }

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.8"

# Error handling
thiserror = "2.0"
anyhow = "1.0"

# CLI
clap = { version = "4.5", features = ["derive"] }

# Utilities
dirs = "6.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
regex = "1.10"      # For ignore patterns
```

---

## Key Implementation Details

### Detecting Clipboard Changes

Two approaches:

1. **Polling** (simpler, works reliably)
   ```rust
   loop {
       let owner = conn.get_selection_owner(atoms.clipboard)?.reply()?.owner;
       if owner != last_owner && owner != our_window {
           // New owner - request content
           request_clipboard_content()?;
           last_owner = owner;
       }
       tokio::time::sleep(Duration::from_millis(100)).await;
   }
   ```

2. **XFixes extension** (more efficient, event-driven)
   ```rust
   // Request SelectionNotify events
   xfixes::select_selection_input(
       &conn,
       our_window,
       atoms.clipboard,
       xfixes::SelectionEventMask::SET_SELECTION_OWNER,
   )?;
   ```

**Recommendation**: Start with polling (simpler), add XFixes later if needed.

### Requesting Clipboard Content

```rust
async fn request_text(&self, selection: Atom) -> Result<Option<String>> {
    // 1. Request conversion to our property
    self.conn.convert_selection(
        self.window,
        selection,
        self.atoms.utf8_string,
        self.atoms.garclip_data,
        x11rb::CURRENT_TIME,
    )?;
    self.conn.flush()?;

    // 2. Wait for SelectionNotify event
    let event = self.wait_for_selection_notify(selection).await?;

    if event.property == x11rb::NONE {
        return Ok(None); // Conversion failed
    }

    // 3. Read property data
    let reply = self.conn.get_property(
        true,  // Delete after reading
        self.window,
        self.atoms.garclip_data,
        self.atoms.utf8_string,
        0,
        u32::MAX / 4,
    )?.reply()?;

    Ok(Some(String::from_utf8_lossy(&reply.value).into_owned()))
}
```

### Serving Clipboard Requests

```rust
fn handle_selection_request(&self, event: SelectionRequestEvent, data: &str) -> Result<()> {
    let target = event.target;
    let property = if event.property == x11rb::NONE {
        event.target  // Old clients
    } else {
        event.property
    };

    if target == self.atoms.targets {
        // Respond with supported targets
        let targets = [
            self.atoms.targets,
            self.atoms.utf8_string,
            self.atoms.string,
            self.atoms.timestamp,
        ];
        self.conn.change_property32(
            PropMode::REPLACE,
            event.requestor,
            property,
            self.atoms.atom,
            &targets.iter().map(|a| a.resource_id()).collect::<Vec<_>>(),
        )?;
    } else if target == self.atoms.utf8_string || target == self.atoms.string {
        // Respond with text
        self.conn.change_property8(
            PropMode::REPLACE,
            event.requestor,
            property,
            target,
            data.as_bytes(),
        )?;
    } else {
        // Unsupported target - send failure
        property = x11rb::NONE;
    }

    // Send SelectionNotify
    let notify = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: event.time,
        requestor: event.requestor,
        selection: event.selection,
        target: event.target,
        property,
    };
    self.conn.send_event(false, event.requestor, EventMask::NO_EVENT, notify)?;
    self.conn.flush()?;

    Ok(())
}
```

---

## Integration Points

### With gar (Window Manager)

- Could query gar IPC for focused window class (for source tracking)
- Future: Workspace-aware clipboard history

### With garlaunch

- garlaunch could have a clipboard history picker mode
- Or garclip could have its own popup (using gartk)

---

## Testing Strategy

1. **Unit tests**
   - History operations
   - Config parsing
   - IPC protocol serialization

2. **Integration tests (Xephyr)**
   ```bash
   Xephyr -br -ac -noreset -screen 1280x720 :1 &
   DISPLAY=:1 cargo run --bin garclip -- daemon
   DISPLAY=:1 garclipctl status
   ```

3. **Manual testing**
   - Copy in various apps
   - Close source app, verify paste works
   - History persistence across daemon restart

---

## Progress & TODO

### Phase 1: Foundation (COMPLETE)
- [x] Project scaffolding (workspace, Cargo.toml)
- [x] Error types
- [x] X11 atom interning
- [x] Selection ownership management
- [x] Data transfer (request/response)
- [x] Clipboard entry types (text + images)
- [x] History storage with persistence
- [x] Clipboard manager
- [x] Configuration (TOML)
- [x] IPC protocol (JSON over Unix socket)
- [x] IPC server/client
- [x] Daemon state machine
- [x] CLI entry point
- [x] garclipctl control tool

### Phase 2: Daemon Command Handler (COMPLETE)
- [x] Refactor client handler to use channels for daemon state access
- [x] Proper async message passing between IPC and daemon loop
- [x] Handle all commands through the daemon via CommandRequest + oneshot
- [x] Handle Quit command for graceful shutdown
- [x] Handle Subscribe for event streaming to clients

### Phase 3: Selection Monitoring (COMPLETE)
- [x] XFixes extension for event-driven clipboard monitoring
- [x] Claim ownership when original owner releases
- [x] Reduce polling overhead

### Phase 4: Filtering
- [ ] Regex patterns to ignore content
- [ ] Window class filtering (ignore password managers, etc.)
- [ ] Configurable min/max content length enforcement

### Phase 5: Signals & Lifecycle
- [ ] SIGHUP for config reload
- [ ] Graceful SIGTERM handling
- [ ] PID file for single-instance enforcement

### Phase 6: Integration
- [ ] Query gar IPC for focused window class (source tracking)
- [ ] Optional garlaunch integration for history picker UI
- [ ] Systemd user service file

---

## Open Questions

1. **Encryption** - Should sensitive clipboard data be encrypted at rest?
   - Recommendation: Optional, not in initial implementation

2. **Popup UI** - Should garclip have its own history picker UI, or rely on garlaunch?
   - Recommendation: Defer UI to later, CLI-first approach
