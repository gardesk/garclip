use std::sync::Arc;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, ConnectionExt as _, EventMask, GetPropertyType, PropMode, SelectionNotifyEvent,
    SelectionRequestEvent, Window, SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt;
use x11rb::CURRENT_TIME;

use crate::clipboard::ClipboardContent;
use crate::error::{Error, Result};
use crate::x11::{Atoms, SelectionManager};

/// Timeout for selection operations
const SELECTION_TIMEOUT: Duration = Duration::from_secs(3);

/// Maximum property size to read at once (1MB)
const MAX_PROPERTY_SIZE: u32 = 1024 * 1024;

/// Manages data transfer for selection operations
pub struct TransferManager<'a> {
    selection_mgr: &'a SelectionManager,
}

impl<'a> TransferManager<'a> {
    /// Create a new transfer manager
    pub fn new(selection_mgr: &'a SelectionManager) -> Self {
        Self { selection_mgr }
    }

    fn conn(&self) -> &Arc<RustConnection> {
        self.selection_mgr.conn()
    }

    fn window(&self) -> Window {
        self.selection_mgr.window()
    }

    fn atoms(&self) -> &Atoms {
        self.selection_mgr.atoms()
    }

    /// Request the list of available targets from the selection owner
    pub fn request_targets(&self, selection: Atom) -> Result<Vec<Atom>> {
        let atoms = self.atoms();

        // Request TARGETS conversion
        self.conn().convert_selection(
            self.window(),
            selection,
            atoms.targets,
            atoms.garclip_targets,
            CURRENT_TIME,
        )?;
        self.conn().flush()?;

        // Wait for SelectionNotify
        let event = self.wait_for_selection_notify(selection, atoms.targets)?;

        if event.property == x11rb::NONE {
            return Ok(Vec::new());
        }

        // Read the property
        let reply = self
            .conn()
            .get_property(
                true, // delete
                self.window(),
                atoms.garclip_targets,
                GetPropertyType::ANY,
                0,
                MAX_PROPERTY_SIZE / 4,
            )?
            .reply()?;

        // Parse atom list (32-bit values)
        let targets: Vec<Atom> = reply
            .value32()
            .map(|iter| iter.collect())
            .unwrap_or_default();

        Ok(targets)
    }

    /// Request text content from the selection owner
    pub fn request_text(&self, selection: Atom) -> Result<Option<String>> {
        let atoms = self.atoms();

        // First get available targets
        let targets = self.request_targets(selection)?;
        if targets.is_empty() {
            return Ok(None);
        }

        // Find the best text target
        let target = match atoms.preferred_text_target(&targets) {
            Some(t) => t,
            None => return Ok(None),
        };

        // Request conversion
        self.conn().convert_selection(
            self.window(),
            selection,
            target,
            atoms.garclip_data,
            CURRENT_TIME,
        )?;
        self.conn().flush()?;

        // Wait for response
        let event = self.wait_for_selection_notify(selection, target)?;

        if event.property == x11rb::NONE {
            return Err(Error::SelectionConversionFailed);
        }

        // Read the data
        let text = self.read_property_string(atoms.garclip_data)?;
        Ok(Some(text))
    }

    /// Request image content from the selection owner
    pub fn request_image(&self, selection: Atom) -> Result<Option<(Vec<u8>, String)>> {
        let atoms = self.atoms();

        // First get available targets
        let targets = self.request_targets(selection)?;
        if targets.is_empty() {
            return Ok(None);
        }

        // Find the best image target
        let target = match atoms.preferred_image_target(&targets) {
            Some(t) => t,
            None => return Ok(None),
        };

        // Request conversion
        self.conn().convert_selection(
            self.window(),
            selection,
            target,
            atoms.garclip_data,
            CURRENT_TIME,
        )?;
        self.conn().flush()?;

        // Wait for response
        let event = self.wait_for_selection_notify(selection, target)?;

        if event.property == x11rb::NONE {
            return Err(Error::SelectionConversionFailed);
        }

        // Determine MIME type
        let mime_type = self.atom_to_mime(target);

        // Read the data
        let data = self.read_property_bytes(atoms.garclip_data)?;
        Ok(Some((data, mime_type)))
    }

