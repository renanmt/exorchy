//! `GameObject`: a `Game` row as a GObject, so `gio::ListStore` and
//! `GridView` can hold it. The row is replaced whole on refresh; widgets
//! re-read it on bind.

use std::cell::RefCell;

use exorchy_core::models::Game;
use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct GameObject {
        pub game: RefCell<Option<Game>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GameObject {
        const NAME: &'static str = "ExorchyGameObject";
        type Type = super::GameObject;
    }

    impl ObjectImpl for GameObject {}
}

glib::wrapper! {
    pub struct GameObject(ObjectSubclass<imp::GameObject>);
}

impl GameObject {
    pub fn new(game: Game) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().game.replace(Some(game));
        obj
    }

    pub fn game(&self) -> Game {
        self.imp().game.borrow().clone().expect("GameObject always carries a game")
    }

    pub fn id(&self) -> Option<i64> {
        self.imp().game.borrow().as_ref().and_then(|g| g.id)
    }

    pub fn set_game(&self, game: Game) {
        self.imp().game.replace(Some(game));
    }
}
