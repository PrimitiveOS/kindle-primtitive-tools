//! A native app's event loop, screen output, and local persistence.
//!
//! An app implements `App` and hands it to `run`. The kit reads the display settings, draws each frame, sends taps,
//! swipes and page buttons to the app, saves its data at sleep and exit, and handles USB drive mode.
//!
//! # Under Primitive Launcher (the system layer)
//!
//! Primitive Launcher stays in charge of the Kindle while an app is open. It owns the power button, sleep and wake,
//! the lock screen, quick settings, the power menu and the theme (`docs/system-architecture.md` and
//! `docs/contracts.md` §1.1 in the umbrella repo). It starts the app with `PRIMITIVE_LAUNCHER=1`; `run` reads that
//! once into `under_launcher`. When it is set, the kit does the app's part of the contract, and the app needs no code
//! of its own for it:
//!
//! - **SIGUSR1 asks for a redraw.** The kit sets a handler that only marks the request; a thread wakes the event loop.
//!   Without the launcher the kit leaves SIGUSR1 alone, and the signal ends the app.
//! - **The redraw order.** Each time the loop wakes, it takes a waiting request first, before the event that woke it:
//!   it reads `dark_mode` again from the launcher's `settings.txt`, and draws the whole screen once. An event that
//!   waited while the app was frozen under quick settings is handled in the same pass, so nothing is drawn in the old
//!   theme first. A request that comes while the app sleeps waits for the wake.
//! - **Wake.** The launcher puts the app's saved screen back after the sleep picture, so the kit draws nothing on
//!   wake. Only when a redraw was asked for while the app slept does it draw the whole screen, with a full update.
//! - **No flash on the redraw.** The redraw for a request uses `Display::show_calm` (`primitive_app::display`): the
//!   whole screen with GC16 in partial mode, which changes the pixels without a flash. The launcher has just flashed
//!   the screen for the theme switch, and a second flash would show the inverse of the new theme (the old one) for a
//!   moment.
//! - **The system swipe.** A swipe down from the top edge (it starts in the top 160 px, moves more than 150 px down,
//!   and is mostly vertical) opens the launcher's quick settings. The kit drops it, so the app never gets it in
//!   `App::input` or `App::swipe`. Other swipes from the top edge (up, or sideways) still reach the app.
//! - **Foreground portrait turn.** When the supervisor provides `KL_APP_TURN_FILE`, the kit publishes a `v1 U follow`
//!   or `v1 D fixed` record after the corresponding frame is presented. The scoped record follows successful normal,
//!   calm and USB draws. When the supervisor has no scoped path, the existing `orientation` file is used.
//!
//! The app's library entry declares `# Wake: redraw` in its first ten lines and ignores SIGUSR1 in its own shell
//! (`[ "${PRIMITIVE_LAUNCHER:-0}" = 1 ] && trap '' USR1`), as the app template's `deploy/Hello.sh` does.
//!
//! What an app on this kit must not do: put the Kindle to sleep, draw a lock screen or its own quick settings, load or
//! unload the stock screensaver, or offer its own light or dark switch. Under the launcher the kit ignores
//! `App::preferred_dark_mode` and `dark_mode` in the app's own `settings.txt`: the system theme always wins. On its own
//! (opened from the Kindle library) an app's preference still counts.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use kl_core::{storage_lease::user_storage_is_mounted, Settings};
use kl_kindle::{device, display::Display, profile};
use primitive_input as input;
use primitive_ui::{full_screen_message, Canvas, Input};

mod orientation;
mod storage;
pub mod viewport;

use orientation::{Publisher as TurnPublisher, Turn};
use viewport::runtime::Session as ViewportSession;

pub use kl_core::Rotation;
pub use kl_kindle::{
    device as kindle_device, display, eink, input as kindle_input, profile as kindle_profile,
};
pub use storage::with_storage;

pub const WIDTH: u32 = profile::ACTIVE.screen_w;
pub const HEIGHT: u32 = profile::ACTIVE.screen_h;

/// The redraw needed after an event. Partial updates use the device driver's pixel comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    None,
    Partial,
    Full,
    Exit,
}

pub trait App {
    fn draw(&self, canvas: &mut Canvas);
    fn input(&mut self, input: Input) -> Change;

    /// A swipe with its horizontal start in screen coordinates.
    fn swipe(&mut self, input: Input, _start_x: f32) -> Change {
        self.input(input)
    }

    /// Temporarily override the native portrait turn. `None` follows Launcher settings.
    fn preferred_rotation(&self) -> Option<Rotation> {
        None
    }

    /// Opt in to the negotiated viewport lifecycle. `None` at startup uses
    /// the portrait lifecycle, including when Launcher offers a viewport.
    /// Once negotiated, `None` follows Launcher without leaving the protocol.
    fn viewport_request(&self) -> Option<viewport::ViewportRequest> {
        None
    }

