//! The Settings dialog. Stub: replaced by the settings port.

use gtk::prelude::*;

/// Open Settings on `section` ("general", "collections", "emulators",
/// "appearance", "storage", "network", "packs", "about").
pub fn open(parent: &impl IsA<gtk::Widget>, section: &str) {
    crate::ui::dialogs::error(parent, "Settings", &format!("Settings ({section}) are being ported."));
}
