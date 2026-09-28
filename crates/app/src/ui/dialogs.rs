//! Small modal helpers: folder picker, confirm, error.

use adw::prelude::*;


/// Ask for a folder with the portal/GTK file dialog. `done(None)` on cancel.
pub fn pick_folder(parent: &impl IsA<gtk::Window>, title: &str, done: impl FnOnce(Option<String>) + 'static) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    dialog.select_folder(Some(parent), gtk::gio::Cancellable::NONE, move |res| {
        done(res.ok().and_then(|f| f.path()).map(|p| p.to_string_lossy().into_owned()))
    });
}

/// Ask for an image file. `done(None)` on cancel.
pub fn pick_image(parent: &impl IsA<gtk::Window>, title: &str, done: impl FnOnce(Option<std::path::PathBuf>) + 'static) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    filter.add_mime_type("image/*");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder().title(title).modal(true).filters(&filters).default_filter(&filter).build();
    dialog.open(Some(parent), gtk::gio::Cancellable::NONE, move |res| done(res.ok().and_then(|f| f.path())));
}

/// A yes/no question. `on_yes` runs only on the confirming response.
pub fn confirm(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    yes_label: &str,
    destructive: bool,
    on_yes: impl FnOnce() + 'static,
) {
    let dialog = adw::AlertDialog::builder().heading(heading).body(body).build();
    dialog.add_responses(&[("cancel", "Cancel"), ("yes", yes_label)]);
    dialog.set_response_appearance(
        "yes",
        if destructive { adw::ResponseAppearance::Destructive } else { adw::ResponseAppearance::Suggested },
    );
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let on_yes = std::cell::Cell::new(Some(on_yes));
    dialog.connect_response(None, move |_, r| {
        if r == "yes" {
            if let Some(f) = on_yes.take() {
                f();
            }
        }
    });
    dialog.present(Some(parent));
}

/// An error the user must acknowledge.
pub fn error(parent: &impl IsA<gtk::Widget>, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::builder().heading(heading).body(body).build();
    dialog.add_responses(&[("ok", "OK")]);
    dialog.set_default_response(Some("ok"));
    dialog.present(Some(parent));
}
