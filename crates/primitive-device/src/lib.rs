//! The Kindle's controls through the property bus (`lipc-get-prop` / `lipc-set-prop`), the power
//! events, and a few files from /proc and /sys. Property names follow Primitive Browser 0.2.12
//! (`crates/kb-kindle/src/device.rs`), which uses the ones KOReader uses on this firmware. Nothing here
//! is fatal: a value that cannot be read shows as unknown.

use std::{
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use kl_core::{battery::{Reading, Source}, rotation::Debouncer};
pub use kl_core::{Input, Rotation, Settings, Status, Usb};

pub mod profile;

pub fn log(msg: &str) {
    eprintln!("[kl] {msg}");
}

const POWERD: &str = "com.lab126.powerd";
const WIFID: &str = "com.lab126.wifid";
const CMD: &str = "com.lab126.cmd";
pub const LEVEL_MAX: u32 = 24;
pub const NTPDATE: &str = "/usr/bin/ntpdate";

fn get_int(service: &str, prop: &str) -> Option<u32> {
    let out = Command::new("lipc-get-prop").args(["-i", service, prop]).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn get_text(service: &str, prop: &str) -> String {
    Command::new("lipc-get-prop")
        .args([service, prop])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn set_int(service: &str, prop: &str, value: u32) -> bool {
    let ok = Command::new("lipc-set-prop")
        .args(["-i", service, prop, &value.to_string()])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    log(&format!("set {service} {prop} {value}: {}", if ok { "ok" } else { "failed" }));
    ok
}

/// Refresh the active idle timer. Leave the persistent screensaver setting alone.
pub fn refresh_screen_timeout() -> bool {
    if may_refresh_screen_timeout(
        &get_text(POWERD, "state"),
        get_int(POWERD, "preventScreenSaver"),
        get_int(POWERD, "isCharging"),
    ) {
        set_int(POWERD, "touchScreenSaverTimeout", 1)
    } else {
        false
    }
}

fn may_refresh_screen_timeout(state: &str, inhibited: Option<u32>, charging: Option<u32>) -> bool {
    state.eq_ignore_ascii_case("active") && inhibited == Some(0) && charging == Some(0)
}

fn sysfs_text(path: &str) -> String {
    std::fs::read_to_string(path).map(|t| t.trim().to_string()).unwrap_or_default()
}

/// The USB state: the cable (the charger's `online`) and whether the storage partition is mounted on the Kindle. That
/// partition is `/mnt/base-us` (vfat); `/mnt/us` is a FUSE layer over it that stays mounted. While the drive is on a
/// computer, `/mnt/base-us` is not mounted.
pub fn usb_state() -> kl_core::Usb {
    let cable_in = sysfs_text(profile::ACTIVE.charger_online) == "1";
    let mounted = std::fs::read_to_string("/proc/mounts")
        .map(|m| m.lines().any(|l| l.split_whitespace().nth(1) == Some("/mnt/base-us")))
        .unwrap_or(true);
    kl_core::Usb::from_device(cable_in, mounted)
}

/// The local time and date, e.g. "10:38" and "Mon 16 Nov".
pub fn local_time() -> (String, String, i32) {
    let reading = read_clock();
    (reading.time, reading.date, reading.minute)
}

struct ClockReading {
    time: String,
    date: String,
    minute: i32,
    year: Option<i32>,
    utc_offset: Option<i32>,
}

fn read_clock() -> ClockReading {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    // SAFETY: `time`, `gmtime_r` and `localtime_r` only write into the values passed here.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        // The zone offset comes from /etc/localtime directly: musl reads the Kindle's file as UTC
        // (kl_core::tz). localtime_r is the fallback for a file that cannot be read.
        let offset = std::fs::read("/etc/localtime").ok().and_then(|d| kl_core::tz::offset_at(&d, now as i64));
        let done = match offset {
            Some(off) => {
                let shifted = now.saturating_add(off as _);
                !libc::gmtime_r(&shifted, &mut tm).is_null()
            }
            None => !libc::localtime_r(&now, &mut tm).is_null(),
        };
        if !done {
            return ClockReading { time: "--:--".into(), date: String::new(), minute: -1, year: None, utc_offset: None };
        }
        let clock = format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
        let day = DAYS.get(tm.tm_wday as usize).copied().unwrap_or("");
        let month = MONTHS.get(tm.tm_mon as usize).copied().unwrap_or("");
        ClockReading {
            time: clock,
            date: format!("{day} {} {month}", tm.tm_mday),
            minute: tm.tm_hour * 60 + tm.tm_min,
            year: tm.tm_year.checked_add(1900),
            utc_offset: offset,
        }
    }
}

/// Refresh only the time settings, without waiting for device property calls.
pub fn refresh_clock(status: &mut Status) {
    let reading = read_clock();
    status.clock = reading.time;
    status.date = reading.date;
    status.clock_year = reading.year;
    status.utc_offset = reading.utc_offset;
}

/// The battery level as a percentage, if it can be read.
pub fn battery() -> Option<u32> {
    let valid = |value: &u32| *value <= 100;
    get_int(POWERD, "battLevel").filter(valid).or_else(|| {
        sysfs_text(profile::ACTIVE.battery_capacity).parse().ok().filter(valid)
    })
}

/// Whether the Kindle is charging.
pub fn charging() -> bool {
    power_source() == Source::Charging
}

pub fn battery_reading() -> Reading {
    Reading { percent: battery(), source: power_source() }
}

fn power_source() -> Source {
    let bit = |value| match value {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    };
    classify_power_source(
        bit(sysfs_text(profile::ACTIVE.charger_online).parse().ok()),
        [
            bit(get_int(POWERD, "isCharging")),
            bit(sysfs_text(profile::ACTIVE.charger_charging).parse().ok()),
        ],
        &sysfs_text(profile::ACTIVE.charger_status),
    )
}

fn classify_power_source(online: Option<bool>, charging: [Option<bool>; 2], status: &str) -> Source {
    if charging.contains(&Some(true)) || status == "Charging" {
        Source::Charging
    } else if online == Some(false) || (online.is_none() && status == "Discharging") {
        Source::OnBattery
    } else if online == Some(true) && status == "Full" {
        Source::Full
    } else if online == Some(true) && (charging.contains(&Some(false)) || status == "Not charging") {
        Source::PluggedIn
    } else {
        Source::Unknown
    }
}

fn mem_available() -> String {
    let info = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    info.lines()
        .find(|l| l.starts_with("MemAvailable:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| format!("MemAvailable {} MB", kb / 1024))
        .unwrap_or_default()
}

/// CPU, memory and storage for quick settings (the shared reader). The CPU share needs the previous reading, kept
/// here between calls.
fn system() -> primitive_quick::Stats {
    static METER: std::sync::Mutex<Option<primitive_quick::CpuMeter>> = std::sync::Mutex::new(None);
    let mut guard = match METER.lock() {
        Ok(g) => g,
        Err(_) => return primitive_quick::Stats::default(),
    };
    let meter = guard.get_or_insert_with(primitive_quick::CpuMeter::new);
    primitive_quick::read(meter, "/mnt/us")
}

/// Whether the SSH server started by `ssh.sh` is running (its pid file).
fn ssh_running() -> bool {
    std::fs::read_to_string("/tmp/dropbear_launcher.pid")
        .ok()
        .and_then(|p| p.trim().parse::<i32>().ok())
        // SAFETY: signal 0 only checks that the process exists.
        .is_some_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
}

pub fn status(settings: &Settings) -> Status {
    let storage = free_storage();
    let ip = wlan_ip();
    let ssh = ssh_running();
    // The connect command, shown while the server runs (the address is the Kindle's Wi-Fi address).
    let ssh_address = match (ssh, ip.as_str()) {
        (false, _) => String::new(),
        (true, "not connected") => "no Wi-Fi address yet".into(),
        (true, ip) => format!("ssh -p 2222 root@{ip}"),
    };
    let rotation = settings.rotation;
    let sleep_launcher = settings.sleep_screen_launcher;
    let clock = read_clock();
    let (wifi_on, wifi_state) = wifi_status();
    let power = battery_reading();
    let light = light_level();
    let warmth = warmth_level();
    let display_support = kl_core::ui::DisplaySupport {
        front_light: light.is_some(),
        warmth: warmth.is_some(),
        auto_rotation: auto_rotation_supported(),
    };
    Status {
        clock: clock.time,
        date: clock.date,
        clock_year: clock.year,
        utc_offset: clock.utc_offset,
        clock_sync_available: std::fs::metadata(NTPDATE)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0),
        battery: power.percent,
        charging: power.source == Source::Charging,
        power,
        battery_history: Vec::new(),
        battery_now: 0,
        low_power: settings.low_power,
        usb: usb_state(),
        autostart: !Path::new(&launcher_dir()).join("DISABLE").exists(),
        sleep_launcher,
        auto_rotation: settings.auto_rotation && display_support.auto_rotation,
        dark_mode: settings.dark_mode,
        animation: settings.animation,
        large_text: settings.large_text,
        high_contrast: settings.high_contrast,
        wifi_on,
        wifi_state,
        wifi_signal: lipc_text(WIFID, "signalStrength").and_then(|t| kl_core::wifi::parse_signal(&t)),
        light: light.unwrap_or(0),
        warmth: warmth.unwrap_or(0),
        display_support,
        ssh,
        ssh_address,
        ssh_password_set: ssh_password_set(),
        public_dns: public_dns(),
        rotation,
        memory: mem_available(),
        ip,
        system: system(),
        storage,
    }
}

/// Whether Wi-Fi is enabled and the current connection state, without starting a scan.
pub fn wifi_status() -> (bool, String) {
    let enabled = get_int(WIFID, "enable").is_some_and(|v| v != 0);
    let state = get_text(WIFID, "cmState");
    (enabled, wifi_state_label(enabled, &state).into())
}

fn wifi_state_label(enabled: bool, state: &str) -> &'static str {
    match (enabled, state.to_ascii_uppercase().as_str()) {
        (false, _) => "off",
        (true, "CONNECTED") => "connected",
        (true, "") => "",
        (true, _) => "not connected",
    }
}

/// A property's text, or `None` when it cannot be read.
fn lipc_text(service: &str, prop: &str) -> Option<String> {
    let text = get_text(service, prop);
    (!text.is_empty()).then_some(text)
}

/// Scan for Wi-Fi networks and read the connected one from `com.lab126.wifid`: a scan takes a few seconds, so this
/// blocks; the driver runs it on its own thread. With Wi-Fi off there is nothing to scan.
pub fn wifi_info() -> kl_core::wifi::WifiInfo {
    if get_int(WIFID, "enable") != Some(1) {
        return kl_core::wifi::WifiInfo::default();
    }
    let _ = Command::new("lipc-set-prop").args([WIFID, "scan", "1"]).stderr(Stdio::null()).status();
    // The scan starts after the request; scanState reads "idle" when it is done (about 12 s at most).
    thread::sleep(Duration::from_secs(2));
    for _ in 0..20 {
        if get_text(WIFID, "scanState") == "idle" {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    let scan = Command::new("sh")
        .arg("-c")
        .arg("echo '{}' | lipc-hash-prop -n com.lab126.wifid scanList")
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let report = get_text(WIFID, "711");
    // A network is saved when the Kindle has a profile for it (the connected one may have been forgotten).
    let saved: Vec<String> = saved_profiles().into_iter().map(|(_, n)| n).collect();
    let mut current = kl_core::wifi::parse_current(&report);
    if let Some(c) = current.as_mut() {
        c.known = saved.contains(&c.name);
    }
    let networks = kl_core::wifi::parse_scan(&scan)
        .into_iter()
        .map(|n| kl_core::wifi::Network { known: saved.contains(&n.name), ..n })
        .collect();
    kl_core::wifi::WifiInfo { current, networks }
}

/// The saved profiles from the tool, with their passwords (see `saved_profile_list`).
fn saved_profile_list() -> Vec<kl_core::wifi::SavedProfile> {
    let text = Command::new("sh")
        .arg("-c")
        .arg("echo '{}' | lipc-hash-prop -n com.lab126.wifid profileData")
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    kl_core::wifi::parse_saved_profiles(&text)
}

/// The saved profiles: network ID and name.
fn saved_profiles() -> Vec<(u32, String)> {
    saved_profile_list().into_iter().map(|p| (p.id, p.name)).collect()
}

/// The network ID of the saved profile named `name`, if there is one.
fn profile_id(name: &str) -> Option<u32> {
    saved_profiles().into_iter().find(|(_, n)| n == name).map(|(id, _)| id)
}

/// Delete a saved profile by its network ID.
fn delete_profile(id: u32) -> bool {
    Command::new("lipc-set-prop")
        .args(["-i", WIFID, "deleteProfile", &id.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Save a profile for a network with its password (`password` for a secured network), on the tool's input.
fn create_profile(name: &str, secured: bool, password: Option<&str>) -> bool {
    let input = kl_core::wifi::profile_input(name, if secured { password } else { None });
    let Ok(mut child) = Command::new("lipc-hash-prop")
        .args([WIFID, "createProfile"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(input.as_bytes());
    }
    child.wait().is_ok_and(|s| s.success())
}

/// Save (when the network is not saved yet) and connect to a network. A new profile replaces an old one with the
/// same name; if the join then fails, the old profile (and its password) is put back, so the Kindle keeps what it had.
/// Passwords go to the tool on its input only: never on a command line, never logged. Blocks while the connection is
/// made (up to about 40 s), so the driver runs it on its own thread. Succeeds when the connected network is `name`.
pub fn join_network(name: &str, secured: bool, password: Option<&str>, saved: bool) -> Result<(), String> {
    let previous = saved_profile_list().into_iter().find(|p| p.name == name);
    if !saved {
        if let Some(old) = &previous {
            delete_profile(old.id);
        }
        if !create_profile(name, secured, password) {
            restore(previous.as_ref());
            return Err("the network could not be saved".into());
        }
    }
    let _ = Command::new("lipc-set-prop")
        .args([CMD, "ensureConnection", &format!("wifi:{name}")])
        .stderr(Stdio::null())
        .status();
    // Wait for the connection to report this network: the report (property 711) names the current AP.
    let mut connected = false;
    for _ in 0..80 {
        thread::sleep(Duration::from_millis(500));
        let report = get_text(WIFID, "711");
        if kl_core::wifi::parse_current(&report).is_some_and(|n| n.name == name) {
            connected = true;
            break;
        }
    }
    if !connected {
        let in_range = wifi_info().networks.iter().any(|n| n.name == name);
        log(&format!("join {name}: not connected, connection state {}, in range {in_range}", get_text(WIFID, "cmState")));
        if !saved {
            restore(previous.as_ref());
        }
        return Err(kl_core::wifi::join_failure(in_range, saved));
    }
    // Connected is not enough: the network must also give an address (DHCP), or nothing on the Kindle can reach it.
    for _ in 0..30 {
        if wlan_ip() != "not connected" {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
    log(&format!("join {name}: connected, but no address"));
    Err(kl_core::wifi::NO_ADDRESS.into())
}

/// Restart the launcher, as after an install: the supervisor starts it again.
pub fn restart_launcher() {
    log("restart requested from Settings");
    // SAFETY: getpid and kill take no pointers; SIGTERM to this process is the same signal the install sends.
    unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
}

/// Put an old profile back after a failed join (its password was kept in memory, and is not logged).
fn restore(previous: Option<&kl_core::wifi::SavedProfile>) {
    if let Some(old) = previous {
        if create_profile(&old.name, old.secured, old.psk.as_deref()) {
            log(&format!("restored the saved profile for {}", old.name));
        }
    }
}

/// Forget a saved network: delete its profile.
pub fn forget_network(name: &str) -> Result<(), String> {
    match profile_id(name) {
        Some(id) if delete_profile(id) => Ok(()),
        Some(_) => Err("it could not be forgotten".into()),
        None => Err("it is not saved".into()),
    }
}

/// The Wi-Fi address (`ifconfig wlan0`), or "not connected".
pub fn wlan_ip() -> String {
    Command::new("ifconfig")
        .arg("wlan0")
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|o| kl_core::wifi::parse_ip(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_else(|| "not connected".into())
}

/// Free space on the Kindle's user partition (`/mnt/us`), e.g. "2.1 GB free".
pub fn free_storage() -> String {
    let Ok(bytes) = available_storage_bytes(std::path::Path::new("/mnt/us")) else { return "unknown".into() };
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB free", bytes as f64 / 1e9)
    } else {
        format!("{} MB free", bytes / 1_000_000)
    }
}

pub fn available_storage_bytes(path: &std::path::Path) -> std::io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: The path is NUL terminated, and statvfs initializes the output on success.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let stats = unsafe { stats.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

pub fn set_light(level: u32) {
    let _ = try_set_light(level);
}

pub fn light_level() -> Option<u32> {
    get_int(POWERD, "flIntensity").filter(|level| *level <= LEVEL_MAX)
}

pub fn try_set_light(level: u32) -> bool {
    set_int(POWERD, "flIntensity", level.min(LEVEL_MAX))
}

pub fn set_warmth(level: u32) {
    let _ = try_set_warmth(level);
}

pub fn warmth_level() -> Option<u32> {
    get_int(POWERD, "currentAmberLevel").filter(|level| *level <= LEVEL_MAX)
}

pub fn try_set_warmth(level: u32) -> bool {
    set_int(POWERD, "currentAmberLevel", level.min(LEVEL_MAX))
}

pub fn set_wifi(on: bool) {
    set_int(CMD, "wirelessEnable", u32::from(on));
}

/// The device's turn from the framework's accelerometer (`U` upright, `D` upside down). Landscape and
/// unreadable values give nothing. The browser reads the same property.
pub fn accelerometer_turn() -> Option<Rotation> {
    // The accelerometer itself (works with the stock UI stopped); the window manager's property otherwise.
    if let Some(path) = accel_value_path() {
        let text = std::fs::read_to_string(path).ok()?;
        let (x, y, z) = kl_core::rotation::parse_accel(&text)?;
        return kl_core::rotation::turn_from_accel(x, y, z);
    }
    let out = Command::new("lipc-get-prop").args(["com.lab126.winmgr", "accelerometer"]).output().ok()?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "U" => Some(Rotation::Upright),
        "D" => Some(Rotation::UpsideDown),
        _ => None,
    }
}

fn auto_rotation_supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        accel_value_path().is_some()
            || matches!(get_text("com.lab126.winmgr", "accelerometer").as_str(), "U" | "D" | "L" | "R")
    })
}

/// The accelerometer's `value` file: the input device named `bma2x2` (`/sys/class/input/input6` on the
/// Oasis 3; found by name, as the numbers can change). Looked up once.
fn accel_value_path() -> Option<&'static std::path::PathBuf> {
    static PATH: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let found = std::fs::read_dir("/sys/class/input").ok()?.flatten().map(|e| e.path()).find(|p| {
            std::fs::read_to_string(p.join("name")).is_ok_and(|n| n.trim() == "bma2x2") && p.join("value").exists()
        });
        log(&format!("accelerometer: {}", found.as_ref().map_or("not found, using the window manager".into(), |p| p.display().to_string())));
        found.map(|p| p.join("value"))
    })
    .as_ref()
}

/// Follow the device's turn, 4 readings a second. A new turn is sent after it has been read for 800 ms in a
/// row (`kl_core::rotation`); a flat, sideways or unreadable reading starts the wait again. Return false
/// from the callback to stop following orientation changes.
pub fn orientation_loop(start: Rotation, mut report: impl FnMut(Rotation) -> bool) {
    let mut debounce = Debouncer::new(start, 800);
    let began = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let now = began.elapsed().as_millis() as u64;
        if let Some(turn) = debounce.feed(accelerometer_turn(), now) {
            if !report(turn) {
                return;
            }
        }
    }
}

/// Unload the stock screensaver (blanket's module), so the launcher's own sleep screen is what shows on
/// sleep. KOReader does the same in its framework-stopped mode. It stays unloaded until blanket restarts
/// (Kindle mode or a reboot).
pub fn unload_stock_screensaver() {
    set_stock_screensaver(false);
}

pub fn stock_screensaver_loaded() -> bool {
    get_text("com.lab126.blanket", "load").split_whitespace().any(|module| module == "screensaver")
}

pub fn set_stock_screensaver(enabled: bool) -> bool {
    let operation = if enabled { "load" } else { "unload" };
    let ok = Command::new("lipc-set-prop")
        .args(["com.lab126.blanket", operation, "screensaver"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    log(&format!("stock screensaver {operation}: {ok}"));
    ok
}

/// The launcher's folder (`KL_DIR`, default /mnt/us/launcher).
pub fn launcher_dir() -> String {
    std::env::var("KL_DIR").unwrap_or_else(|_| "/mnt/us/launcher".into())
}

/// Autostart on: the boot guard may start the launcher. Off: the boot guard leaves the stock UI alone (the
/// DISABLE file). Turning it on also clears the failed-boot count, so the guard counts from zero.
pub fn set_autostart(dir: &str, on: bool) {
    let disable = Path::new(dir).join("DISABLE");
    if on {
        let _ = std::fs::remove_file(&disable);
        let _ = std::fs::remove_file(Path::new(dir).join("boot-attempts"));
    } else {
        let _ = std::fs::write(&disable, b"");
    }
    log(&format!("autostart {}", if on { "on" } else { "off" }));
}

/// Run one of the launcher's scripts (in `dir`) with `arg`, and `input` on its stdin. Returns whether it succeeded
/// and what it printed, on one line. The output is logged; `input` (a password) never is.
fn run_script(dir: &str, script: &str, arg: &str, input: Option<&str>) -> (bool, String) {
    use std::io::Write;
    let path = format!("{dir}/{script}");
    let child = Command::new("/bin/sh")
        .args([path.as_str(), arg])
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            log(&format!("{script} {arg}: {e}"));
            return (false, format!("could not run {script}"));
        }
    };
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = writeln!(stdin, "{text}");
    }
    match child.wait_with_output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("; ");
            log(&format!("{script} {arg}: {} {text}", out.status));
            (out.status.success(), text)
        }
        Err(e) => {
            log(&format!("{script} {arg}: {e}"));
            (false, format!("{script} did not finish"))
        }
    }
}

