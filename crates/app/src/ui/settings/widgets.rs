//! The pieces every settings page is made of: the page context, the
//! three-column setting row (label · value/hint · action), switch binding
//! with rollback, small buttons and the mini progress bar.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

type Follower = Box<dyn Fn(bool) -> bool>;

thread_local! {
    /// Whether the Settings page is narrow, and the widgets that follow it (each
    /// follower returns false once its widget is gone).
    static NARROW: Cell<bool> = const { Cell::new(false) };
    /// Which page the state belongs to: a closing page's breakpoint may
    /// unapply after the next one opened and must not reset it.
    static VIEW: Cell<u64> = const { Cell::new(0) };
    static FOLLOWERS: RefCell<Vec<Follower>> = RefCell::new(Vec::new());
}

/// A new page starts wide with no followers; the id it gets back is what
/// its breakpoint passes to `set_narrow`.
pub fn begin_view() -> u64 {
    FOLLOWERS.with(|f| f.borrow_mut().clear());
    NARROW.with(|n| n.set(false));
    VIEW.with(|d| {
        d.set(d.get() + 1);
        d.get()
    })
}

/// A narrow page stacks every row (label over body over action) instead
/// of squeezing the body to a few characters beside a 170 px label column.
/// The page's breakpoint calls this; widgets built later follow the state.
pub fn set_narrow(view: u64, narrow: bool) {
    if VIEW.with(Cell::get) != view {
        return;
    }
    NARROW.with(|n| n.set(narrow));
    FOLLOWERS.with(|f| f.borrow_mut().retain(|follow| follow(narrow)));
}

/// Run `f` with the narrow state now and on every change while `widget` lives.
pub fn follow_narrow<W: IsA<gtk::Widget>>(widget: &W, f: impl Fn(&W, bool) + 'static) {
    f(widget, NARROW.with(Cell::get));
    let weak = widget.downgrade();
    FOLLOWERS.with(|l| {
        l.borrow_mut().push(Box::new(move |narrow| match weak.upgrade() {
            Some(w) => {
                f(&w, narrow);
                true
            }
            None => false,
        }))
    });
}

/// Row refreshers a page calls when the pack store changes.
pub type Refreshers = Rc<RefCell<Vec<Rc<dyn Fn()>>>>;

/// What a page needs from Settings: the window (for pickers and confirm
/// dialogs), whether Settings is still open (timers stop on close), and
/// navigation between sections.
#[derive(Clone)]
pub struct Ctx {
    pub window: gtk::Window,
    pub alive: Rc<Cell<bool>>,
    go: Rc<dyn Fn(&str)>,
    close: Rc<dyn Fn()>,
}

impl Ctx {
    pub fn new(window: gtk::Window, alive: Rc<Cell<bool>>, go: Rc<dyn Fn(&str)>, close: Rc<dyn Fn()>) -> Self {
        Self { window, alive, go, close }
    }

    pub fn is_alive(&self) -> bool {
        self.alive.get()
    }

    /// Switch Settings to another section.
    pub fn go(&self, section: &str) {
        (self.go)(section);
    }

    pub fn close(&self) {
        (self.close)();
    }