    /// Prepare a renderable candidate while retaining the accepted layout.
    /// The kit calls `finish_viewport` even if staging reports an error.
    fn stage_viewport(&mut self, _candidate: viewport::Viewport) -> io::Result<()> {
        Ok(())
    }

    /// Draw the prepared layout into a canvas sized to the candidate.
    fn draw_staged_viewport(&mut self, canvas: &mut Canvas) {
        self.draw(canvas);
    }

    /// Adopt a verified candidate or discard its staged state. Commit cannot
    /// fail, because the presented frame already belongs to the candidate.
    fn finish_viewport(
        &mut self,
        _candidate: viewport::Viewport,
        _decision: viewport::ViewportDecision,
    ) {
    }

    /// Override the inherited interface colors. Pictures retain their original shades. Under Primitive Launcher the
    /// app follows the system theme: keep this `None` there (see the module docs).
    fn preferred_dark_mode(&self) -> Option<bool> {
        None
    }

    fn press_point(&self, input: Input) -> Option<(f32, f32)> {
        match input {
            Input::Tap(x, y) => Some((x, y)),
            _ => None,
        }
    }

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn tick(&mut self, _now: Instant) -> Change {
        Change::None
    }

    fn save(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn storage_returned(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default, PartialEq, Eq)]
enum State {
    #[default]
    Awake,
    Asleep,
    Closed,
}

/// Shared sleep, wake and exit transitions for device and host app drivers.
#[derive(Default)]
pub struct Lifecycle {
    state: State,
    usb_drive: bool,
}

impl Lifecycle {
    pub fn is_awake(&self) -> bool {
        self.state == State::Awake
    }

    pub fn deadline(&self, app: &impl App) -> Option<Instant> {
        if self.is_awake() && !self.usb_drive {
            app.deadline()
        } else {
            None
        }
    }

    fn set_usb_drive(&mut self, active: bool) -> bool {
        let changed = self.usb_drive != active;
        self.usb_drive = active;
        changed
    }

    /// Draw the pressed style before the app receives the action. Normal frames clear it.
    pub fn press_feedback(
        &self,
        app: &impl App,
        canvas: &mut Canvas,
        input: Input,
        dark: bool,
    ) -> Option<Vec<u8>> {
        if !self.is_awake() || self.usb_drive {
            return None;
        }
        let (x, y) = app.press_point(input)?;
        if !canvas.press_at(x, y) {
            return None;
        }
        let pressed_frame = frame(app, canvas, dark);
        canvas.clear_press();
        Some(pressed_frame)
    }

    pub fn input(
        &mut self,
        app: &mut impl App,
        input: Input,
        on_save_error: impl FnOnce(io::Error),
    ) -> Change {
        self.dispatch_input(app, input, None, on_save_error)
    }

    /// Route a swipe through the same sleep, storage, and exit rules as ordinary input.
    pub fn swipe(
        &mut self,
        app: &mut impl App,
        input: Input,
        start_x: f32,
        on_save_error: impl FnOnce(io::Error),
    ) -> Change {
        self.dispatch_input(app, input, Some(start_x), on_save_error)
    }

    fn dispatch_input(
        &mut self,
        app: &mut impl App,
        input: Input,
        start_x: Option<f32>,
        on_save_error: impl FnOnce(io::Error),
    ) -> Change {
        if self.usb_drive && !matches!(input, Input::Sleep | Input::Wake) {
            return Change::None;
        }
        match (&self.state, input) {
            (State::Awake, Input::Sleep) => {
                self.state = State::Asleep;
                app.input(Input::Sleep);
                persist(app, on_save_error);
                Change::None
            }
            (State::Asleep, Input::Wake) => {
                self.state = State::Awake;
                app.input(Input::Wake);
                Change::Full
            }
            (State::Awake, Input::Wake) | (State::Asleep | State::Closed, _) => Change::None,
            (State::Awake, input) => {
                let change = match (input, start_x) {
                    (Input::Swipe { .. }, Some(start_x)) => app.swipe(input, start_x),
                    _ => app.input(input),
                };
                if change == Change::Exit && !self.finish(app, on_save_error) {
                    return Change::None;
                }
                change
            }
        }
    }

    /// Persist once when an app exits or its event channel closes.
    pub fn close(&mut self, app: &mut impl App, on_save_error: impl FnOnce(io::Error)) {
        self.finish(app, on_save_error);
    }

