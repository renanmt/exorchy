//! The Reading Room (Media Pack magazines, books, catalogues). Stub.

use gtk::prelude::*;

pub fn build(_window: &gtk::Window) -> gtk::Widget {
    gtk::Label::builder().label("The Reading Room is being ported.").css_classes(["muted"]).vexpand(true).build().upcast()
}
