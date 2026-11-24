//! Touch and page-button readers (evdev), adapted from Primitive Browser 0.2.12
//! (`crates/kb-kindle/src/main.rs`). Each runs on its own thread and sends gestures in screen
//! coordinates, already turned for the launcher's rotation. Viewport-aware callers can
//! opt into native coordinates and generation-tagged events.

pub mod viewport;

use std::{
    fs::{self, File},
    io::Read,
    os::unix::io::AsRawFd,
    sync::atomic::{AtomicI32, AtomicU32, Ordering},
    time::{Duration, Instant},
};

pub use kl_core::Input;
use primitive_device::{log, profile};
use viewport::{NativeTouch, TouchPhase};

/// The turn the launcher draws in: 0 upright, 2 upside down. Touch and buttons follow it.
pub static TURN: AtomicU32 = AtomicU32::new(0);

const NATIVE_W: u32 = profile::ACTIVE.screen_w;
const NATIVE_H: u32 = profile::ACTIVE.screen_h;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const KEY_PAGEUP: u16 = profile::ACTIVE.page_back_key;
const KEY_PAGEDOWN: u16 = profile::ACTIVE.page_forward_key;
const HOLD_START: Duration = Duration::from_millis(800);
const HOLD_PAN_INTERVAL: Duration = Duration::from_millis(200);
const HOLD_PAN_MIN_DISTANCE: f32 = 6.0;

/// Turn a point from the panel's native (upright) frame into the screen's frame.
pub fn turn_point(turn: u32, x: f32, y: f32) -> (f32, f32) {
    if turn == 2 {
        (NATIVE_W as f32 - 1.0 - x, NATIVE_H as f32 - 1.0 - y)
    } else {
        (x, y)
    }
}

/// `_IOR('E', 0x40 + code, struct input_absinfo)` for the 24-byte absinfo.
const fn eviocgabs(code: u16) -> libc::c_ulong {
    (0x8000_0000u32 | (24 << 16) | (0x45 << 8) | (0x40 + code as u32)) as libc::c_ulong
}

#[repr(C)]
#[derive(Default)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

/// The kernel's `struct input_event` on the 32-bit Kindle kernel.
#[repr(C)]
struct InputEvent {
    tv_sec: i32,
    tv_usec: i32,
    kind: u16,
    code: u16,
    value: i32,
}

#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<InputEvent>() == 16);

/// Find an input device by part of its kernel name (e.g. "cyttsp", "gpio-keys").
pub fn find(name_hint: &str) -> Option<String> {
    for entry in fs::read_dir("/sys/class/input").ok()?.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("event") {
            continue;
        }
        let name = fs::read_to_string(entry.path().join("device/name")).unwrap_or_default();
        if name.trim().contains(name_hint) {
            return Some(format!("/dev/input/{fname}"));
        }
    }
    None
}

fn abs_range(file: &File, code: u16) -> Option<AbsInfo> {
    let mut info = AbsInfo::default();
    // SAFETY: AbsInfo is plain data of the size encoded in the request.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), eviocgabs(code) as _, &mut info as *mut AbsInfo) };
    (rc == 0).then_some(info)
}

fn scale(raw: i32, info: &AbsInfo, out: u32) -> f32 {
    let span = (info.maximum - info.minimum).max(1) as f32;
    ((raw - info.minimum) as f32 / span * (out as f32 - 1.0)).clamp(0.0, out as f32 - 1.0)
}

fn read_event(file: &mut File) -> Option<InputEvent> {
    let mut buf = [0u8; std::mem::size_of::<InputEvent>()];
    file.read_exact(&mut buf).ok()?;
    // SAFETY: InputEvent is repr(C) plain data and buf has its size.
    Some(unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const InputEvent) })
}