    fn finish(&mut self, app: &mut impl App, on_save_error: impl FnOnce(io::Error)) -> bool {
        if self.state == State::Closed {
            return true;
        }
        if let Err(error) = app.save() {
            let suspended = error.kind() == io::ErrorKind::WouldBlock;
            on_save_error(error);
            // Stay in the app if USB takes the volume before the exit save finishes.
            if suspended {
                return false;
            }
        }
        self.state = State::Closed;
        true
    }
}

fn persist(app: &mut impl App, on_save_error: impl FnOnce(io::Error)) {
    if let Err(error) = app.save() {
        on_save_error(error);
    }
}

pub fn frame(app: &impl App, canvas: &mut Canvas, dark: bool) -> Vec<u8> {
    canvas.begin();
    app.draw(canvas);
    canvas.finish(effective_dark(
        app.preferred_dark_mode(),
        dark,
        UNDER_LAUNCHER.load(Ordering::Relaxed),
    ))
}

/// Under Primitive Launcher the theme is the system's (`docs/system-architecture.md` §4.2 in the umbrella repo): an
/// app's own preference counts only when it runs on its own.
static UNDER_LAUNCHER: AtomicBool = AtomicBool::new(false);

fn effective_dark(preferred: Option<bool>, system: bool, under_launcher: bool) -> bool {
    if under_launcher { system } else { preferred.unwrap_or(system) }
}

pub fn font_paths(directory: &Path) -> Vec<String> {
    ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf"]
        .into_iter()
        .map(|font| directory.join(font).to_string_lossy().into_owned())
        .collect()
}

pub fn write_png(
    path: &Path,
    width: u32,
    height: u32,
    gray: &[u8],
) -> Result<(), png::EncodingError> {
    let mut png = png::Encoder::new(io::BufWriter::new(File::create(path)?), width, height);
    png.set_color(png::ColorType::Grayscale);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(gray)
}

/// Atomically replace a text or data file without leaving a trailing open handle on user storage.
pub fn save(path: &Path, contents: &[u8]) -> io::Result<()> {
    with_storage(|| write_atomic(path, contents))
}

fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
    let mut temporary_name = name.to_owned();
    temporary_name.push(".tmp");
    let temporary = path.with_file_name(temporary_name);
    let mut file = File::create(&temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)
}

fn display_settings(launcher: &str, app: &str) -> Settings {
    display_settings_for(launcher, app, UNDER_LAUNCHER.load(Ordering::Relaxed))
}

fn display_settings_for(launcher: &str, app: &str, system_theme: bool) -> Settings {
    let mut settings = Settings::parse(launcher);
    for (key, value) in app.lines().filter_map(|line| line.split_once('=')) {
        match (key.trim(), value.trim()) {
            // Under the launcher the app's own dark_mode does not override the system theme.
            ("dark_mode", _) if system_theme => {}
            ("rotation", "upright") => settings.rotation = Rotation::Upright,
            ("rotation", "upside-down") => settings.rotation = Rotation::UpsideDown,
            ("auto_rotation", "on") => settings.auto_rotation = true,
            ("auto_rotation", "off") => settings.auto_rotation = false,
            ("dark_mode", "on") => settings.dark_mode = true,
            ("dark_mode", "off") => settings.dark_mode = false,
            _ => {}
        }
    }
    settings
}

/// Set by SIGUSR1 (the system layer's redraw signal, `docs/system-architecture.md` §4.1 in the umbrella repo).
static REDRAW: AtomicBool = AtomicBool::new(false);

extern "C" fn on_redraw(_: libc::c_int) {
    REDRAW.store(true, Ordering::Relaxed);
}

/// The swipe that opens quick settings under the launcher: down from the top edge, the launcher's own rule
/// (`opens_quick_settings` in kl-launcher; the status bar is 120 px, plus 40 px of slack).
fn is_system_swipe(input: Input) -> bool {
    matches!(input, Input::Swipe { dx, dy, start_y } if dy > 150.0 && dy.abs() > dx.abs() * 1.5 && start_y <= 160.0)
}

/// Under Primitive Launcher the app answers the redraw signal: SIGUSR1 marks `REDRAW`, and this thread wakes the
/// loop with `Event::Redraw`. Without the launcher the signal is left alone.
fn follow_redraw_signal(events: mpsc::Sender<Event>) {
    // SAFETY: the handler only stores to an atomic, which is safe in a signal handler.
    unsafe { libc::signal(libc::SIGUSR1, on_redraw as *const () as libc::sighandler_t) };
    // This thread only wakes the loop; the loop takes the request itself, before the event that woke it, so nothing
    // queued behind the freeze is drawn in the old theme first.
    thread::spawn(move || {
        let mut woken = false;
        loop {
            thread::sleep(Duration::from_millis(150));
            let pending = REDRAW.load(Ordering::Relaxed);
            if pending && !woken && events.send(Event::Redraw).is_err() {
                break;
            }
            woken = pending;
        }
    });
}

enum Event {
    Input(Input, Option<f32>),
    ViewportInput(input::viewport::ViewportEvent),
    Turn(Rotation),
    StorageChanged,
    UsbRedraw,
    /// Under Primitive Launcher: SIGUSR1, the launcher asks for the whole screen again in the current theme.
    Redraw,
}

fn publish_legacy_rotation(run: &Path, rotation: Rotation) {
    input::TURN.store(rotation.turn(), Ordering::Relaxed);
    // The supervisor's orientation file is in RAM and needs no flash lease.
    let _ = write_atomic(&run.join("orientation"), rotation.letter().as_bytes());
}

