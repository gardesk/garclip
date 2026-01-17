use std::collections::HashSet;
use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, Screen, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt;
use x11rb::CURRENT_TIME;

use crate::error::Result;
use crate::x11::Atoms;

/// Manages X11 selection ownership for clipboard operations
pub struct SelectionManager {
    conn: Arc<RustConnection>,
    screen: Screen,
    window: Window,
    atoms: Atoms,
    owned_selections: HashSet<Atom>,
}

impl SelectionManager {
    /// Create a new selection manager
    pub fn new(conn: Arc<RustConnection>, screen_num: usize, atoms: Atoms) -> Result<Self> {
        let screen = conn.setup().roots[screen_num].clone();

        // Create a hidden window for selection ownership
        let window = conn.generate_id()?;
        conn.create_window(
            screen.root_depth,
            window,
            screen.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
        )?;

        // Set window name for debugging
        conn.change_property8(
            PropMode::REPLACE,
            window,
            Atom::from(x11rb::protocol::xproto::AtomEnum::WM_NAME),
            Atom::from(x11rb::protocol::xproto::AtomEnum::STRING),
            b"garclip",
        )?;

        conn.flush()?;

        Ok(Self {
            conn,
            screen,
            window,
            atoms,
            owned_selections: HashSet::new(),
        })
    }

    /// Get the connection
    pub fn conn(&self) -> &Arc<RustConnection> {
        &self.conn
    }

    /// Get our window ID
    pub fn window(&self) -> Window {
        self.window
    }

    /// Get the atoms
    pub fn atoms(&self) -> &Atoms {
        &self.atoms
    }

    /// Get the root window
    pub fn root(&self) -> Window {
        self.screen.root
    }

    /// Claim ownership of a selection
    pub fn claim_ownership(&mut self, selection: Atom) -> Result<()> {
        self.conn
            .set_selection_owner(self.window, selection, CURRENT_TIME)?;
        self.conn.flush()?;

        // Verify we got ownership
        let owner = self.conn.get_selection_owner(selection)?.reply()?.owner;
        if owner == self.window {
            self.owned_selections.insert(selection);
            tracing::debug!(
                "Claimed ownership of selection {}",
                self.atoms.name(&*self.conn, selection).unwrap_or_default()
            );
        } else {
            tracing::warn!(
                "Failed to claim ownership of selection {}",
                self.atoms.name(&*self.conn, selection).unwrap_or_default()
            );
        }

        Ok(())
    }

    /// Release ownership of a selection
    pub fn release_ownership(&mut self, selection: Atom) -> Result<()> {
        // Only release if we own it
        if self.owned_selections.contains(&selection) {
            self.conn
                .set_selection_owner(x11rb::NONE, selection, CURRENT_TIME)?;
            self.conn.flush()?;
            self.owned_selections.remove(&selection);
            tracing::debug!(
                "Released ownership of selection {}",
                self.atoms.name(&*self.conn, selection).unwrap_or_default()
            );
        }
        Ok(())
    }

    /// Check if we own a selection
    pub fn is_owner(&self, selection: Atom) -> Result<bool> {
        let owner = self.conn.get_selection_owner(selection)?.reply()?.owner;
        Ok(owner == self.window)
    }

    /// Get the current owner of a selection
    pub fn get_owner(&self, selection: Atom) -> Result<Window> {
        Ok(self.conn.get_selection_owner(selection)?.reply()?.owner)
    }

    /// Handle SelectionClear event (we lost ownership)
    pub fn handle_selection_clear(&mut self, selection: Atom) {
        self.owned_selections.remove(&selection);
        tracing::debug!(
            "Lost ownership of selection {}",
            self.atoms.name(&*self.conn, selection).unwrap_or_default()
        );
    }

    /// Check which selections we currently own
    pub fn owned_selections(&self) -> &HashSet<Atom> {
        &self.owned_selections
    }
}

impl Drop for SelectionManager {
    fn drop(&mut self) {
        // Release all owned selections
        for &selection in self.owned_selections.clone().iter() {
            let _ = self.release_ownership(selection);
        }

        // Destroy our window
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}
