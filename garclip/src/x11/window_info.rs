use std::sync::Arc;

use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window};
use x11rb::rust_connection::RustConnection;

/// Get the WM_CLASS property of a window
/// Returns (instance, class) if available
pub fn get_wm_class(conn: &Arc<RustConnection>, window: Window) -> Option<(String, String)> {
    if window == 0 {
        return None;
    }

    let wm_class_atom: Atom = AtomEnum::WM_CLASS.into();
    let string_atom: Atom = AtomEnum::STRING.into();

    let reply = conn
        .get_property(false, window, wm_class_atom, string_atom, 0, 1024)
        .ok()?
        .reply()
        .ok()?;

    if reply.value.is_empty() {
        return None;
    }

    // WM_CLASS contains two null-terminated strings: instance and class
    let value = &reply.value;
    let mut parts = value.split(|&b| b == 0).filter(|s| !s.is_empty());

    let instance = parts
        .next()
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .unwrap_or_default();

    let class = parts
        .next()
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .unwrap_or_default();

    if class.is_empty() && instance.is_empty() {
        None
    } else {
        Some((instance, class))
    }
}

/// Get the window class name (the second part of WM_CLASS)
pub fn get_window_class(conn: &Arc<RustConnection>, window: Window) -> Option<String> {
    get_wm_class(conn, window).map(|(_, class)| class)
}

/// Get the window instance name (the first part of WM_CLASS)
pub fn get_window_instance(conn: &Arc<RustConnection>, window: Window) -> Option<String> {
    get_wm_class(conn, window).map(|(instance, _)| instance)
}