fn publish_presented_turn(publisher: &mut TurnPublisher, turn: Turn, success: bool) {
    if publisher.scoped() && success {
        input::TURN.store(turn.rotation.turn(), Ordering::Relaxed);
    }
    publisher.presented(turn, success);
}

fn inherited_turn(settings: &Settings, read_sensor: impl FnOnce() -> Option<Rotation>) -> Rotation {
    if settings.auto_rotation {
        read_sensor().unwrap_or(settings.rotation)
    } else {
        settings.rotation
    }
}

fn watch_turn(events: mpsc::Sender<Event>, initial: Rotation) {
    thread::spawn(move || {
        primitive_device::orientation_loop(initial, |turn| events.send(Event::Turn(turn)).is_ok());
    });
}

fn negotiated_viewport(
    app: &impl App,
    run: &Path,
    offer: Option<PathBuf>,
) -> io::Result<Option<ViewportSession>> {
    if offer.is_none() || app.viewport_request().is_none() {
        return Ok(None);
    }
    viewport::ViewportPublisher::new(run, offer)
        .map(|publisher| publisher.map(ViewportSession::new))
}

fn log_viewport(result: io::Result<()>) -> bool {
    if let Err(error) = result {
        kl_kindle::log(&format!("Viewport: {error}"));
        false
    } else {
        true
    }
}