/// Restart the Kindle (`reboot`) or turn it off (`halt -f -p`), from the Power menu. The command runs detached: the
/// launcher ends with the system. `/sbin/poweroff` is a link to `reboot`, which runs the stock shutdown job and
/// shows its "restarting" cover before it turns off; `halt -f -p` turns off without that job.
pub fn system_command(cmd: &str) {
    log(&format!("{cmd} requested from the power menu"));
    let _ = std::process::Command::new("/bin/sh")
        .args(["-c", &format!("sync; /sbin/{cmd}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Start or stop the SSH server (`ssh.sh`): from Settings > SSH & DNS and the quick settings' SSH switch.
pub fn set_ssh(dir: &str, on: bool) {
    run_script(dir, "ssh.sh", if on { "start" } else { "stop" }, None);
}

/// Check the SSH server (`ssh.sh check`): its process, the port and the firewall, as one line.
pub fn check_ssh(dir: &str) -> String {
    let (_, text) = run_script(dir, "ssh.sh", "check", None);
    text
}

/// Set the root password for SSH (`ssh.sh password`), with the password on stdin.
pub fn set_ssh_password(dir: &str, password: &str) -> Result<(), String> {
    match run_script(dir, "ssh.sh", "password", Some(password)) {
        (true, _) => Ok(()),
        (false, text) if text.is_empty() => Err("could not set the password".into()),
        (false, text) => Err(text),
    }
}

/// Public DNS on or off (`dns.sh`), from Settings > SSH & DNS.
pub fn set_public_dns(dir: &str, on: bool) {
    run_script(dir, "dns.sh", if on { "public" } else { "restore" }, None);
}

/// Whether public DNS (1.1.1.1 and 8.8.8.8) is in use: the device's resolver file names it.
fn public_dns() -> bool {
    std::fs::read_to_string("/etc/resolv.conf").is_ok_and(|t| t.contains("1.1.1.1"))
}

/// Whether a root password for SSH has been set from the launcher (the flag `ssh.sh password` writes).
fn ssh_password_set() -> bool {
    Path::new(&launcher_dir()).join("ssh/password-set").exists()
}

/// Map a `lipc-wait-event` line to an input. Lines look like `goingToScreenSaver 2`, or with the source
/// first (`com.lab126.powerd goingToScreenSaver 2`) in some modes, so any word may be the event.
pub fn power_event(line: &str) -> Option<Input> {
    line.split_whitespace().find_map(|word| match word {
        "goingToScreenSaver" => Some(Input::Sleep),
        "outOfScreenSaver" | "exitingScreenSaver" => Some(Input::Wake),
        _ => None,
    })
}

/// Follow the power events. `lipc-wait-event -m` runs as a child; if it ends, it is started again after
/// a pause. Off the Kindle it fails at once, and after a few tries the thread gives up.
pub fn power_loop(mut report: impl FnMut(Input)) {
    let mut failures = 0;
    loop {
        let mut command = Command::new("lipc-wait-event");
        command
            .args(["-m", POWERD, "goingToScreenSaver,outOfScreenSaver,exitingScreenSaver"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // The listener ends with the program that started it: without this, every launcher or app that exits
        // leaves one behind (seen on the Kindle: ten left over after a day of testing).
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // The listener is in the app's process group, and under Primitive Launcher SIGUSR1 to that group asks
            // an app that declares `# Wake: redraw` to redraw. Ignored here (it stays ignored through exec), the
            // signal does not end the listener, which would leave the app without power events for five seconds
            // each time, and for good after five.
            // SAFETY: prctl and signal are async-signal-safe; they only set this child's parent-death signal and
            // its SIGUSR1 disposition.
            unsafe {
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    libc::signal(libc::SIGUSR1, libc::SIG_IGN);
                    Ok(())
                });
            }
        }
        let child = command.spawn();
        match child {
            Ok(mut child) => {
                if let Some(out) = child.stdout.take() {
                    for line in BufReader::new(out).lines().map_while(Result::ok) {
                        log(&format!("power: {line}"));
                        failures = 0;
                        if let Some(input) = power_event(&line) {
                            report(input);
                        }
                    }
                }
                let _ = child.wait();
            }
            Err(e) => log(&format!("lipc-wait-event: {e}")),
        }
        failures += 1;
        if failures >= 5 {
            return log("power events unavailable; giving up");
        }
        thread::sleep(Duration::from_secs(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_state_distinguishes_missing_and_idle_charger_readings() {
        assert_eq!(classify_power_source(None, [Some(false), None], ""), Source::Unknown);
        assert_eq!(classify_power_source(Some(true), [None, None], ""), Source::Unknown);
        assert_eq!(classify_power_source(Some(true), [Some(false), None], ""), Source::PluggedIn);
        assert_eq!(classify_power_source(Some(true), [None, None], "Full"), Source::Full);
        assert_eq!(classify_power_source(Some(false), [None, None], "Full"), Source::OnBattery);
        assert_eq!(classify_power_source(Some(true), [None, Some(true)], ""), Source::Charging);
        assert_eq!(classify_power_source(None, [None, None], "Discharging"), Source::OnBattery);
    }

    #[test]
    fn idle_timeout_refresh_requires_an_active_known_power_state() {
        assert!(may_refresh_screen_timeout("active", Some(0), Some(0)));
        assert!(!may_refresh_screen_timeout("screensaver", Some(0), Some(0)));
        assert!(!may_refresh_screen_timeout("active", Some(1), Some(0)));
        assert!(!may_refresh_screen_timeout("active", Some(0), Some(1)));
        assert!(!may_refresh_screen_timeout("active", None, Some(0)));
        assert!(!may_refresh_screen_timeout("", Some(0), Some(0)));
    }

    #[test]
    fn wifi_status_distinguishes_off_disconnected_and_unknown() {
        assert_eq!(wifi_state_label(false, "CONNECTED"), "off");
        assert_eq!(wifi_state_label(true, "Connected"), "connected");
        assert_eq!(wifi_state_label(true, "DISCONNECTED"), "not connected");
        assert_eq!(wifi_state_label(true, ""), "");
    }

    #[test]
    fn power_lines_map_to_inputs() {
        assert_eq!(power_event("goingToScreenSaver 2"), Some(Input::Sleep));
        assert_eq!(power_event("outOfScreenSaver 1"), Some(Input::Wake));
        assert_eq!(power_event("exitingScreenSaver"), Some(Input::Wake));
        assert_eq!(power_event("com.lab126.powerd goingToScreenSaver 2"), Some(Input::Sleep));
        assert_eq!(power_event("t1TimerReset"), None);
        assert_eq!(power_event(""), None);
    }
}
