//! Per-game emulator settings (the web UI's `GameSettingsDialog`): each key
//! overrides the global one (engine, CRT shader, fullscreen, CPU cycles,
//! custom DOSBox config). An installed eXoScummVM game gets the variant
//! tree's menus (edition, sound, subtitles, aspect) instead of the DOSBox
//! controls; Win9x rows get the same form as DOS rows, as on the web.

use std::cell::RefCell;
use std::rc::Rc;

use exorchy_core::commands::scummvm::{ScummVmVariants, SvmOptions, SvmVariant};
use exorchy_core::commands::{games, scummvm};
use exorchy_core::models::Game;
use gtk::glib;
use adw::prelude::*;

use crate::app;

// "eXo's choice" names what it resolves to for this game.
const ENGINE_EXO_STAGING: [(&str, &str); 3] = [("", "eXo's choice (DOSBox Staging)"), ("staging", "DOSBox Staging"), ("dosbox-x", "DOSBox-X")];
const ENGINE_EXO_X: [(&str, &str); 3] = [("", "eXo's choice (DOSBox-X)"), ("staging", "DOSBox Staging"), ("dosbox-x", "DOSBox-X")];
const SHADER: [(&str, &str); 3] = [("", "Default (global)"), ("crt-auto", "On"), ("sharp", "Off")];
const FULLSCREEN: [(&str, &str); 3] = [("", "Default (global)"), ("true", "On"), ("false", "Off")];
const CYCLES: [(&str, &str); 4] = [("", "Default (game's own)"), ("auto", "Auto"), ("max", "Max"), ("fixed", "Fixed")];
const ON_OFF: [(&str, &str); 2] = [("false", "Off"), ("true", "On")];
const ASPECT: [(&str, &str); 2] = [("true", "Corrected (4:3)"), ("false", "Pixel-exact")];

pub fn open(parent: &impl IsA<gtk::Widget>, game: &Game) {
    let Some(id) = game.id else { return };
    let parent = parent.clone().upcast::<gtk::Widget>();
    let title = format!("Game Settings: {}", game.title);
    // Read everything first (local and quick), then build the dialog with its
    // form in place: a dialog presented around a "Loading…" line kept that
    // height when the form arrived, and showed one row.
    let core = app::core();
    app::local(async move {
        let c = core.clone();
        let svm = app::call(async move { scummvm::scummvm_variants(c.state(), id).await }).await.ok().flatten();
        let c = core.clone();
        let engine = app::call(async move { games::game_engine_info(c.state(), id).await }).await.ok();
        let c = core.clone();
        let settings = app::call(async move { games::get_game_settings(c.state(), id).await }).await;

        let dialog = adw::Dialog::builder().title(title).content_width(540).build();
        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&adw::HeaderBar::new());
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_start(16).margin_end(16).margin_top(4).margin_bottom(16).css_classes(["dialog-body"]).build();
        match settings {
            Ok(s) => build_form(&dialog, &body, id, svm, engine, s),
            Err(e) => body.append(&gtk::Label::builder().label(format!("Couldn't load the settings: {e}")).wrap(true).xalign(0.0).css_classes(["danger"]).build()),
        }
        tv.set_content(Some(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).max_content_height(720).child(&body).build()));
        dialog.set_child(Some(&tv));
        dialog.present(Some(&parent));
    });
}

/// A labelled row: the label column is fixed so the controls line up.
fn row(body: &gtk::Box, label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).build();
    r.append(&gtk::Label::builder().label(label).xalign(0.0).width_chars(16).css_classes(["form-label"]).build());
    control.set_hexpand(true);
    r.append(control);
    body.append(&r);
    r
}

fn note(body: &gtk::Box, text: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["form-note", "muted", "small"]).build();
    body.append(&l);
    l
}

fn drop(options: &[(&str, &str)], current: &str) -> gtk::DropDown {
    let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
    let d = gtk::DropDown::from_strings(&labels);
    d.add_css_class("drop");
    let idx = options.iter().position(|(v, _)| *v == current).unwrap_or(0);
    d.set_selected(idx as u32);
    d
}

fn drop_value(d: &gtk::DropDown, options: &[(&str, &str)]) -> String {
    options.get(d.selected() as usize).map(|(v, _)| v.to_string()).unwrap_or_default()
}

fn strings(d: &gtk::DropDown, items: &[String], current: Option<&str>) {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    d.set_model(Some(&gtk::StringList::new(&refs)));
    let idx = current.and_then(|c| items.iter().position(|i| i == c)).unwrap_or(0);
    d.set_selected(idx as u32);
}

fn string_value(d: &gtk::DropDown, items: &[String]) -> Option<String> {
    items.get(d.selected() as usize).cloned()
}

/// The ScummVM menus eXo would ask about at launch. The version itself is
/// picked in the game's panel; `selected.variant` is what these apply to.
struct SvmForm {
    tree: ScummVmVariants,
    sub: Option<gtk::DropDown>,
    sub_names: Vec<String>,
    sound: gtk::DropDown,
    sound_row: gtk::Box,
    sounds: RefCell<Vec<String>>,
    subtitles: gtk::DropDown,
    subtitles_row: gtk::Box,
    aspect: gtk::DropDown,
}