/// Use the shared launcher settings and device driver while foregrounded by the launcher.
pub fn run(mut app: impl App, app_dir: &Path) -> Result<(), String> {
    if !cfg!(target_os = "linux") || !Path::new("/dev/fb0").exists() {
        return Err("Use this app's desktop target to simulate it on a computer.".into());
    }
    let launcher =
        std::env::var_os("KL_DIR").map_or_else(|| PathBuf::from("/mnt/us/launcher"), PathBuf::from);
    let run =
        std::env::var_os("KL_RUN").map_or_else(|| PathBuf::from("/var/tmp/kl"), PathBuf::from);
    let log_path = app_dir.join("log-app.txt");
    let log = |message: &str| {
        kl_kindle::log(message);
        let _ = with_storage(|| {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)?;
            writeln!(file, "{message}")
        });
    };
    let under_launcher = std::env::var("PRIMITIVE_LAUNCHER").as_deref() == Ok("1");
    UNDER_LAUNCHER.store(under_launcher, Ordering::Relaxed);
    let viewport_offer = under_launcher
        .then(|| std::env::var_os("KL_APP_VIEWPORT_FILE").map(PathBuf::from))
        .flatten();
    let mut viewport =
        negotiated_viewport(&app, &run, viewport_offer).map_err(|error| error.to_string())?;
    let turn_path = if under_launcher {
        std::env::var_os("KL_APP_TURN_FILE").map(PathBuf::from)
    } else {
        None
    };
    let mut turn_publisher = TurnPublisher::new(&run, turn_path);
    let mut settings = display_settings(
        &fs::read_to_string(launcher.join("settings.txt")).unwrap_or_default(),
        &fs::read_to_string(app_dir.join("settings.txt")).unwrap_or_default(),
    );
    let ram_fonts = run.join("fonts");
    let fonts_in_ram = ram_fonts.is_dir();
    let fonts = if fonts_in_ram {
        ram_fonts
    } else {
        launcher.join("fonts")
    };
    let fonts = font_paths(&fonts);
    let mut canvas = Canvas::new(WIDTH, HEIGHT, &fonts)?;
    let mut display = Display::open();

    let mut inherited_rotation = inherited_turn(&settings, device::accelerometer_turn);
    let mut turn = Turn::resolve(app.preferred_rotation(), inherited_rotation);
    settings.rotation = turn.rotation;
    let mut dark_override = app.preferred_dark_mode();
    if viewport.is_none() && !turn_publisher.scoped() {
        publish_legacy_rotation(&run, settings.rotation);
    }
    let (tx, rx) = mpsc::channel();
    if let Some(path) = input::find(profile::ACTIVE.touch_name) {
        let events = tx.clone();
        let generation = viewport.as_ref().map(ViewportSession::input_generation);
        thread::spawn(move || {
            if let Some(generation) = generation {
                input::viewport::viewport_touch_loop(path, &generation, |event| {
                    let _ = events.send(Event::ViewportInput(event));
                });
            } else {
                input::touch_loop_with_origin(path, |input, start_x| {
                    let origin = matches!(input, Input::Swipe { .. }).then_some(start_x);
                    let _ = events.send(Event::Input(input, origin));
                });
            }
        });
    }
    if let Some(path) = input::find("gpio-keys") {
        let events = tx.clone();
        let generation = viewport.as_ref().map(ViewportSession::input_generation);
        thread::spawn(move || {
            if let Some(generation) = generation {
                input::viewport::viewport_button_loop(path, &generation, |event| {
                    let _ = events.send(Event::ViewportInput(event));
                });
            } else {
                input::button_loop(path, |input| {
                    let _ = events.send(Event::Input(input, None));
                });
            }
        });
    }
    {
        let events = tx.clone();
        thread::spawn(move || {
            primitive_device::power_loop(|input| {
                let _ = events.send(Event::Input(input, None));
            });
        });
    }
    let mut dormant_turn_watch = None;
    if settings.auto_rotation {
        watch_turn(tx.clone(), inherited_rotation);
    } else if turn_publisher.scoped() || viewport.is_some() {
        dormant_turn_watch = Some(tx.clone());
    }
    log("Opened");
    if let Some(viewport) = &mut viewport {
        log_viewport(viewport.render_app(
            &mut app,
            &mut canvas,
            &mut display,
            &fonts,
            settings.dark_mode,
            inherited_rotation,
            Some(true),
        ));
    } else {
        let presented = display.present(
            frame(&app, &mut canvas, settings.dark_mode),
            &settings,
            true,
        );
        publish_presented_turn(&mut turn_publisher, turn, presented);
    }

    // A fallback font source may map a file from user storage, so keep the startup pause in that case.
    let managed_storage = fonts_in_ram
        && storage::enable(&run).unwrap_or_else(|error| {
            kl_kindle::log(&format!("Could not enable idle USB: {error}"));
            false
        });
    if managed_storage {
        let events = tx.clone();
        thread::spawn(move || watch_usb(events));
    }
    // Under Primitive Launcher the launcher owns sleep, the lock screen and the theme; it asks for a redraw.
    if under_launcher {
        follow_redraw_signal(tx.clone());
    }
    let mut redraw_on_wake = false;
    drop(tx);

    let mut lifecycle = Lifecycle::default();
    let on_save_error = |error| kl_kindle::log(&format!("Could not save: {error}"));
    loop {
        let app_deadline = lifecycle.deadline(&app);
        let viewport_deadline = viewport
            .as_ref()
            .filter(|_| lifecycle.is_awake())
            .and_then(|viewport| viewport.deadline(lifecycle.usb_drive));
        let event = match app_deadline.into_iter().chain(viewport_deadline).min() {
            Some(deadline) => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        // The launcher's redraw request (SIGUSR1, sent while the app was frozen) comes first: read the theme again
        // before handling what woke the loop, and draw the whole screen once.
        let mut redraw_now = false;
        if under_launcher && REDRAW.swap(false, Ordering::Relaxed) {
            if let Some(viewport) = &mut viewport {
                viewport.invalidate();
            }
            let launcher_settings = fs::read_to_string(launcher.join("settings.txt"));
            // Retain the last known Launcher settings while drive mode unmounts Documents.
            if launcher_settings.is_ok() || !(turn_publisher.scoped() || viewport.is_some()) {
                let fresh = display_settings(
                    &launcher_settings.unwrap_or_default(),
                    &fs::read_to_string(app_dir.join("settings.txt")).unwrap_or_default(),
                );
                settings.dark_mode = fresh.dark_mode;
                if turn_publisher.scoped() || viewport.is_some() {
                    settings.auto_rotation = fresh.auto_rotation;
                    inherited_rotation = inherited_turn(&fresh, device::accelerometer_turn);
                    if settings.auto_rotation {
                        if let Some(events) = dormant_turn_watch.take() {
                            watch_turn(events, inherited_rotation);
                        }
                    }
                } else if !settings.auto_rotation {
                    inherited_rotation = fresh.rotation;
                }
            }
            if lifecycle.is_awake() {
                redraw_now = true;
            } else {
                redraw_on_wake = true;
            }
        }
        let usb_drive = managed_storage && !user_storage_is_mounted();
        let usb_changed = lifecycle.set_usb_drive(usb_drive);
        if usb_changed {
            if let Some(viewport) = &mut viewport {
                viewport.invalidate();
            }
        }
        let usb_redraw = matches!(event, Ok(Event::UsbRedraw));
        let event = event.map(|event| match event {
            Event::ViewportInput(raw) => viewport
                .as_mut()
                .and_then(|viewport| viewport.resolve(raw, &display))
                .map_or(Event::Redraw, |resolved| {
                    Event::Input(resolved.input, resolved.touch_origin.map(|(x, _)| x))
                }),
            event => event,
        });
        let was_awake = lifecycle.is_awake();
        let mut pressed = false;
        let mut change = match event {
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout)
                if !usb_drive
                    && (viewport.is_none()
                        || app_deadline.is_some_and(|deadline| deadline <= Instant::now())) =>
            {
                app.tick(Instant::now())
            }
            // The swipe down from the top edge is the system's (quick settings) under the launcher, not the app's.
            Ok(Event::Input(input, _)) if under_launcher && is_system_swipe(input) => Change::None,
            Ok(Event::Input(input, start_x)) => {
                let mut feedback_ok = true;
                if !usb_changed && (viewport.is_some() || !turn_publisher.needs_frame(turn)) {
                    if let Some(image) =
                        lifecycle.press_feedback(&app, &mut canvas, input, settings.dark_mode)
                    {
                        if let Some(viewport) = &mut viewport {
                            feedback_ok =
                                log_viewport(viewport.present_current(&mut display, image, false));
                        } else {
                            display.show(image, &settings, false);
                        }
                        pressed = feedback_ok;
                    }
                }
                let change = if feedback_ok {
                    lifecycle.dispatch_input(&mut app, input, start_x, on_save_error)
                } else {
                    Change::None
                };
                // Under the launcher the screen is put back after the sleep picture: the app redraws on wake only
                // when a redraw was asked for while it slept.
                if under_launcher && input == Input::Wake && change == Change::Full {
                    if std::mem::take(&mut redraw_on_wake) { Change::Full } else { Change::None }
                } else {
                    change
                }
            }
            // Only a wake-up: the request was taken above.
            Ok(Event::Redraw) => Change::None,
            Ok(Event::Turn(rotation)) if settings.auto_rotation => {
                inherited_rotation = rotation;
                Change::None
            }
            _ => Change::None,
        };
        if redraw_now && change != Change::Exit {
            change = Change::Full;
        }
        if was_awake && !lifecycle.is_awake() {
            if let Some(viewport) = &mut viewport {
                viewport.invalidate();
            }
        }
        if usb_changed && !usb_drive {
            match app.storage_returned() {
                Ok(()) if storage::pending_save() => persist(&mut app, on_save_error),
                Ok(()) => {}
                Err(error) => kl_kindle::log(&format!("Could not reload Kindle storage: {error}")),
            }
        }
        if change != Change::Exit && viewport.is_none() {
            turn = Turn::resolve(app.preferred_rotation(), inherited_rotation);
            let old_rotation = settings.rotation;
            if turn.rotation != old_rotation {
                settings.rotation = turn.rotation;
                if !turn_publisher.scoped() {
                    publish_legacy_rotation(&run, turn.rotation);
                }
            }
            let preferred_dark = app.preferred_dark_mode();
            if lifecycle.is_awake() {
                if turn.rotation != old_rotation || preferred_dark != dark_override || turn_publisher.needs_frame(turn) {
                    change = Change::Full;
                } else {
                    turn_publisher.update_policy(turn);
                }
            }
            dark_override = preferred_dark;
        }
        if usb_drive {
            if lifecycle.is_awake()
                && (usb_changed
                    || usb_redraw
                    || change == Change::Full
                    || viewport.as_ref().is_some_and(ViewportSession::retry_due))
            {
                let image = canvas.usb_drive_frame().unwrap_or_else(|| {
                    canvas.begin();
                    full_screen_message(
                        &mut canvas,
                        "USB drive",
                        "Eject the Kindle from your computer to return to the app.",
                    );
                    canvas.finish(settings.dark_mode)
                });
                if let Some(viewport) = &mut viewport {
                    log_viewport(viewport.present_current(&mut display, image, true));
                } else {
                    let presented = display.present(image, &settings, true);
                    publish_presented_turn(&mut turn_publisher, turn, presented);
                }
            }
            continue;
        }
        let change = if lifecycle.is_awake() && usb_changed && change != Change::Exit {
            Change::Full
        } else {
            change
        };
        if let Some(viewport) = &mut viewport {
            if change == Change::Exit {
                break;
            }
            if lifecycle.is_awake() {
                let redraw = (change != Change::None || pressed)
                    .then_some(change == Change::Full && !redraw_now);
                log_viewport(viewport.render_app(
                    &mut app,
                    &mut canvas,
                    &mut display,
                    &fonts,
                    settings.dark_mode,
                    inherited_rotation,
                    redraw,
                ));
            }
            continue;
        }
        match change {
            Change::None if !pressed => {}
            Change::Exit => break,
            Change::None | Change::Partial | Change::Full => {
                let image = frame(&app, &mut canvas, settings.dark_mode);
                let presented = if redraw_now && change == Change::Full {
                    // The launcher flashed the screen when the theme changed; a second flash would show the old
                    // theme for a moment (the inverse of the new one).
                    display.present_calm(image, &settings)
                } else {
                    display.present(image, &settings, change == Change::Full)
                };
                publish_presented_turn(&mut turn_publisher, turn, presented);
            }
        }
    }
    lifecycle.close(&mut app, on_save_error);
    log("Closed");
    Ok(())
}

