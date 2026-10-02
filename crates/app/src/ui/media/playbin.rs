//! `PlaybinStream`: a `gtk::MediaStream` over GStreamer's classic `playbin`,
//! standing in for `gtk::MediaFile`. GTK's own GStreamer backend goes through
//! GstPlay, which is fixed on `playbin3`; its decodebin3 aborts the whole
//! process on a track change (`mq_slot_handle_stream_start: assertion failed:
//! (collection)`, GStreamer 1.28), so a few quick clicks through the library
//! were enough to crash eXorchy. `playbin` autoplugs with decodebin2 and has
//! no such path.
//!
//! The stream speaks GTK's protocol: `stream_prepared` once the pipeline has
//! prerolled, `update` with the position while playing, `stream_ended` on
//! EOS, `set_error` on an element error. Video frames come off an `appsink` as
//! RGBA and are painted as a `gdk::MemoryTexture`, so `gtk::Picture` and
//! `gtk::Video` take the stream like any other paintable. An audio stream
//! leaves video out of the pipeline entirely.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gst::prelude::*;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk};

/// How often a playing stream reports its position (the seek bar's tick).
const POSITION_TICK_MS: u64 = 100;

/// One decoded frame, handed from the streaming thread to the main loop.
struct Frame {
    bytes: glib::Bytes,
    width: i32,
    height: i32,
    stride: usize,
}

#[derive(Default)]
struct Shared {
    frame: Mutex<Option<Frame>>,
    /// A paint is already queued on the main loop: frames that arrive before
    /// it runs replace the waiting one instead of queueing more work.
    queued: AtomicBool,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PlaybinStream {
        pub(super) playbin: RefCell<Option<gst::Element>>,
        pub(super) bus_watch: RefCell<Option<gst::bus::BusWatchGuard>>,
        pub(super) path: RefCell<Option<PathBuf>>,
        pub(super) video: Cell<bool>,
        /// `stream_prepared` was sent for the current file.
        pub(super) prepared: Cell<bool>,
        pub(super) ticker: RefCell<Option<glib::SourceId>>,
        pub(super) texture: RefCell<Option<gdk::Texture>>,
        pub(super) shared: Arc<Shared>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PlaybinStream {
        const NAME: &'static str = "ExorchyPlaybinStream";
        type Type = super::PlaybinStream;
        type ParentType = gtk::MediaStream;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for PlaybinStream {
        fn dispose(&self) {
            self.obj().unload();
        }
    }

    impl MediaStreamImpl for PlaybinStream {
        fn play(&self) -> bool {
            let Some(playbin) = self.playbin.borrow().clone() else { return false };
            if self.obj().is_ended() {
                let _ = playbin.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, gst::ClockTime::ZERO);
            }
            if playbin.set_state(gst::State::Playing).is_err() {
                return false;
            }
            self.obj().start_ticker();
            true
        }

        fn pause(&self) {
            self.obj().stop_ticker();
            if let Some(playbin) = self.playbin.borrow().as_ref() {
                let _ = playbin.set_state(gst::State::Paused);
            }
        }

        fn seek(&self, timestamp: i64) {
            let obj = self.obj();
            let Some(playbin) = self.playbin.borrow().clone() else {
                obj.seek_failed();
                return;
            };
            let to = gst::ClockTime::from_useconds(timestamp.max(0) as u64);
            if playbin.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, to).is_ok() {
                obj.seek_success();
                obj.update(timestamp.max(0));
            } else {
                obj.seek_failed();
            }
        }

        fn update_audio(&self, muted: bool, volume: f64) {
            if let Some(playbin) = self.playbin.borrow().as_ref() {
                playbin.set_property("mute", muted);
                // GTK's volume is a slider position; playbin's is linear
                // amplitude. Cubic, as GTK's own backend maps it.
                playbin.set_property("volume", volume.clamp(0.0, 1.0).powi(3));
            }
        }
    }

    impl PaintableImpl for PlaybinStream {
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            if let Some(texture) = self.texture.borrow().as_ref() {
                let snapshot = snapshot.downcast_ref::<gtk::Snapshot>().expect("a GTK snapshot");
                snapshot.append_scaled_texture(texture, gsk::ScalingFilter::Linear, &graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
            }
        }

        fn current_image(&self) -> gdk::Paintable {
            match self.texture.borrow().as_ref() {
                Some(t) => t.clone().upcast(),
                None => gdk::Paintable::new_empty(0, 0),
            }
        }

        fn intrinsic_width(&self) -> i32 {
            self.texture.borrow().as_ref().map(|t| t.width()).unwrap_or(0)
        }

        fn intrinsic_height(&self) -> i32 {
            self.texture.borrow().as_ref().map(|t| t.height()).unwrap_or(0)
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            match self.texture.borrow().as_ref() {
                Some(t) if t.height() > 0 => t.width() as f64 / t.height() as f64,
                _ => 0.0,
            }
        }
    }
}

glib::wrapper! {
    pub struct PlaybinStream(ObjectSubclass<imp::PlaybinStream>)
        @extends gtk::MediaStream,
        @implements gdk::Paintable;
}