    /// Run `f` every `every` while Settings is open and `f` returns true.
    pub fn poll(&self, every: Duration, f: impl Fn() -> bool + 'static) {
        let alive = self.alive.clone();
        glib::timeout_add_local(every, move || {
            if alive.get() && f() {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
}

/// A scrolling page of preference groups, one group per section of the
/// web page. Not an `adw::PreferencesPage`: its clamp is narrower than the
/// room the settings rows want. Our own clamp (960 px) only stops a
/// full-screen page stretching rows across a wide monitor.
#[derive(Clone)]
pub struct Page {
    pub widget: gtk::ScrolledWindow,
    body: gtk::Box,
}

impl Page {
    pub fn add(&self, group: &impl IsA<gtk::Widget>) {
        self.body.append(group);
    }

    pub fn remove(&self, group: &impl IsA<gtk::Widget>) {
        if group.parent().is_some_and(|p| p == self.body) {
            self.body.remove(group);
        }
    }

    pub fn upcast(self) -> gtk::Widget {
        self.widget.upcast()
    }
}

pub fn page(title: &str) -> Page {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(22)
        .margin_top(14)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .css_classes(["settings-page"])
        .build();
    body.set_widget_name(title);
    let clamp = adw::Clamp::builder().maximum_size(960).tightening_threshold(720).child(&body).build();
    let widget = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .child(&clamp)
        .build();
    Page { widget, body }
}

/// A titled group; `description` is the web page's section hint.
pub fn group(title: &str, description: Option<&str>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(title).build();
    if let Some(d) = description {
        g.set_description(Some(d));
    }
    g
}

/// One settings line: label column, body (value and hint), action column,
/// and an optional full-width widget below (progress bars, wide inputs).
#[derive(Clone)]
pub struct Row {
    pub widget: gtk::ListBoxRow,
    label: gtk::Label,
    value: gtk::Label,
    hint: gtk::Label,
    action: adw::WrapBox,
    below: gtk::Box,
}

impl Row {
    pub fn new(label: &str) -> Self {
        Self::with_prefix(None::<&gtk::Widget>, label)
    }

    /// A row whose label is preceded by a small widget (a colour dot).
    pub fn with_prefix(prefix: Option<&impl IsA<gtk::Widget>>, label: &str) -> Self {
        let widget = gtk::ListBoxRow::builder().activatable(false).selectable(false).css_classes(["settings-row"]).build();
        let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        let line = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(16).build();

        let label_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .valign(gtk::Align::Start)
            .width_request(170)
            .build();
        if let Some(p) = prefix {
            label_box.append(p);
        }
        let label = gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .css_classes(["settings-row-label"])
            .build();
        label_box.append(&label);
        line.append(&label_box);

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
        let value = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .visible(false)
            .css_classes(["settings-row-value"])
            .build();
        let hint = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .visible(false)
            .css_classes(["settings-row-hint"])
            .build();
        body.append(&value);
        body.append(&hint);
        line.append(&body);

        // Wraps: a row with several buttons must not set the page's minimum width.
        let action = adw::WrapBox::builder().child_spacing(8).line_spacing(6).valign(gtk::Align::Center).build();
        line.append(&action);
        outer.append(&line);
        follow_narrow(&line, move |line, narrow| {
            label_box.set_width_request(if narrow { -1 } else { 170 });
            line.set_orientation(if narrow { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
            line.set_spacing(if narrow { 6 } else { 16 });
        });
        let below = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).visible(false).build();
        outer.append(&below);
        widget.set_child(Some(&outer));
        Self { widget, label, value, hint, action, below }
    }

    pub fn value(self, v: &str) -> Self {
        self.set_value(v);
        self
    }

    pub fn hint(self, h: &str) -> Self {
        self.set_hint(h);
        self
    }

    /// Paths and links: let the user copy them.
    pub fn selectable(self) -> Self {
        self.value.set_selectable(true);
        self
    }

    /// Monospace value (paths, commands).
    pub fn code(self) -> Self {
        self.value.add_css_class("setting-code");
        self
    }

    pub fn action(self, w: &impl IsA<gtk::Widget>) -> Self {
        self.add_action(w);
        self
    }

    pub fn add_action(&self, w: &impl IsA<gtk::Widget>) {
        self.action.append(w);
    }

    /// Clicking the words toggles the switch, like a label's `for`.
    pub fn activates(&self, sw: &gtk::Switch) {
        self.widget.set_activatable(true);
        let sw = sw.clone();
        self.widget.connect_activate(move |_| {
            if sw.is_sensitive() {
                sw.set_active(!sw.is_active());
            }
        });
    }

    pub fn set_label(&self, v: &str) {
        self.label.set_label(v);
    }

    pub fn set_value(&self, v: &str) {
        self.value.set_label(v);
        self.value.set_visible(!v.is_empty());
    }

    pub fn set_hint(&self, h: &str) {
        self.hint.set_label(h);
        self.hint.set_visible(!h.is_empty());
        self.hint.remove_css_class("error");
    }

    /// The hint in the danger colour.
    pub fn set_error(&self, e: &str) {
        self.set_hint(e);
        if !e.is_empty() {
            self.hint.add_css_class("error");
        }
    }

    /// Show `w` under the line, full width (hidden again with `clear_below`).
    pub fn set_below(&self, w: &impl IsA<gtk::Widget>) {
        self.clear_below();
        self.below.append(w);
        self.below.set_visible(true);
    }

    pub fn clear_below(&self) {
        while let Some(c) = self.below.first_child() {
            self.below.remove(&c);
        }
        self.below.set_visible(false);
    }
}

/// A switch whose `on_change` runs only for user toggles; `set_quiet`
/// moves it without a callback (initial load, rollback on failure).
#[derive(Clone)]
pub struct Switch {
    pub widget: gtk::Switch,
    quiet: Rc<Cell<bool>>,
}

impl Switch {
    pub fn new(active: bool) -> Self {
        let widget = gtk::Switch::builder().active(active).valign(gtk::Align::Center).build();
        Self { widget, quiet: Rc::new(Cell::new(false)) }
    }

    pub fn on_change(&self, f: impl Fn(bool) + 'static) {
        let quiet = self.quiet.clone();
        self.widget.connect_state_set(move |_, state| {
            if !quiet.get() {
                f(state);
            }
            glib::Propagation::Proceed
        });
    }

    pub fn set_quiet(&self, active: bool) {
        self.quiet.set(true);
        self.widget.set_active(active);
        self.quiet.set(false);
    }

    pub fn set_sensitive(&self, v: bool) {
        self.widget.set_sensitive(v);
    }
}

/// A row with a switch in the action column: `label`, `hint`, the switch.
pub fn switch_row(label: &str, hint: &str, active: bool) -> (Row, Switch) {
    let sw = Switch::new(active);
    let row = Row::new(label).hint(hint).action(&sw.widget);
    row.activates(&sw.widget);
    (row, sw)
}

/// The settings pages' small button.
pub fn button(label: &str) -> gtk::Button {
    gtk::Button::builder().label(label).css_classes(["btn", "small"]).valign(gtk::Align::Center).build()
}

pub fn danger_button(label: &str) -> gtk::Button {
    let b = button(label);
    b.add_css_class("danger");
    b
}

/// A button that shows `busy_label` and goes insensitive while an action runs.
#[derive(Clone)]
pub struct BusyButton {
    pub widget: gtk::Button,
    idle: Rc<RefCell<String>>,
    busy_label: String,
}

impl BusyButton {
    pub fn new(label: &str, busy_label: &str) -> Self {
        Self { widget: button(label), idle: Rc::new(RefCell::new(label.to_string())), busy_label: busy_label.to_string() }
    }

    pub fn set_label(&self, label: &str) {
        *self.idle.borrow_mut() = label.to_string();
        if self.widget.is_sensitive() {
            self.widget.set_label(label);
        }
    }

    pub fn set_busy(&self, busy: bool) {
        self.widget.set_sensitive(!busy);
        let idle = self.idle.borrow().clone();
        self.widget.set_label(if busy { &self.busy_label } else { &idle });
    }
}

/// The mini progress bar under a row. Indeterminate phases pulse while the
/// dialog is open.
pub fn progress_bar(ctx: &Ctx, fraction: f64, indeterminate: bool) -> gtk::ProgressBar {
    let bar = gtk::ProgressBar::builder().fraction(fraction.clamp(0.0, 1.0)).css_classes(["mini"]).build();
    if indeterminate {
        let alive = ctx.alive.clone();
        let weak = bar.downgrade();
        glib::timeout_add_local(Duration::from_millis(120), move || match weak.upgrade() {
            Some(b) if alive.get() && b.parent().is_some() => {
                b.pulse();
                glib::ControlFlow::Continue
            }
            _ => glib::ControlFlow::Break,
        });
    }
    bar
}

/// A muted one-liner (section notes, empty states).
pub fn note(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(["settings-note"])
        .build()
}

/// A monospace one-liner (commands, URLs).
pub fn code(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).selectable(true).wrap(true).css_classes(["setting-code"]).build()
}

/// An inline status label in a row's action column.
pub fn status_label() -> gtk::Label {
    gtk::Label::builder().xalign(1.0).css_classes(["settings-status"]).build()
}

/// A link that opens in the system browser.
pub fn link(uri: &str, label: &str) -> gtk::LinkButton {
    let b = gtk::LinkButton::with_label(uri, label);
    b.set_halign(gtk::Align::Start);
    b.add_css_class("settings-link");
    b
}