fn watch_usb(events: mpsc::Sender<Event>) {
    let mut attached = false;
    let mut redraw = false;
    loop {
        let current = !user_storage_is_mounted();
        let event = if current != attached {
            attached = current;
            redraw = current;
            Some(Event::StorageChanged)
        } else if redraw {
            // The stock USB screen can appear just after the first redraw.
            redraw = false;
            Some(Event::UsbRedraw)
        } else {
            None
        };
        if event.is_some_and(|event| events.send(event).is_err()) {
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_offer_never_queries_or_stages_viewport_support() {
        struct AppWithViewport;
        impl App for AppWithViewport {
            fn draw(&self, _: &mut Canvas) {}
            fn input(&mut self, _: Input) -> Change {
                Change::None
            }
            fn viewport_request(&self) -> Option<viewport::ViewportRequest> {
                panic!("an absent offer cannot enable the viewport lifecycle")
            }
        }
        let run = Path::new("/var/tmp/kl");
        assert!(negotiated_viewport(&AppWithViewport, run, None)
            .unwrap()
            .is_none());
        assert!(negotiated_viewport(
            &Demo::default(),
            run,
            Some(run.join("app-turn.ABC123/viewport"))
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn the_system_swipe_is_down_from_the_top_edge_only() {
        assert!(is_system_swipe(Input::Swipe { dx: 0.0, dy: 400.0, start_y: 30.0 }));
        assert!(!is_system_swipe(Input::Swipe { dx: 0.0, dy: 400.0, start_y: 700.0 }), "a move in the app");
        assert!(!is_system_swipe(Input::Swipe { dx: 0.0, dy: -400.0, start_y: 30.0 }), "up is the app's");
        assert!(!is_system_swipe(Input::Swipe { dx: 400.0, dy: 20.0, start_y: 30.0 }), "sideways is the app's");
    }

    #[derive(Default)]
    struct Demo {
        saves: usize,
        inputs: Vec<Input>,
        save_fails: bool,
        storage_unavailable: bool,
    }

    impl App for Demo {
        fn draw(&self, _canvas: &mut Canvas) {}

        fn input(&mut self, input: Input) -> Change {
            self.inputs.push(input);
            if input == Input::PageBack {
                Change::Exit
            } else {
                Change::Partial
            }
        }

        fn deadline(&self) -> Option<Instant> {
            Some(Instant::now())
        }

        fn save(&mut self) -> io::Result<()> {
            self.saves += 1;
            if self.storage_unavailable {
                Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "storage exported",
                ))
            } else if self.save_fails {
                Err(io::Error::other("disk full"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn lifecycle_saves_at_sleep_and_exit_and_suppresses_sleeping_input() {
        let mut demo = Demo::default();
        let mut lifecycle = Lifecycle::default();
        let on_error = |error| panic!("unexpected save failure: {error}");

        assert!(lifecycle.deadline(&demo).is_some());
        assert_eq!(
            lifecycle.input(&mut demo, Input::Sleep, on_error),
            Change::None
        );
        assert!(lifecycle.deadline(&demo).is_none());
        assert_eq!(demo.saves, 1);
        assert_eq!(
            lifecycle.input(&mut demo, Input::Sleep, on_error),
            Change::None
        );
        assert_eq!(
            lifecycle.input(&mut demo, Input::Tap(12.0, 12.0), on_error),
            Change::None
        );
        assert_eq!(
            lifecycle.input(&mut demo, Input::Wake, on_error),
            Change::Full
        );
        assert!(lifecycle.deadline(&demo).is_some());
        assert_eq!(
            lifecycle.input(&mut demo, Input::Wake, on_error),
            Change::None
        );
        assert_eq!(
            lifecycle.input(&mut demo, Input::PageBack, on_error),
            Change::Exit
        );
        lifecycle.close(&mut demo, on_error);
        assert_eq!(demo.saves, 2);
        assert_eq!(demo.inputs, [Input::Sleep, Input::Wake, Input::PageBack]);
    }

    #[test]
    fn lifecycle_reports_save_failure_and_completes_transition() {
        let mut demo = Demo {
            save_fails: true,
            ..Demo::default()
        };
        let mut lifecycle = Lifecycle::default();
        let mut errors = Vec::new();
        assert_eq!(
            lifecycle.input(&mut demo, Input::Sleep, |e| errors.push(e)),
            Change::None
        );
        assert!(!lifecycle.is_awake());
        assert_eq!(errors[0].to_string(), "disk full");
    }

    #[test]
    fn drive_mode_suspends_input_and_deadlines_but_preserves_power_events() {
        let mut demo = Demo::default();
        let mut lifecycle = Lifecycle::default();
        let on_error = |error| panic!("unexpected save failure: {error}");
        assert!(lifecycle.set_usb_drive(true));
        assert!(lifecycle.deadline(&demo).is_none());
        assert_eq!(
            lifecycle.input(&mut demo, Input::PageBack, on_error),
            Change::None
        );
        assert_eq!(
            lifecycle.input(&mut demo, Input::Sleep, on_error),
            Change::None
        );
        assert!(!lifecycle.is_awake());
        assert_eq!(
            lifecycle.input(&mut demo, Input::Wake, on_error),
            Change::Full
        );
        assert!(lifecycle.is_awake());
        assert!(lifecycle.deadline(&demo).is_none());
        assert_eq!(demo.inputs, [Input::Sleep, Input::Wake]);
        assert!(lifecycle.set_usb_drive(false));
        assert!(lifecycle.deadline(&demo).is_some());
    }

    #[test]
    fn swipes_reach_opted_in_apps_only_while_awake_and_storage_is_available() {
        #[derive(Default)]
        struct SwipeDemo {
            demo: Demo,
            received: Vec<(Input, f32)>,
        }

        impl App for SwipeDemo {
            fn draw(&self, _canvas: &mut Canvas) {}

            fn input(&mut self, input: Input) -> Change {
                self.demo.input(input)
            }

            fn swipe(&mut self, input: Input, start_x: f32) -> Change {
                self.received.push((input, start_x));
                Change::Exit
            }

            fn save(&mut self) -> io::Result<()> {
                self.demo.save()
            }
        }

        let gesture = Input::Swipe {
            dx: 20.0,
            dy: 250.0,
            start_y: 34.0,
        };
        let on_error = |error| panic!("unexpected save failure: {error}");
        let mut lifecycle = Lifecycle::default();
        let mut normal = Demo::default();
        assert_eq!(
            lifecycle.swipe(&mut normal, gesture, 54.0, on_error),
            Change::Partial
        );
        assert_eq!(normal.inputs, [gesture]);

        let mut app = SwipeDemo::default();
        assert_eq!(
            lifecycle.input(&mut app, Input::Sleep, on_error),
            Change::None
        );
        assert_eq!(
            lifecycle.swipe(&mut app, gesture, 100.0, on_error),
            Change::None
        );
        assert_eq!(
            lifecycle.input(&mut app, Input::Wake, on_error),
            Change::Full
        );
        lifecycle.set_usb_drive(true);
        assert_eq!(
            lifecycle.swipe(&mut app, gesture, 200.0, on_error),
            Change::None
        );
        assert!(app.received.is_empty());

        lifecycle.set_usb_drive(false);
        assert_eq!(
            lifecycle.swipe(&mut app, gesture, 320.0, on_error),
            Change::Exit
        );
        assert_eq!(app.received, [(gesture, 320.0)]);
        assert_eq!(app.demo.inputs, [Input::Sleep, Input::Wake]);
        assert_eq!(app.demo.saves, 2);
        assert_eq!(
            lifecycle.swipe(&mut app, gesture, 430.0, on_error),
            Change::None
        );
        assert_eq!(app.received.len(), 1);
    }

    #[test]
    fn an_exit_save_losing_storage_keeps_the_session_alive() {
        let mut demo = Demo {
            storage_unavailable: true,
            ..Demo::default()
        };
        let mut lifecycle = Lifecycle::default();
        let on_error = |error: io::Error| assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            lifecycle.input(&mut demo, Input::PageBack, on_error),
            Change::None
        );
        assert!(lifecycle.is_awake());
        demo.storage_unavailable = false;
        assert_eq!(
            lifecycle.input(&mut demo, Input::PageBack, on_error),
            Change::Exit
        );
        assert!(!lifecycle.is_awake());
        assert_eq!(demo.saves, 2);
    }

    #[test]
    fn under_the_launcher_the_system_theme_wins() {
        let launcher = "dark_mode=on";
        assert!(display_settings_for(launcher, "dark_mode=off", true).dark_mode, "the app's own setting is ignored");
        assert!(!display_settings_for(launcher, "dark_mode=off", false).dark_mode, "on its own it still counts");
        assert!(display_settings_for("dark_mode=off", "rotation=upside-down", true).rotation == Rotation::UpsideDown);
        assert!(effective_dark(Some(false), true, true), "a preference does not override the system theme");
        assert!(!effective_dark(Some(false), true, false));
        assert!(effective_dark(None, true, false));
    }

    #[test]
    fn inherited_turn_uses_manual_settings_or_the_current_sensor() {
        let mut settings = Settings {
            rotation: Rotation::UpsideDown,
            auto_rotation: false,
            ..Settings::default()
        };
        assert_eq!(inherited_turn(&settings, || panic!("manual orientation has no sensor read")), Rotation::UpsideDown);
        settings.auto_rotation = true;
        assert_eq!(inherited_turn(&settings, || Some(Rotation::Upright)), Rotation::Upright);
        assert_eq!(inherited_turn(&settings, || None), Rotation::UpsideDown);
    }

    #[test]
    fn per_app_settings_override_only_documented_display_preferences() {
        let launcher = "rotation=upside-down\nauto_rotation=off\ndark_mode=on\nstore_daily_check=0";
        let inherited = display_settings(launcher, "");
        assert_eq!(inherited.rotation, Rotation::UpsideDown);
        assert!(inherited.dark_mode);
        assert!(!inherited.auto_rotation);
        let overridden = display_settings(
            launcher,
            "rotation=upright\nauto_rotation=on\ndark_mode=invalid\nstore_daily_check=1",
        );
        assert_eq!(overridden.rotation, Rotation::Upright);
        assert!(overridden.auto_rotation);
        assert!(overridden.dark_mode);
        assert!(!overridden.store_daily_check);
        assert!(!display_settings(launcher, "dark_mode=off").dark_mode);
    }
}
