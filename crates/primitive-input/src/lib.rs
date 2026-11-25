//! Touch and page-button readers (evdev), adapted from Primitive Browser 0.2.12
//! (`crates/kb-kindle/src/main.rs`). Each runs on its own thread and sends gestures in screen
//! coordinates, already turned for the launcher's rotation.

use std::{
    fs::{self, File},
    io::Read,
    os::unix::io::AsRawFd,
    sync::atomic::{AtomicI32, AtomicU32, Ordering},
    time::{Duration, Instant},
};

pub use kl_core::Input;
use primitive_device::{log, profile};

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
    match (code, turn == 2) {
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
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return log(&format!("cannot open {path}: {e}")),
    };
    let (Some(xi), Some(yi)) = (abs_range(&file, ABS_MT_POSITION_X), abs_range(&file, ABS_MT_POSITION_Y)) else {
        return log("touch ranges unavailable");
    };
    log(&format!("touch {path}: x {}..{} y {}..{}", xi.minimum, xi.maximum, yi.minimum, yi.maximum));
    TOUCH_FD.store(file.as_raw_fd(), Ordering::Relaxed);
    let (mut raw_x, mut raw_y) = (0i32, 0i32);
    let mut active = false;
    let mut start: Option<((f32, f32), u32)> = None;
    let mut last = (0.0f32, 0.0f32);
    let mut down_at = Instant::now();
    let mut hold_started = false;
    let mut last_pan = (0.0f32, 0.0f32);
    let mut last_pan_at = down_at;
    loop {
        let timeout = (active && start.is_some() && !hold_started)
            .then(|| HOLD_START.saturating_sub(down_at.elapsed()));
        let ready = match wait_for_event(&file, timeout) {
            Ok(ready) => ready,
            Err(error) => return log(&format!("touch poll failed: {error}")),
        };
        if !ready {
            if active && !hold_started {
                if let Some((point, _)) = start {
                    report(Input::HoldStart(point.0, point.1), point.0);
                    hold_started = true;
                    last_pan = last;
                    last_pan_at = Instant::now();
                }
            }
            continue;
        }
        let Some(ev) = read_event(&mut file) else {
            return log("touch read failed");
        };
        match (ev.kind, ev.code) {
            (EV_ABS, ABS_MT_TRACKING_ID) => {
                if ev.value >= 0 {
                    active = true;
                    start = None;
                    down_at = Instant::now();
                    hold_started = false;
                    last_pan = (0.0, 0.0);
                    last_pan_at = down_at;
                } else if active {
                    active = false;
                    if let Some((point, _)) = start {
                        let held = down_at.elapsed();
                        if hold_started || is_hold_threshold_reached(held) {
                            if !hold_started {
                                report(Input::HoldStart(point.0, point.1), point.0);
                                hold_started = true;
                            }
                            if (last.0 - last_pan.0).abs() >= 1.0
                                || (last.1 - last_pan.1).abs() >= 1.0
                            {
                                report(Input::HoldPan(last.0, last.1), point.0);
                            }
                            report(
                                Input::HoldRelease {
                                    start_x: point.0,
                                    start_y: point.1,
                                    x: last.0,
                                    y: last.1,
                                    held_ms: held_milliseconds(held),
                                },
                                point.0,
                            );
                        } else {
                            report(gesture(point, last, held), point.0);
                        }
                    }
                }
            }
            (EV_ABS, ABS_MT_POSITION_X) => raw_x = ev.value,
            (EV_ABS, ABS_MT_POSITION_Y) => raw_y = ev.value,
            (EV_SYN, SYN_REPORT) if active => {
                // Use a single orientation for the gesture if the device turns mid-touch.
                let turn = start.map_or_else(|| TURN.load(Ordering::Relaxed), |(_, turn)| turn);
                last = turn_point(turn, scale(raw_x, &xi, NATIVE_W), scale(raw_y, &yi, NATIVE_H));
                if start.is_none() {
                    start = Some((last, turn));
                } else if hold_started
                    && ((last.0 - last_pan.0).abs() >= HOLD_PAN_MIN_DISTANCE
                        || (last.1 - last_pan.1).abs() >= HOLD_PAN_MIN_DISTANCE)
                    && last_pan_at.elapsed() >= HOLD_PAN_INTERVAL
                {
                    report(
                        Input::HoldPan(last.0, last.1),
                        start.map_or(last.0, |(point, _)| point.0),
                    );
                    last_pan = last;
                    last_pan_at = Instant::now();
                }
            }
            _ => {}
        }
    }
}

/// Page buttons: presses only. Upside down, forward and back trade places.
/// Like `button_loop`, and also reports every press (`down` true) and release (`down` false) of each key, so a launcher
/// can time a hold of two keys. Autorepeat (value 2) is not reported.
pub fn button_loop_keys(path: String, mut report: impl FnMut(Input), mut key: impl FnMut(u16, bool)) {
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return log(&format!("cannot open {path}: {e}")),
    };
    log(&format!("buttons {path}"));
    loop {
        let Some(ev) = read_event(&mut file) else {
            return log("button read failed");
        };
        if ev.kind != EV_KEY || ev.value == 2 {
            continue;
        }
        key(ev.code, ev.value == 1);
        if ev.value == 1 {
            if let Some(input) = page_button(ev.code, TURN.load(Ordering::Relaxed)) {
                report(input);
            }
        }
    }
}

pub fn button_loop(path: String, mut report: impl FnMut(Input)) {
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return log(&format!("cannot open {path}: {e}")),
    };
    log(&format!("buttons {path}"));
    loop {
        let Some(ev) = read_event(&mut file) else {
            return log("button read failed");
        };
        if ev.kind != EV_KEY || ev.value != 1 {
            continue;
        }
        if let Some(input) = page_button(ev.code, TURN.load(Ordering::Relaxed)) {
            report(input);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn held_selection_starts_at_the_long_press_threshold() {
        assert!(!is_hold_threshold_reached(Duration::from_millis(799)));
        assert!(is_hold_threshold_reached(Duration::from_millis(800)));
        assert_eq!(held_milliseconds(Duration::from_millis(1_234)), 1_234);
        assert_eq!(HOLD_PAN_INTERVAL, Duration::from_millis(200));
        assert_eq!(HOLD_PAN_MIN_DISTANCE, 6.0);
    }
}