impl SvmForm {
    fn variant(&self) -> Option<&SvmVariant> {
        let sel = self.tree.selected.variant.as_deref()?;
        self.tree.variants.iter().find(|v| v.name == sel)
    }

    fn sub_name(&self) -> Option<String> {
        self.sub.as_ref().and_then(|d| string_value(d, &self.sub_names))
    }

    /// A sub's own sound list wins when non-empty; subtitles are offered
    /// when either level has them.
    fn sync(&self) {
        let Some(v) = self.variant() else { return };
        let sub = self.sub_name().and_then(|n| v.subs.iter().find(|s| s.name == n));
        let sounds = match sub {
            Some(s) if !s.sounds.is_empty() => s.sounds.clone(),
            _ => v.sounds.clone(),
        };
        let current = string_value(&self.sound, &self.sounds.borrow());
        strings(&self.sound, &sounds, current.as_deref());
        self.sound_row.set_visible(!sounds.is_empty());
        self.sounds.replace(sounds);
        self.subtitles_row.set_visible(sub.map(|s| s.has_subtitles).unwrap_or(false) || v.has_subtitles);
    }
}

fn build_form(dialog: &adw::Dialog, body: &gtk::Box, id: i64, svm: Option<ScummVmVariants>, engine_info: Option<games::GameEngineInfo>, s: games::GameSettings) {
    let svm_form = svm.map(|tree| {
        let variant = tree.selected.variant.as_deref().and_then(|sel| tree.variants.iter().find(|v| v.name == sel)).cloned();
        let sub_names: Vec<String> = variant.as_ref().map(|v| v.subs.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
        let sub = (!sub_names.is_empty()).then(|| {
            let d = gtk::DropDown::from_strings(&[]);
            d.add_css_class("drop");
            strings(&d, &sub_names, tree.selected.sub.as_deref());
            row(body, "Edition", &d);
            d
        });
        let sound = gtk::DropDown::from_strings(&[]);
        sound.add_css_class("drop");
        let sound_row = row(body, "Sound", &sound);
        let subtitles = drop(&ON_OFF, if tree.selected.subtitles { "true" } else { "false" });
        let subtitles_row = row(body, "Subtitles", &subtitles);
        let aspect = drop(&ASPECT, if tree.selected.aspect { "true" } else { "false" });
        row(body, "Aspect ratio", &aspect);
        note(body, "The version itself is picked in the game's panel; these are the menus eXo would ask about at launch.");
        let initial_sound = tree.selected.sound.clone();
        let f = Rc::new(SvmForm { tree, sub, sub_names, sound, sound_row, sounds: RefCell::new(Vec::new()), subtitles, subtitles_row, aspect });
        // The stored sound must survive the first sync, which reads the
        // (empty) current list.
        if let Some(v) = f.variant() {
            let sub = f.sub_name().and_then(|n| v.subs.iter().find(|s| s.name == n));
            let sounds = match sub {
                Some(s) if !s.sounds.is_empty() => s.sounds.clone(),
                _ => v.sounds.clone(),
            };
            strings(&f.sound, &sounds, initial_sound.as_deref());
            f.sounds.replace(sounds);
        }
        f.sync();
        if let Some(d) = &f.sub {
            d.connect_selected_notify(glib::clone!(#[weak] f, move |_| f.sync()));
        }
        f
    });

    let is_svm = svm_form.is_some();
    // DOS and Windows 3.x games pick their emulator; the backend answers
    // None for every other collection.
    let exo_x = engine_info.as_ref().and_then(|e| e.exo_engine.as_deref()) == Some("dosbox-x");
    let prints = engine_info.as_ref().is_some_and(|e| e.prints);
    let engine_opts: &'static [(&str, &str); 3] = if exo_x { &ENGINE_EXO_X } else { &ENGINE_EXO_STAGING };
    let engine = (!is_svm && engine_info.as_ref().is_some_and(|e| e.engine.is_some())).then(|| {
        let d = drop(engine_opts, s.engine.as_deref().unwrap_or(""));
        row(body, "Emulator", &d);
        note(
            body,
            if prints {
                "This game prints. Under DOSBox-X its pages are saved as PNG images in the game's !prints folder; DOSBox Staging has no printer."
            } else if exo_x {
                "eXo runs this game under DOSBox-X. DOSBox Staging adds CRT shaders and a newer feature set, but eXo did not test the game with it."
            } else {
                "eXo's configuration for this game targets DOSBox Staging or DOSBox ECE (Windows only); DOSBox-X is an alternative if it misbehaves."
            },
        );
        d
    });
    let glshader = (!is_svm).then(|| {
        let d = drop(&SHADER, s.glshader.as_deref().unwrap_or(""));
        row(body, "CRT Shader", &d);
        let n = note(body, "CRT shaders are a DOSBox Staging feature: they do not apply while this game runs under DOSBox-X. Switch the emulator above to DOSBox Staging for the CRT look.");
        // What would run it with the choice currently in the dialog is what
        // the note reflects - switching the engine updates it before saving.
        let uses_x = glib::clone!(#[weak] d, #[weak] n, #[strong] engine, move || {
            let x = match engine.as_ref().map(|e| drop_value(e, engine_opts)) {
                Some(v) if v == "dosbox-x" => true,
                Some(v) if v == "staging" => false,
                _ => exo_x,
            };
            d.set_sensitive(!x);
            n.set_visible(x);
        });
        uses_x();
        if let Some(e) = &engine {
            e.connect_selected_notify(move |_| uses_x());
        }
        d
    });
    let fullscreen = drop(&FULLSCREEN, s.fullscreen.as_deref().unwrap_or(""));
    row(body, "Fullscreen", &fullscreen);
    let cycles = (!is_svm).then(|| {
        let stored = s.cycles.clone().unwrap_or_default();
        let fixed = !stored.is_empty() && stored.chars().all(|c| c.is_ascii_digit());
        let mode = drop(&CYCLES, if fixed { "fixed" } else { &stored });
        let value = gtk::SpinButton::with_range(100.0, 100_000.0, 500.0);
        value.set_value(if fixed { stored.parse().unwrap_or(10_000.0) } else { 10_000.0 });
        value.set_visible(fixed);
        value.add_css_class("field");
        let bx = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bx.append(&mode);
        bx.append(&value);
        row(body, "CPU Cycles", &bx);
        mode.connect_selected_notify(glib::clone!(#[weak] value, move |m| value.set_visible(drop_value(m, &CYCLES) == "fixed")));
        (mode, value)
    });
    let custom = (!is_svm).then(|| {
        body.append(&gtk::Label::builder().label("Custom DOSBox Config").xalign(0.0).css_classes(["form-label"]).build());
        body.append(&gtk::Label::builder().label("Appended to the game's conf, e.g. [cpu] cycles = max  or  [sblaster] sbtype = sb16").xalign(0.0).wrap(true).css_classes(["muted", "small"]).build());
        let tv = gtk::TextView::builder().monospace(true).wrap_mode(gtk::WrapMode::None).accepts_tab(false).css_classes(["conf-editor"]).build();
        tv.buffer().set_text(s.custom_conf.as_deref().unwrap_or(""));
        let sw = gtk::ScrolledWindow::builder().min_content_height(120).child(&tv).css_classes(["conf-scroller"]).build();
        body.append(&sw);
        tv
    });

    let actions = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).halign(gtk::Align::End).margin_top(4).build();
    let error = gtk::Label::builder().xalign(0.0).wrap(true).hexpand(true).css_classes(["danger", "small"]).visible(false).build();
    actions.append(&error);
    let cancel = gtk::Button::builder().label("Cancel").css_classes(["btn"]).build();
    let save = gtk::Button::builder().label("Save").css_classes(["btn", "primary"]).build();
    actions.append(&cancel);
    actions.append(&save);
    body.append(&actions);
    cancel.connect_clicked(glib::clone!(#[weak] dialog, move |_| { dialog.close(); }));

    let dialog = dialog.clone();
    save.connect_clicked(move |save| {
        save.set_sensitive(false);
        save.set_label("Saving…");
        error.set_visible(false);
        let fullscreen = Some(drop_value(&fullscreen, &FULLSCREEN)).filter(|v| !v.is_empty());
        let core = app::core();
        let fut: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> = if let Some(f) = &svm_form {
            let options = SvmOptions {
                variant: f.tree.selected.variant.clone(),
                sub: f.sub_name(),
                sound: string_value(&f.sound, &f.sounds.borrow()).filter(|_| f.sound_row.is_visible()),
                subtitles: Some(drop_value(&f.subtitles, &ON_OFF) == "true"),
                aspect: Some(drop_value(&f.aspect, &ASPECT) == "true"),
            };
            Box::pin(async move {
                scummvm::set_scummvm_options(core.state(), id, options).await?;
                games::set_game_settings(core.state(), id, None, None, fullscreen, None, None).await
            })
        } else {
            let engine = engine.as_ref().map(|d| drop_value(d, engine_opts)).filter(|v| !v.is_empty());
            let glshader = glshader.as_ref().map(|d| drop_value(d, &SHADER)).filter(|v| !v.is_empty());
            let cycles = cycles.as_ref().and_then(|(mode, value)| match drop_value(mode, &CYCLES).as_str() {
                "" => None,
                "fixed" => Some((value.value() as i64).to_string()),
                other => Some(other.to_string()),
            });
            let custom = custom.as_ref().map(|tv| {
                let b = tv.buffer();
                b.text(&b.start_iter(), &b.end_iter(), false).to_string()
            }).filter(|v| !v.trim().is_empty());
            Box::pin(async move { games::set_game_settings(core.state(), id, engine, glshader, fullscreen, cycles, custom).await })
        };
        let (dialog, error, save) = (dialog.clone(), error.clone(), save.clone());
        app::spawn(fut, move |res| {
            save.set_sensitive(true);
            save.set_label("Save");
            match res {
                Ok(()) => {
                    dialog.close();
                }
                Err(e) => {
                    error.set_label(&format!("Failed to save: {e}"));
                    error.set_visible(true);
                }
            }
        });
    });
}