fn wait_for_event(file: &File, timeout: Option<Duration>) -> std::io::Result<bool> {
    let timeout_ms = timeout.map_or(-1, |duration| {
        duration.as_millis().min(i32::MAX as u128) as i32
    });
    let mut descriptor = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: descriptor is a valid, initialized pollfd for the open input file.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if result == 0 {
            return Ok(false);
        }
        if result > 0 {
            return Ok(descriptor.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn held_milliseconds(duration: Duration) -> u32 {
    duration.as_millis().min(u128::from(u32::MAX)) as u32
}

fn is_hold_threshold_reached(duration: Duration) -> bool {
    duration >= HOLD_START
}

/// The gesture for a finished touch: from its start and end points and how long it was held.
pub fn gesture(start: (f32, f32), end: (f32, f32), held: Duration) -> Input {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let still = dx.abs() < 40.0 && dy.abs() < 40.0;
    if still && held < Duration::from_millis(800) {
        Input::Tap(start.0, start.1)
    } else if still {
        Input::LongTap(start.0, start.1)
    } else {
        Input::Swipe { dx, dy, start_y: start.1 }
    }
}

fn page_button(code: u16, turn: u32) -> Option<Input> {
    page_button_for(code, turn == 2)
}

fn page_button_for(code: u16, swapped: bool) -> Option<Input> {
    match (code, swapped) {
        (KEY_PAGEDOWN, false) | (KEY_PAGEUP, true) => Some(Input::PageForward),
        (KEY_PAGEUP, false) | (KEY_PAGEDOWN, true) => Some(Input::PageBack),
        _ => None,
    }
}

/// The open touch device, for `grab_touch` (-1 until the touch loop opens it).
static TOUCH_FD: AtomicI32 = AtomicI32::new(-1);

/// `EVIOCGRAB`: `_IOW('E', 0x90, int)`.
const EVIOCGRAB: u32 = 0x4004_4590;

/// Take the touch screen for this process only (`true`), or give it back (`false`). While it is taken, no other
/// process gets touches: the system panel over an app keeps the app from seeing the taps meant for the panel.
/// Returns whether the device took the request.
pub fn grab_touch(on: bool) -> bool {
    let fd = TOUCH_FD.load(Ordering::Relaxed);
    if fd < 0 {
        return false;
    }
    // SAFETY: EVIOCGRAB takes an int by value on an evdev descriptor this process opened and keeps open.
    unsafe { libc::ioctl(fd, EVIOCGRAB as _, on as libc::c_int) == 0 }
}

/// Read touches from the device and report completed gestures.
pub fn touch_loop(path: String, mut report: impl FnMut(Input)) {
    touch_loop_with_origin(path, |input, _| report(input));
}

/// Report completed gestures together with their horizontal start in screen coordinates.
pub fn touch_loop_with_origin(path: String, mut report: impl FnMut(Input, f32)) {
    read_touches(
        path,
        || (),
        || TURN.load(Ordering::Relaxed),
        |touch, (), turn| {
            let (input, origin) = touch.reduce(|(x, y)| turn_point(turn, x, y));
            report(input, origin.0);
        },
    );
}

struct Contact<Generation> {
    generation: Generation,
    pressed_at: Instant,
    first: Option<((f32, f32), u32)>,
    last: (f32, f32),
    hold_started: bool,
    last_pan: (f32, f32),
    last_pan_at: Instant,
}

impl<Generation: Copy> Contact<Generation> {
    fn new(pressed_at: Instant, generation: Generation) -> Self {
        Self {
            generation,
            pressed_at,
            first: None,
            last: (0.0, 0.0),
            hold_started: false,
            last_pan: (0.0, 0.0),
            last_pan_at: pressed_at,
        }
    }

    fn timeout(&self, now: Instant) -> Option<Duration> {
        (self.first.is_some() && !self.hold_started)
            .then(|| HOLD_START.saturating_sub(now.duration_since(self.pressed_at)))
    }

    fn emit(&self, phase: TouchPhase, report: &mut impl FnMut(NativeTouch, Generation, u32)) {
        if let Some((start, turn)) = self.first {
            let touch = NativeTouch {
                start,
                end: self.last,
                phase,
            };
            report(touch, self.generation, turn);
        }
    }

    fn last_in_legacy_view(&self) -> (f32, f32) {
        let turn = self.first.map_or(0, |(_, turn)| turn);
        turn_point(turn, self.last.0, self.last.1)
    }

    fn moved_since_last_pan(&self, distance: f32) -> bool {
        let (x, y) = self.last_in_legacy_view();
        (x - self.last_pan.0).abs() >= distance || (y - self.last_pan.1).abs() >= distance
    }

    fn idle(&mut self, now: Instant, report: &mut impl FnMut(NativeTouch, Generation, u32)) {
        if !self.hold_started && self.first.is_some() {
            self.emit(TouchPhase::HoldStart, report);
            self.hold_started = true;
            self.last_pan = self.last_in_legacy_view();
            self.last_pan_at = now;
        }
    }

    fn sample(
        &mut self,
        point: (f32, f32),
        now: Instant,
        first_turn: impl FnOnce() -> u32,
        report: &mut impl FnMut(NativeTouch, Generation, u32),
    ) {
        self.last = point;
        if self.first.is_none() {
            self.first = Some((point, first_turn()));
        } else if self.hold_started
            && self.moved_since_last_pan(HOLD_PAN_MIN_DISTANCE)
            && now.duration_since(self.last_pan_at) >= HOLD_PAN_INTERVAL
        {
            self.emit(TouchPhase::HoldPan, report);
            self.last_pan = self.last_in_legacy_view();
            self.last_pan_at = now;
        }
    }

    fn finish(self, now: Instant, report: &mut impl FnMut(NativeTouch, Generation, u32)) {
        let held = now.duration_since(self.pressed_at);
        if self.hold_started || is_hold_threshold_reached(held) {
            if !self.hold_started {
                self.emit(TouchPhase::HoldStart, report);
            }
            if self.moved_since_last_pan(1.0) {
                self.emit(TouchPhase::HoldPan, report);
            }
            self.emit(
                TouchPhase::HoldRelease {
                    held_ms: held_milliseconds(held),
                },
                report,
            );
        } else {
            self.emit(TouchPhase::Finished(held), report);
        }
    }
}

fn read_touches<Generation: Copy>(
    path: String,
    mut on_press: impl FnMut() -> Generation,
    mut first_turn: impl FnMut() -> u32,
    mut report: impl FnMut(NativeTouch, Generation, u32),
) {
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return log(&format!("cannot open {path}: {e}")),
    };
    let (Some(xi), Some(yi)) = (
        abs_range(&file, ABS_MT_POSITION_X),
        abs_range(&file, ABS_MT_POSITION_Y),
    ) else {
        return log("touch ranges unavailable");
    };
    log(&format!(
        "touch {path}: x {}..{} y {}..{}",
        xi.minimum, xi.maximum, yi.minimum, yi.maximum
    ));
    TOUCH_FD.store(file.as_raw_fd(), Ordering::Relaxed);
    let (mut raw_x, mut raw_y) = (0i32, 0i32);
    let mut contact: Option<Contact<Generation>> = None;
    loop {
        let timeout = contact
            .as_ref()
            .and_then(|touch| touch.timeout(Instant::now()));
        let ready = match wait_for_event(&file, timeout) {
            Ok(ready) => ready,
            Err(error) => return log(&format!("touch poll failed: {error}")),
        };
        if !ready {
            if let Some(touch) = contact.as_mut() {
                touch.idle(Instant::now(), &mut report);
            }
            continue;
        }
        let Some(ev) = read_event(&mut file) else {
            return log("touch read failed");
        };
        match (ev.kind, ev.code) {
            (EV_ABS, ABS_MT_TRACKING_ID) if ev.value >= 0 => {
                contact = Some(Contact::new(Instant::now(), on_press()));
            }
            (EV_ABS, ABS_MT_TRACKING_ID) => {
                if let Some(touch) = contact.take() {
                    touch.finish(Instant::now(), &mut report);
                }
            }
            (EV_ABS, ABS_MT_POSITION_X) => raw_x = ev.value,
            (EV_ABS, ABS_MT_POSITION_Y) => raw_y = ev.value,
            (EV_SYN, SYN_REPORT) => {
                if let Some(touch) = contact.as_mut() {
                    let point = (scale(raw_x, &xi, NATIVE_W), scale(raw_y, &yi, NATIVE_H));
                    touch.sample(point, Instant::now(), &mut first_turn, &mut report);
                }
            }
            _ => {}
        }
    }
}

/// Page buttons: presses only. Upside down, forward and back trade places.
pub fn button_loop(path: String, mut report: impl FnMut(Input)) {
    read_buttons(path, |code| {
        if let Some(input) = page_button(code, TURN.load(Ordering::Relaxed)) {
            report(input);
        }
    });
}

fn read_buttons(path: String, mut report: impl FnMut(u16)) {
    read_buttons_with_clock(path, false, |code, _| report(code));
}

fn read_buttons_with_clock(path: String, timed: bool, mut report: impl FnMut(u16, Option<u64>)) {
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return log(&format!("cannot open {path}: {e}")),
    };
    log(&format!("buttons {path}"));
    let monotonic = timed && use_monotonic_events(&file);
    if timed && !monotonic {
        log("page key timestamps are unavailable; Launcher overlay key filtering is disabled");
    }
    loop {
        let Some(ev) = read_event(&mut file) else {
            return log("button read failed");
        };
        if ev.kind != EV_KEY || ev.value != 1 {
            continue;
        }
        if matches!(ev.code, KEY_PAGEUP | KEY_PAGEDOWN) {
            let time = monotonic.then(|| monotonic_micros(ev.tv_sec.into(), ev.tv_usec.into())).flatten();
            if monotonic && time.is_none() {
                continue;
            }
            report(ev.code, time);
        }
    }
}

fn use_monotonic_events(file: &File) -> bool {
    // _IOW('E', 0xa0, int) selects the clock for only this evdev descriptor.
    const EVIOCSCLOCKID: u32 = 0x4004_45a0;
    let clock = libc::CLOCK_MONOTONIC;
    // SAFETY: the kernel reads one initialized c_int from a live pointer.
    unsafe { libc::ioctl(file.as_raw_fd(), EVIOCSCLOCKID as _, &clock) == 0 }
}

fn monotonic_micros(seconds: i64, micros: i64) -> Option<u64> {
    if !(0..1_000_000).contains(&micros) {
        return None;
    }
    u64::try_from(seconds).ok()?.checked_mul(1_000_000)?.checked_add(micros as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evdev_monotonic_microseconds_include_only_valid_timevals() {
        assert_eq!(monotonic_micros(24, 123), Some(24_000_123));
        assert_eq!(monotonic_micros(-1, 123), None);
        assert_eq!(monotonic_micros(3, -1), None);
        assert_eq!(monotonic_micros(3, 1_000_000), None);
        assert_eq!(monotonic_micros(i64::MAX, 1), None);
    }

    #[test]
    fn upside_down_turns_touches() {
        assert_eq!(turn_point(0, 10.0, 20.0), (10.0, 20.0));
        assert_eq!(turn_point(2, 0.0, 0.0), (1263.0, 1679.0));
    }

    #[test]
    fn page_buttons_follow_the_screen_turn() {
        assert_eq!(page_button(KEY_PAGEUP, 0), Some(Input::PageBack));
        assert_eq!(page_button(KEY_PAGEUP, 2), Some(Input::PageForward));
        assert_eq!(page_button(KEY_PAGEDOWN, 0), Some(Input::PageForward));
        assert_eq!(page_button(KEY_PAGEDOWN, 2), Some(Input::PageBack));
        assert_eq!(page_button(1, 0), None);
    }

    #[test]
    fn gestures_by_distance_and_time() {
        let short = Duration::from_millis(100);
        assert_eq!(gesture((10.0, 10.0), (20.0, 15.0), short), Input::Tap(10.0, 10.0));
        assert_eq!(gesture((10.0, 10.0), (20.0, 15.0), Duration::from_secs(1)), Input::LongTap(10.0, 10.0));
        assert_eq!(gesture((500.0, 10.0), (100.0, 30.0), short), Input::Swipe { dx: -400.0, dy: 20.0, start_y: 10.0 });
    }

    #[test]
    fn legacy_touch_latches_its_turn_at_the_first_sample_and_keeps_horizontal_origin() {
        let pressed = Instant::now();
        let mut touch = Contact::new(pressed, ());
        let mut events = Vec::new();
        let mut report = |native: NativeTouch, (), turn| {
            events.push(native.reduce(|(x, y)| turn_point(turn, x, y)));
        };
        touch.sample((10.0, 20.0), pressed, || 2, &mut report);
        touch.sample(
            (10.0, 220.0),
            pressed + Duration::from_millis(200),
            || 0,
            &mut report,
        );
        touch.finish(pressed + Duration::from_millis(300), &mut report);
        let (start_x, start_y) = turn_point(2, 10.0, 20.0);
        assert_eq!(
            events,
            [(
                Input::Swipe {
                    dx: 0.0,
                    dy: -200.0,
                    start_y
                },
                (start_x, start_y)
            )]
        );
    }

    #[test]
    fn held_selection_starts_at_the_long_press_threshold() {
        assert!(!is_hold_threshold_reached(Duration::from_millis(799)));
        assert!(is_hold_threshold_reached(Duration::from_millis(800)));
        assert_eq!(held_milliseconds(Duration::from_millis(1_234)), 1_234);
        assert_eq!(HOLD_PAN_INTERVAL, Duration::from_millis(200));
        assert_eq!(HOLD_PAN_MIN_DISTANCE, 6.0);
    }
}
