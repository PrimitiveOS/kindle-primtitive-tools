//! Quick settings pieces shared by Primitive Launcher and Primitive Browser. Each app draws its own panel (the launcher
//! with vello, the browser as HTML); what both show the same way lives here:
//!
//! - the system figures (CPU, memory, storage), read from Linux, with their labels, values and bar fractions;
//! - the icons of the panel, as SVG paths on a 24-unit grid that both apps can draw.

/// The CPU, memory and storage figures. `None` when a figure could not be read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    /// Busy share since the last reading, in percent.
    pub cpu: Option<u32>,
    /// Memory in use and in total, in MB.
    pub ram: Option<(u64, u64)>,
    /// Storage in use and in total, in bytes.
    pub storage: Option<(u64, u64)>,
}

/// One row of the panel's system section: its label, icon, how full its bar is (0 to 1), and the value to show.
#[derive(Debug, Clone, PartialEq)]
pub struct StatRow {
    pub label: &'static str,
    pub icon: Icon,
    pub fraction: Option<f32>,
    pub value: String,
}

impl Stats {
    /// The three rows: CPU, Memory, Storage.
    pub fn rows(&self) -> [StatRow; 3] {
        let share = |used: u64, total: u64| (total > 0).then(|| (used as f32 / total as f32).clamp(0.0, 1.0));
        [
            StatRow {
                label: "CPU",
                icon: icons::CPU,
                fraction: self.cpu.map(|p| p.min(100) as f32 / 100.0),
                value: self.cpu.map_or("–".into(), |p| format!("{p}%")),
            },
            StatRow {
                label: "Memory",
                icon: icons::MEMORY,
                fraction: self.ram.and_then(|(u, t)| share(u, t)),
                value: self.ram.map_or("–".into(), |(u, t)| format!("{u} of {t} MB")),
            },
            StatRow {
                label: "Storage",
                icon: icons::STORAGE,
                fraction: self.storage.and_then(|(u, t)| share(u, t)),
                value: self.storage.map_or("–".into(), |(u, t)| format!("{} of {}", gb(u), gb(t))),
            },
        ]
    }
}

/// Bytes as gigabytes with one decimal ("1.1 GB").
fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Memory in use and in total (MB) from /proc/meminfo: total less `MemAvailable`.
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |name: &str| -> Option<u64> {
        text.lines().find(|l| l.starts_with(name)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok())
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    Some((total.saturating_sub(available) / 1024, total / 1024))
}

/// The CPU's busy share between two readings of /proc/stat.
#[derive(Debug, Default)]
pub struct CpuMeter {
    previous: Option<(u64, u64)>,
}

impl CpuMeter {
    pub fn new() -> CpuMeter {
        CpuMeter::default()
    }

    /// Feed a /proc/stat text. The busy share since the last one, in percent; `None` for the first.
    pub fn feed(&mut self, stat: &str) -> Option<u32> {
        let fields: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        if fields.len() < 4 {
            return None;
        }
        let total: u64 = fields.iter().sum();
        let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
        let busy = match self.previous {
            Some((t, i)) if total > t => {
                let dt = total - t;
                let di = idle.saturating_sub(i).min(dt);
                Some(((dt - di) * 100 / dt) as u32)
            }
            _ => None,
        };
        self.previous = Some((total, idle));
        busy
    }
}

/// Storage in use and in total (bytes) of the file system at `path`.
#[cfg(unix)]
pub fn storage(path: &str) -> Option<(u64, u64)> {
    let path = std::ffi::CString::new(path).ok()?;
    // SAFETY: statvfs fills the struct it is given; the path is a valid C string.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = st.f_frsize as u64;
    let total = st.f_blocks as u64 * block;
    let free = st.f_bavail as u64 * block;
    Some((total.saturating_sub(free), total))
}

/// Read the three figures on the device: /proc/stat (through `meter`), /proc/meminfo, and the file system at
/// `storage_path` (`/mnt/us` on the Kindle).
#[cfg(unix)]
pub fn read(meter: &mut CpuMeter, storage_path: &str) -> Stats {
    let cpu = std::fs::read_to_string("/proc/stat").ok().and_then(|t| meter.feed(&t));
    let ram = std::fs::read_to_string("/proc/meminfo").ok().and_then(|t| parse_meminfo(&t));
    Stats { cpu, ram, storage: storage(storage_path) }
}

/// An icon: SVG paths on a 24-unit grid; `stroke` paths are drawn as lines 2 units wide, `fill` paths are filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon {
    pub stroke: &'static [&'static str],
    pub fill: &'static [&'static str],
}

/// The panel's icons.
pub mod icons {
    use super::Icon;