    /// Request clipboard content (files, images, or text)
    pub fn request_content(&self, selection: Atom) -> Result<Option<ClipboardContent>> {
        let atoms = self.atoms();

        // Get available targets
        let targets = self.request_targets(selection)?;
        if targets.is_empty() {
            return Ok(None);
        }

        // Prefer files first (most specialized content type)
        if let Some(file_target) = atoms.preferred_file_target(&targets) {
            if let Some((uris, is_cut)) = self.request_files_target(selection, file_target)? {
                return Ok(Some(ClipboardContent::Files { uris, is_cut }));
            }
        }

        // Then images (images often have text alternatives)
        if let Some(img_target) = atoms.preferred_image_target(&targets) {
            if let Some((data, mime)) = self.request_image_target(selection, img_target)? {
                return Ok(Some(ClipboardContent::Image { data, mime_type: mime }));
            }
        }

        // Fall back to text
        if let Some(text_target) = atoms.preferred_text_target(&targets) {
            if let Some(text) = self.request_text_target(selection, text_target)? {
                return Ok(Some(ClipboardContent::Text(text)));
            }
        }

        Ok(None)
    }

    /// Request text with a specific target
    fn request_text_target(&self, selection: Atom, target: Atom) -> Result<Option<String>> {
        let atoms = self.atoms();

        self.conn().convert_selection(
            self.window(),
            selection,
            target,
            atoms.garclip_data,
            CURRENT_TIME,
        )?;
        self.conn().flush()?;

        let event = self.wait_for_selection_notify(selection, target)?;
        if event.property == x11rb::NONE {
            return Ok(None);
        }

        let text = self.read_property_string(atoms.garclip_data)?;
        Ok(Some(text))
    }

    /// Request image with a specific target
    fn request_image_target(
        &self,
        selection: Atom,
        target: Atom,
    ) -> Result<Option<(Vec<u8>, String)>> {
        let atoms = self.atoms();

        self.conn().convert_selection(
            self.window(),
            selection,
            target,
            atoms.garclip_data,
            CURRENT_TIME,
        )?;
        self.conn().flush()?;

        let event = self.wait_for_selection_notify(selection, target)?;
        if event.property == x11rb::NONE {
            return Ok(None);
        }

        let data = self.read_property_bytes(atoms.garclip_data)?;
        let mime_type = self.atom_to_mime(target);
        Ok(Some((data, mime_type)))
    }

    /// Request file URIs from clipboard
    fn request_files_target(
        &self,
        selection: Atom,
        target: Atom,
    ) -> Result<Option<(Vec<String>, bool)>> {
        let atoms = self.atoms();

        self.conn().convert_selection(
            self.window(),
            selection,
            target,
            atoms.garclip_data,
            CURRENT_TIME,
        )?;
        self.conn().flush()?;

        let event = self.wait_for_selection_notify(selection, target)?;
        if event.property == x11rb::NONE {
            return Ok(None);
        }

        let data = self.read_property_string(atoms.garclip_data)?;

        // Parse based on format
        if target == atoms.gnome_copied_files {
            // Format: "copy\nfile:///path1\nfile:///path2" or "cut\n..."
            let mut lines = data.lines();
            let action = lines.next().unwrap_or("copy");
            let is_cut = action == "cut";
            let uris: Vec<String> = lines.map(|s| s.to_string()).collect();
            if uris.is_empty() {
                return Ok(None);
            }
            Ok(Some((uris, is_cut)))
        } else {
            // text/uri-list format: "file:///path1\r\nfile:///path2\r\n"
            let uris: Vec<String> = data
                .lines()
                .map(|s| s.trim_end_matches('\r').to_string())
                .filter(|s| !s.is_empty() && !s.starts_with('#'))
                .collect();
            if uris.is_empty() {
                return Ok(None);
            }
            // text/uri-list doesn't indicate cut, assume copy
            Ok(Some((uris, false)))
        }
    }