impl PlaybinStream {
    /// Sound only: no video branch in the pipeline at all.
    pub fn new_audio() -> Self {
        Self::with_video(false)
    }

    /// Sound and frames, painted as this paintable.
    pub fn new_video() -> Self {
        Self::with_video(true)
    }

    fn with_video(video: bool) -> Self {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            if let Err(e) = gst::init() {
                log::warn!("media: GStreamer did not initialise: {e}");
            }
        });
        let stream: Self = glib::Object::new();
        stream.imp().video.set(video);
        stream
    }

    /// The file this stream plays, if any.
    pub fn file(&self) -> Option<PathBuf> {
        self.imp().path.borrow().clone()
    }

    /// Load a file (prerolled, paused), or unload with `None`.
    pub fn set_filename(&self, path: Option<&Path>) {
        self.unload();
        let Some(path) = path else { return };
        match self.load(path) {
            Ok(()) => {
                self.imp().path.replace(Some(path.to_path_buf()));
            }
            Err(e) => {
                self.unload();
                self.set_error(glib::Error::new(gtk::gio::IOErrorEnum::NotSupported, &e));
            }
        }
    }

    pub fn clear(&self) {
        self.set_filename(None);
    }

    fn load(&self, path: &Path) -> Result<(), String> {
        let imp = self.imp();
        let uri = glib::filename_to_uri(path, None).map_err(|e| e.to_string())?;
        let playbin = gst::ElementFactory::make("playbin").property("uri", uri.as_str()).build().map_err(|e| e.to_string())?;
        if imp.video.get() {
            playbin.set_property_from_str("flags", "audio+video+soft-volume");
            playbin.set_property("video-sink", &self.video_sink()?);
        } else {
            playbin.set_property_from_str("flags", "audio+soft-volume");
        }
        // Developer aid: tests and headless runs stay silent.
        if std::env::var_os("EXORCHY_MEDIA_FAKE_SINKS").is_some_and(|v| v == "1") {
            let sink = gst::ElementFactory::make("fakesink").property("sync", true).build().map_err(|e| e.to_string())?;
            playbin.set_property("audio-sink", &sink);
        }
        playbin.set_property("mute", self.is_muted());
        playbin.set_property("volume", self.volume().clamp(0.0, 1.0).powi(3));

        let bus = playbin.bus().ok_or("playbin has no bus")?;
        let weak = self.downgrade();
        let watch = bus
            .add_watch_local(move |_, msg| {
                if let Some(s) = weak.upgrade() {
                    s.on_message(msg);
                }
                glib::ControlFlow::Continue
            })
            .map_err(|e| e.to_string())?;
        imp.bus_watch.replace(Some(watch));
        imp.playbin.replace(Some(playbin.clone()));
        // Preroll: `async-done` brings the duration and `stream_prepared`.
        playbin.set_state(gst::State::Paused).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// `videoconvert ! appsink` taking RGBA; each sample becomes a texture.
    fn video_sink(&self) -> Result<gst::Element, String> {
        let caps = gst_video::VideoCapsBuilder::new().format(gst_video::VideoFormat::Rgba).build();
        let appsink = gst_app::AppSink::builder().caps(&caps).max_buffers(2).drop(true).build();
        let shared = self.imp().shared.clone();
        let target = glib::SendWeakRef::from(self.downgrade());
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(frame) = frame_of(&sample) {
                        *shared.frame.lock().unwrap() = Some(frame);
                        if !shared.queued.swap(true, Ordering::AcqRel) {
                            let target = target.clone();
                            glib::MainContext::default().invoke(move || {
                                if let Some(s) = target.upgrade() {
                                    s.paint_waiting_frame();
                                }
                            });
                        }
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        let convert = gst::ElementFactory::make("videoconvert").build().map_err(|e| e.to_string())?;
        let bin = gst::Bin::new();
        bin.add_many([&convert, appsink.upcast_ref()]).map_err(|e| e.to_string())?;
        convert.link(&appsink).map_err(|e| e.to_string())?;
        let pad = convert.static_pad("sink").ok_or("videoconvert has no sink pad")?;
        let ghost = gst::GhostPad::with_target(&pad).map_err(|e| e.to_string())?;
        bin.add_pad(&ghost).map_err(|e| e.to_string())?;
        Ok(bin.upcast())
    }

    fn paint_waiting_frame(&self) {
        let imp = self.imp();
        imp.shared.queued.store(false, Ordering::Release);
        let Some(frame) = imp.shared.frame.lock().unwrap().take() else { return };
        if imp.playbin.borrow().is_none() {
            return;
        }
        let texture: gdk::Texture = gdk::MemoryTexture::new(frame.width, frame.height, gdk::MemoryFormat::R8g8b8a8, &frame.bytes, frame.stride).upcast();
        let resized = imp.texture.borrow().as_ref().map(|t| (t.width(), t.height())) != Some((frame.width, frame.height));
        imp.texture.replace(Some(texture));
        if resized {
            self.invalidate_size();
        }
        self.invalidate_contents();
    }

    fn on_message(&self, msg: &gst::Message) {
        use gst::MessageView;
        let imp = self.imp();
        let Some(playbin) = imp.playbin.borrow().clone() else { return };
        match msg.view() {
            MessageView::AsyncDone(_) => {
                if !imp.prepared.get() {
                    imp.prepared.set(true);
                    let has_audio = playbin.property::<i32>("n-audio") > 0;
                    let has_video = imp.video.get() && playbin.property::<i32>("n-video") > 0;
                    let mut seeking = gst::query::Seeking::new(gst::Format::Time);
                    let seekable = playbin.query(&mut seeking) && seeking.result().0;
                    let duration = playbin.query_duration::<gst::ClockTime>().map(|d| d.useconds() as i64).unwrap_or(0);
                    self.stream_prepared(has_audio, has_video, seekable, duration);
                }
            }
            MessageView::Eos(_) => {
                self.stop_ticker();
                self.report_position();
                self.stream_ended();
            }
            MessageView::Error(err) => {
                self.stop_ticker();
                let message = err.error().message().to_string();
                log::warn!("media: playback failed for {:?}: {message}", imp.path.borrow());
                let _ = playbin.set_state(gst::State::Null);
                if self.error().is_none() {
                    self.set_error(glib::Error::new(gtk::gio::IOErrorEnum::Failed, &message));
                }
            }
            _ => {}
        }
    }

    fn report_position(&self) {
        // GTK takes positions only once the stream is prepared.
        if !self.is_prepared() {
            return;
        }
        let Some(playbin) = self.imp().playbin.borrow().clone() else { return };
        if let Some(pos) = playbin.query_position::<gst::ClockTime>() {
            self.update(pos.useconds() as i64);
        }
    }

    fn start_ticker(&self) {
        self.stop_ticker();
        let weak = self.downgrade();
        let id = glib::timeout_add_local(Duration::from_millis(POSITION_TICK_MS), move || {
            let Some(s) = weak.upgrade() else { return glib::ControlFlow::Break };
            s.report_position();
            glib::ControlFlow::Continue
        });
        self.imp().ticker.replace(Some(id));
    }

    fn stop_ticker(&self) {
        if let Some(id) = self.imp().ticker.take() {
            id.remove();
        }
    }

    /// Tear the pipeline down; GTK's state returns to unprepared.
    fn unload(&self) {
        let imp = self.imp();
        self.stop_ticker();
        imp.bus_watch.replace(None);
        if let Some(playbin) = imp.playbin.take() {
            let _ = playbin.set_state(gst::State::Null);
        }
        imp.path.replace(None);
        imp.shared.frame.lock().unwrap().take();
        if imp.texture.take().is_some() {
            self.invalidate_size();
            self.invalidate_contents();
        }
        if imp.prepared.replace(false) {
            self.stream_unprepared();
        }
    }
}

fn frame_of(sample: &gst::Sample) -> Option<Frame> {
    let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let stride = usize::try_from(*info.stride().first()?).ok()?;
    Some(Frame { bytes: glib::Bytes::from(map.as_slice()), width: info.width() as i32, height: info.height() as i32, stride })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spin(ms: u64) {
        let ctx = glib::MainContext::default();
        let until = std::time::Instant::now() + Duration::from_millis(ms);
        while std::time::Instant::now() < until {
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Rapid source changes, the pattern that aborted GTK's playbin3 backend.
    /// Needs a display and real files: `EXORCHY_MEDIA_TEST_FILES=a.mp4:b.mp3
    /// EXORCHY_MEDIA_FAKE_SINKS=1 cargo test -p exorchy playbin -- --ignored`.
    #[test]
    #[ignore]
    fn survives_rapid_source_changes() {
        gtk::init().expect("a display");
        let files: Vec<PathBuf> = std::env::var("EXORCHY_MEDIA_TEST_FILES").expect("files").split(':').map(PathBuf::from).collect();
        let painted = std::rc::Rc::new(Cell::new(0u32));
        for round in 0..60 {
            let path = &files[round % files.len()];
            let stream = if path.extension().is_some_and(|e| e == "mp4") { PlaybinStream::new_video() } else { PlaybinStream::new_audio() };
            let p = painted.clone();
            stream.connect_invalidate_contents(move |_| p.set(p.get() + 1));
            stream.set_filename(Some(path));
            stream.play();
            // Mostly faster than a preroll, now and then long enough to play.
            spin(if round % 10 == 9 { 1500 } else { 20 + (round as u64 * 37) % 200 });
            assert!(stream.error().is_none(), "{path:?}: {:?}", stream.error());
            if round % 10 == 9 {
                assert!(stream.is_prepared(), "{path:?} never prepared");
                assert!(stream.timestamp() > 0, "{path:?} never advanced");
                if stream.is_seekable() {
                    stream.seek(0);
                    spin(100);
                    assert!(!stream.is_seeking(), "{path:?} still seeking");
                }
            }
            stream.pause();
            stream.clear();
        }
        assert!(painted.get() > 0, "no video frame was painted");
    }
}