    /// A processor: a chip with pins on each side.
    pub const CPU: Icon = Icon {
        stroke: &["M7 7 H17 V17 H7 Z", "M10 10 H14 V14 H10 Z", "M9 3 V7 M12 3 V7 M15 3 V7 M9 17 V21 M12 17 V21 M15 17 V21 M3 9 H7 M3 12 H7 M3 15 H7 M17 9 H21 M17 12 H21 M17 15 H21"], fill: &[],
    };
    /// Memory: a module with chips and contacts.
    pub const MEMORY: Icon = Icon {
        stroke: &["M3 7 H21 V15 H3 Z", "M6 10 H8 V12 H6 Z M11 10 H13 V12 H11 Z M16 10 H18 V12 H16 Z", "M6 15 V18 M10 15 V18 M14 15 V18 M18 15 V18"], fill: &[],
    };
    /// Storage: a drive with a light.
    pub const STORAGE: Icon = Icon {
        stroke: &["M4 5 H20 V19 H4 Z", "M4 13 H20", "M15 16 H17"], fill: &[],
    };
    /// Airplane mode (the Material Design "flight" icon, Apache License 2.0).
    pub const PLANE: Icon = Icon {
        stroke: &[], fill: &["M21 16v-2l-8-5V3.5c0-.83-.67-1.5-1.5-1.5S10 2.67 10 3.5V9l-8 5v2l8-2.5V19l-2 1.5V22l3.5-1 3.5 1v-1.5L13 19v-5.5l8 2.5z"],
    };
    /// Auto-rotate: two arcs with arrow heads.
    pub const ROTATE: Icon = Icon {
        stroke: &["M4 12 A8 8 0 0 1 17.7 6.3", "M18 2 V7 H13", "M20 12 A8 8 0 0 1 6.3 17.7", "M6 22 V17 H11"], fill: &[],
    };
    /// Dark mode: a crescent moon.
    pub const MOON: Icon = Icon {
        stroke: &[], fill: &["M20 14.5 A8.5 8.5 0 1 1 9.5 4 A7 7 0 0 0 20 14.5 Z"],
    };
    /// Brightness: a sun.
    pub const SUN: Icon = Icon {
        stroke: &["M12 7 A5 5 0 1 0 12 17 A5 5 0 1 0 12 7 Z", "M12 2 V4 M12 20 V22 M2 12 H4 M20 12 H22 M4.9 4.9 L6.3 6.3 M17.7 17.7 L19.1 19.1 M4.9 19.1 L6.3 17.7 M17.7 6.3 L19.1 4.9"], fill: &[],
    };
    /// Warmth: a circle half filled.
    pub const WARMTH: Icon = Icon { stroke: &["M12 3 A9 9 0 1 0 12 21 A9 9 0 1 0 12 3 Z"], fill: &["M12 3 A9 9 0 0 1 12 21 Z"] };
    /// Images: a picture frame with a hill and a sun.
    pub const IMAGES: Icon = Icon {
        stroke: &["M3 5 H21 V19 H3 Z", "M3 16 L9 11 L14 15 L17 13 L21 16", "M15.5 8.5 A1.5 1.5 0 1 0 15.5 8.51"], fill: &[],
    };
    /// Animation: a frame inside a frame.
    pub const ANIMATION: Icon = Icon { stroke: &["M3 3 H21 V21 H3 Z", "M8 8 H16 V16 H8 Z"], fill: &[] };
    /// SSH: a prompt.
    pub const SSH: Icon = Icon { stroke: &["M4 7 L9 12 L4 17", "M12 18 H20"], fill: &[] };
    /// Bluetooth: the rune (a stroke through the middle, with its two arms).
    pub const BLUETOOTH: Icon = Icon { stroke: &["M7 7.5 L17 16.5 L12 21 V3 L17 7.5 L7 16.5"], fill: &[] };
    /// Headphones: a headband and two ear cups.
    pub const HEADPHONES: Icon = Icon { stroke: &["M4 15 V12 A8 8 0 0 1 20 12 V15", "M4 15 H8 V20 H4 Z", "M16 15 H20 V20 H16 Z"], fill: &[] };
    /// Sound: a speaker with two waves (the sound row in quick settings).
    pub const SPEAKER: Icon = Icon {
        stroke: &["M4 9 H8 L13 5 V19 L8 15 H4 Z", "M16 9 A4 4 0 0 1 16 15", "M18.5 6 A8 8 0 0 1 18.5 18"], fill: &[],
    };
    /// Settings: a slider with knobs (a gear does not stroke well at small sizes).
    pub const SETTINGS: Icon = Icon {
        stroke: &["M4 7 H20 M4 17 H20", "M9 4.5 V9.5 M15 14.5 V19.5"], fill: &[],
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_in_use_is_total_less_available() {
        let text = "MemTotal:         512000 kB\nMemFree:  10000 kB\nMemAvailable:     333568 kB\n";
        assert_eq!(parse_meminfo(text), Some(((512000 - 333568) / 1024, 512000 / 1024)));
        assert_eq!(parse_meminfo("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn cpu_needs_two_readings() {
        let mut m = CpuMeter::new();
        assert_eq!(m.feed("cpu  100 0 100 800 0 0 0 0 0 0\n"), None);
        // 100 more ticks: 25 busy (user 20 + system 5), 75 idle.
        assert_eq!(m.feed("cpu  120 0 105 875 0 0 0 0 0 0\n"), Some(25));
    }

    #[test]
    fn rows_give_labels_values_and_bars() {
        let s = Stats { cpu: Some(12), ram: Some((180, 512)), storage: Some((1_100_000_000, 6_100_000_000)) };
        let rows = s.rows();
        assert_eq!(rows[0].value, "12%");
        assert_eq!(rows[1].value, "180 of 512 MB");
        assert_eq!(rows[2].value, "1.1 GB of 6.1 GB");
        assert!((rows[1].fraction.unwrap() - 180.0 / 512.0).abs() < 0.001);
        let empty = Stats::default().rows();
        assert_eq!(empty[0].fraction, None);
        assert_eq!(empty[2].value, "–");
    }

    #[cfg(unix)]
    #[test]
    fn the_root_file_system_has_a_size() {
        let (used, total) = storage("/").unwrap();
        assert!(total > 0 && used <= total);
    }
}