    /// Wait for a SelectionNotify event
    fn wait_for_selection_notify(
        &self,
        selection: Atom,
        target: Atom,
    ) -> Result<SelectionNotifyEvent> {
        let start = Instant::now();

        loop {
            if start.elapsed() > SELECTION_TIMEOUT {
                return Err(Error::SelectionTimeout);
            }

            // Poll for events with a short timeout
            if let Ok(Some(event)) = self.conn().poll_for_event() {
                if let Event::SelectionNotify(notify) = event {
                    if notify.selection == selection && notify.target == target {
                        return Ok(notify);
                    }
                }
            }

            // Small sleep to avoid busy waiting
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Read a property as a string
    fn read_property_string(&self, property: Atom) -> Result<String> {
        let reply = self
            .conn()
            .get_property(
                true, // delete
                self.window(),
                property,
                GetPropertyType::ANY,
                0,
                MAX_PROPERTY_SIZE / 4,
            )?
            .reply()?;

        Ok(String::from_utf8_lossy(&reply.value).into_owned())
    }

    /// Read a property as raw bytes
    fn read_property_bytes(&self, property: Atom) -> Result<Vec<u8>> {
        let reply = self
            .conn()
            .get_property(
                true, // delete
                self.window(),
                property,
                GetPropertyType::ANY,
                0,
                MAX_PROPERTY_SIZE,
            )?
            .reply()?;

        Ok(reply.value)
    }

    /// Convert an atom to a MIME type string
    fn atom_to_mime(&self, atom: Atom) -> String {
        let atoms = self.atoms();
        if atom == atoms.image_png {
            "image/png".to_string()
        } else if atom == atoms.image_jpeg {
            "image/jpeg".to_string()
        } else if atom == atoms.image_gif {
            "image/gif".to_string()
        } else if atom == atoms.image_bmp {
            "image/bmp".to_string()
        } else if atom == atoms.image_webp {
            "image/webp".to_string()
        } else {
            // Try to get the atom name
            self.atoms()
                .name(self.conn().as_ref(), atom)
                .unwrap_or_else(|_| "application/octet-stream".to_string())
        }
    }

    /// Handle an incoming SelectionRequest (serve data to a requestor)
    pub fn handle_selection_request(
        &self,
        event: &SelectionRequestEvent,
        content: &ClipboardContent,
    ) -> Result<()> {
        let atoms = self.atoms();
        let target = event.target;
        let property = if event.property == x11rb::NONE {
            event.target // Old clients
        } else {
            event.property
        };

        let mut success = true;

        if target == atoms.targets {
            // Respond with list of supported targets
            let mut targets = vec![atoms.targets, atoms.timestamp];

            match content {
                ClipboardContent::Text(_) => {
                    targets.extend(atoms.supported_text_targets());
                }
                ClipboardContent::Image { mime_type, .. } => {
                    // Add the specific image type
                    if let Some(atom) = self.mime_to_atom(mime_type) {
                        targets.push(atom);
                    }
                    // Also offer PNG as a common format
                    if !targets.contains(&atoms.image_png) {
                        targets.push(atoms.image_png);
                    }
                }
                ClipboardContent::Files { is_cut, .. } => {
                    targets.extend(atoms.supported_file_targets());
                    // Also offer text formats for compatibility
                    targets.extend(atoms.supported_text_targets());
                    // Add KDE cut indicator if this is a cut operation
                    if *is_cut {
                        targets.push(atoms.kde_cut_selection);
                    }
                }
            }

            self.conn().change_property32(
                PropMode::REPLACE,
                event.requestor,
                property,
                atoms.atom,
                &targets,
            )?;
        } else if target == atoms.timestamp {
            // Respond with timestamp (we use CURRENT_TIME)
            self.conn().change_property32(
                PropMode::REPLACE,
                event.requestor,
                property,
                Atom::from(x11rb::protocol::xproto::AtomEnum::INTEGER),
                &[CURRENT_TIME],
            )?;
        } else if atoms.is_file_target(target) {
            // Respond with file URIs
            match content {
                ClipboardContent::Files { uris, is_cut } => {
                    let data = if target == atoms.gnome_copied_files {
                        // x-special/gnome-copied-files format
                        let action = if *is_cut { "cut" } else { "copy" };
                        std::iter::once(action.to_string())
                            .chain(uris.iter().cloned())
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        // text/uri-list format
                        uris.iter()
                            .map(|uri| format!("{}\r\n", uri))
                            .collect::<String>()
                    };
                    self.conn().change_property8(
                        PropMode::REPLACE,
                        event.requestor,
                        property,
                        target,
                        data.as_bytes(),
                    )?;
                }
                _ => success = false,
            }
        } else if target == atoms.kde_cut_selection {
            // KDE cut selection indicator
            match content {
                ClipboardContent::Files { is_cut, .. } => {
                    let data = if *is_cut { "1" } else { "0" };
                    self.conn().change_property8(
                        PropMode::REPLACE,
                        event.requestor,
                        property,
                        target,
                        data.as_bytes(),
                    )?;
                }
                _ => success = false,
            }
        } else if atoms.is_text_target(target) {
            // Respond with text
            match content {
                ClipboardContent::Text(text) => {
                    self.conn().change_property8(
                        PropMode::REPLACE,
                        event.requestor,
                        property,
                        target,
                        text.as_bytes(),
                    )?;
                }
                ClipboardContent::Files { uris, .. } => {
                    // For text targets, send URIs as newline-separated text
                    let text = uris.join("\n");
                    self.conn().change_property8(
                        PropMode::REPLACE,
                        event.requestor,
                        property,
                        target,
                        text.as_bytes(),
                    )?;
                }
                _ => success = false,
            }
        } else if atoms.is_image_target(target) {
            // Respond with image
            match content {
                ClipboardContent::Image { data, mime_type } => {
                    // Convert if necessary
                    let output_data = self.convert_image_format(data, mime_type, target)?;
                    self.conn().change_property8(
                        PropMode::REPLACE,
                        event.requestor,
                        property,
                        target,
                        &output_data,
                    )?;
                }
                _ => success = false,
            }
        } else {
            success = false;
        }

        // Send SelectionNotify
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: event.time,
            requestor: event.requestor,
            selection: event.selection,
            target: event.target,
            property: if success { property } else { x11rb::NONE },
        };

        self.conn()
            .send_event(false, event.requestor, EventMask::NO_EVENT, notify)?;
        self.conn().flush()?;

        Ok(())
    }

    /// Convert MIME type to atom
    fn mime_to_atom(&self, mime: &str) -> Option<Atom> {
        let atoms = self.atoms();
        match mime {
            "image/png" => Some(atoms.image_png),
            "image/jpeg" => Some(atoms.image_jpeg),
            "image/gif" => Some(atoms.image_gif),
            "image/bmp" => Some(atoms.image_bmp),
            "image/webp" => Some(atoms.image_webp),
            _ => None,
        }
    }

    /// Convert image data to requested format
    fn convert_image_format(
        &self,
        data: &[u8],
        source_mime: &str,
        target: Atom,
    ) -> Result<Vec<u8>> {
        let atoms = self.atoms();

        // Determine target MIME type
        let target_mime = if target == atoms.image_png {
            "image/png"
        } else if target == atoms.image_jpeg {
            "image/jpeg"
        } else if target == atoms.image_gif {
            "image/gif"
        } else if target == atoms.image_webp {
            "image/webp"
        } else if target == atoms.image_bmp {
            "image/bmp"
        } else {
            return Err(Error::UnsupportedTarget(format!("{:?}", target)));
        };

        // If same format, return as-is
        if source_mime == target_mime {
            return Ok(data.to_vec());
        }

        // Load image and convert
        let img = image::load_from_memory(data)?;
        let mut output = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut output);

        match target_mime {
            "image/png" => img.write_to(&mut cursor, image::ImageFormat::Png)?,
            "image/jpeg" => img.write_to(&mut cursor, image::ImageFormat::Jpeg)?,
            "image/gif" => img.write_to(&mut cursor, image::ImageFormat::Gif)?,
            "image/webp" => img.write_to(&mut cursor, image::ImageFormat::WebP)?,
            "image/bmp" => img.write_to(&mut cursor, image::ImageFormat::Bmp)?,
            _ => return Err(Error::UnsupportedTarget(target_mime.to_string())),
        }

        Ok(output)
    }
}
