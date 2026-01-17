use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt};

use crate::error::Result;

/// Cached X11 atoms for clipboard operations
#[derive(Debug, Clone, Copy)]
pub struct Atoms {
    // Selection atoms
    pub clipboard: Atom,
    pub primary: Atom,
    pub secondary: Atom,

    // Standard target atoms
    pub targets: Atom,
    pub multiple: Atom,
    pub timestamp: Atom,
    pub atom: Atom,
    pub incr: Atom,

    // Text targets
    pub utf8_string: Atom,
    pub string: Atom,
    pub text: Atom,
    pub text_plain: Atom,
    pub text_plain_utf8: Atom,

    // Image targets
    pub image_png: Atom,
    pub image_jpeg: Atom,
    pub image_gif: Atom,
    pub image_bmp: Atom,
    pub image_webp: Atom,

    // Clipboard manager atoms
    pub clipboard_manager: Atom,
    pub save_targets: Atom,

    // Our custom atoms
    pub garclip_data: Atom,
    pub garclip_targets: Atom,
}

impl Atoms {
    /// Intern all required atoms
    pub fn intern<C: Connection>(conn: &C) -> Result<Self> {
        // Request all atoms in parallel for efficiency
        let clipboard = conn.intern_atom(false, b"CLIPBOARD")?;
        let primary = conn.intern_atom(false, b"PRIMARY")?;
        let secondary = conn.intern_atom(false, b"SECONDARY")?;

        let targets = conn.intern_atom(false, b"TARGETS")?;
        let multiple = conn.intern_atom(false, b"MULTIPLE")?;
        let timestamp = conn.intern_atom(false, b"TIMESTAMP")?;
        let incr = conn.intern_atom(false, b"INCR")?;

        let utf8_string = conn.intern_atom(false, b"UTF8_STRING")?;
        let text = conn.intern_atom(false, b"TEXT")?;
        let text_plain = conn.intern_atom(false, b"text/plain")?;
        let text_plain_utf8 = conn.intern_atom(false, b"text/plain;charset=utf-8")?;

        let image_png = conn.intern_atom(false, b"image/png")?;
        let image_jpeg = conn.intern_atom(false, b"image/jpeg")?;
        let image_gif = conn.intern_atom(false, b"image/gif")?;
        let image_bmp = conn.intern_atom(false, b"image/bmp")?;
        let image_webp = conn.intern_atom(false, b"image/webp")?;

        let clipboard_manager = conn.intern_atom(false, b"CLIPBOARD_MANAGER")?;
        let save_targets = conn.intern_atom(false, b"SAVE_TARGETS")?;

        let garclip_data = conn.intern_atom(false, b"GARCLIP_DATA")?;
        let garclip_targets = conn.intern_atom(false, b"GARCLIP_TARGETS")?;

        // Wait for all replies
        Ok(Self {
            clipboard: clipboard.reply()?.atom,
            primary: primary.reply()?.atom,
            secondary: secondary.reply()?.atom,

            targets: targets.reply()?.atom,
            multiple: multiple.reply()?.atom,
            timestamp: timestamp.reply()?.atom,
            atom: AtomEnum::ATOM.into(),
            incr: incr.reply()?.atom,

            utf8_string: utf8_string.reply()?.atom,
            string: AtomEnum::STRING.into(),
            text: text.reply()?.atom,
            text_plain: text_plain.reply()?.atom,
            text_plain_utf8: text_plain_utf8.reply()?.atom,

            image_png: image_png.reply()?.atom,
            image_jpeg: image_jpeg.reply()?.atom,
            image_gif: image_gif.reply()?.atom,
            image_bmp: image_bmp.reply()?.atom,
            image_webp: image_webp.reply()?.atom,

            clipboard_manager: clipboard_manager.reply()?.atom,
            save_targets: save_targets.reply()?.atom,

            garclip_data: garclip_data.reply()?.atom,
            garclip_targets: garclip_targets.reply()?.atom,
        })
    }

    /// Get atom name for debugging
    pub fn name<C: Connection>(&self, conn: &C, atom: Atom) -> Result<String> {
        let reply = conn.get_atom_name(atom)?.reply()?;
        Ok(String::from_utf8_lossy(&reply.name).into_owned())
    }

    /// Check if an atom is a text target
    pub fn is_text_target(&self, atom: Atom) -> bool {
        atom == self.utf8_string
            || atom == self.string
            || atom == self.text
            || atom == self.text_plain
            || atom == self.text_plain_utf8
    }

    /// Check if an atom is an image target
    pub fn is_image_target(&self, atom: Atom) -> bool {
        atom == self.image_png
            || atom == self.image_jpeg
            || atom == self.image_gif
            || atom == self.image_bmp
            || atom == self.image_webp
    }

    /// Get the preferred text target from a list of targets
    pub fn preferred_text_target(&self, targets: &[Atom]) -> Option<Atom> {
        // Prefer UTF8_STRING, then text/plain;charset=utf-8, then STRING
        let preference = [
            self.utf8_string,
            self.text_plain_utf8,
            self.text_plain,
            self.text,
            self.string,
        ];

        for preferred in preference {
            if targets.contains(&preferred) {
                return Some(preferred);
            }
        }
        None
    }

    /// Get the preferred image target from a list of targets
    pub fn preferred_image_target(&self, targets: &[Atom]) -> Option<Atom> {
        // Prefer PNG for lossless, then JPEG, then others
        let preference = [
            self.image_png,
            self.image_jpeg,
            self.image_gif,
            self.image_webp,
            self.image_bmp,
        ];

        for preferred in preference {
            if targets.contains(&preferred) {
                return Some(preferred);
            }
        }
        None
    }

    /// Get the list of targets we support for text
    pub fn supported_text_targets(&self) -> Vec<Atom> {
        vec![
            self.utf8_string,
            self.text_plain_utf8,
            self.text_plain,
            self.text,
            self.string,
        ]
    }

    /// Get the list of targets we support for images
    pub fn supported_image_targets(&self) -> Vec<Atom> {
        vec![
            self.image_png,
            self.image_jpeg,
            self.image_gif,
            self.image_webp,
            self.image_bmp,
        ]
    }
}
