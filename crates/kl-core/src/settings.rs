//! The launcher's `key=value` settings. Keep newer power and accessibility preferences in a separate
//! file because older Store builds rewrite `settings.txt` using only the keys they recognize.

use std::{fs, path::{Path, PathBuf}};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    Upright,
    UpsideDown,
}

impl Rotation {
    /// The turn in quarter turns the browser uses: 0 upright, 2 upside down.
    pub fn turn(self) -> u32 {
        match self {
            Rotation::Upright => 0,
            Rotation::UpsideDown => 2,
        }
    }

    /// The letter the framework uses for its accelerometer property, published for apps in
    /// `/var/tmp/kl/orientation`.
    pub fn letter(self) -> &'static str {
        match self {
            Rotation::Upright => "U",
            Rotation::UpsideDown => "D",
        }
    }

    pub fn flipped(self) -> Rotation {
        match self {
            Rotation::Upright => Rotation::UpsideDown,
            Rotation::UpsideDown => Rotation::Upright,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub rotation: Rotation,
    /// True: the screen follows the device's turn (framework accelerometer). False: `rotation` is fixed.
    pub auto_rotation: bool,
    /// True: the launcher's own sleep screen (the stock screensaver is unloaded). False: the Kindle's.
    pub sleep_screen_launcher: bool,
    /// White on black: the launcher's screens are drawn inverted (picture icons keep their look).
    pub dark_mode: bool,
    /// The opening zoom when an app starts. On by default.
    pub animation: bool,
    pub large_text: bool,
    pub high_contrast: bool,
    /// Defer automatic Store refreshes and limit the front light while enabled.
    pub low_power: bool,
    /// Refresh the store automatically when its catalog is a day old.
    pub store_daily_check: bool,
    /// The Kindle model the owner chose (`models::MODELS` id), used only when the model cannot be detected.
    pub device_model: Option<String>,
    /// The owner's name for this Kindle, typed on the Device name page (up to 32 characters).
    pub device_name: Option<String>,
    /// Seconds the screen stays on before the Kindle sleeps (0 = never). One of `SCREEN_TIMEOUTS`.
    pub screen_timeout: u32,
}

/// The screen timeouts the owner can choose, in seconds. 0 is never.
pub const SCREEN_TIMEOUTS: [u32; 7] = [15, 30, 60, 120, 300, 600, 0];

/// The next timeout in the cycle (quick settings and Settings both step through it).
pub fn next_screen_timeout(current: u32) -> u32 {
    let at = SCREEN_TIMEOUTS.iter().position(|&secs| secs == current).unwrap_or(SCREEN_TIMEOUTS.len() - 1);
    SCREEN_TIMEOUTS[(at + 1) % SCREEN_TIMEOUTS.len()]
}

/// The timeout as the owner reads it: "15 s", "5 min", "Never".
pub fn screen_timeout_label(secs: u32) -> String {
    match secs {
        0 => "Never".to_string(),
        s if s < 60 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            rotation: Rotation::Upright,
            auto_rotation: true,
            sleep_screen_launcher: false,
            dark_mode: false,
            animation: true,
            large_text: false,
            high_contrast: false,
            low_power: false,
            store_daily_check: true,
            device_model: None,
            device_name: None,
            screen_timeout: 600,
        }
    }
}

impl Settings {
    /// Read each file's own fields so a rewrite of the legacy file cannot reset newer preferences.
    pub fn load(path: &Path) -> Self {
        let base = Self::parse(&fs::read_to_string(path).unwrap_or_default());
        let preferences = Self::parse(&fs::read_to_string(Self::preferences_path(path)).unwrap_or_default());
        Self {
            large_text: preferences.large_text,
            high_contrast: preferences.high_contrast,
            low_power: preferences.low_power,
            screen_timeout: preferences.screen_timeout,
            ..base
        }
    }

    pub fn preferences_path(settings_path: &Path) -> PathBuf {
        settings_path.with_file_name("launcher-preferences.txt")
    }

    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match (key.trim(), value.trim()) {
                ("rotation", "upright") => s.rotation = Rotation::Upright,
                ("rotation", "upside-down") => s.rotation = Rotation::UpsideDown,
                ("sleep_screen", "launcher") => s.sleep_screen_launcher = true,
                ("auto_rotation", "on") => s.auto_rotation = true,
                ("auto_rotation", "off") => s.auto_rotation = false,
                ("sleep_screen", "kindle") => s.sleep_screen_launcher = false,
                ("dark_mode", value) => s.dark_mode = value == "on",
                ("animation", value) => s.animation = value != "off",
                ("large_text", "1") => s.large_text = true,
                ("large_text", "0") => s.large_text = false,
                ("high_contrast", "1") => s.high_contrast = true,
                ("high_contrast", "0") => s.high_contrast = false,
                ("low_power", "1") => s.low_power = true,
                ("low_power", "0") => s.low_power = false,
                ("store_daily_check", "1") => s.store_daily_check = true,
                ("store_daily_check", "0") => s.store_daily_check = false,
                ("device_model", value) if !value.is_empty() => s.device_model = Some(value.to_string()),
                ("device_name", value) if !value.is_empty() => s.device_name = Some(value.to_string()),
                ("screen_timeout", value) => {
                    if let Some(secs) = value.parse().ok().filter(|secs| SCREEN_TIMEOUTS.contains(secs)) {
                        s.screen_timeout = secs;
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        self.base_text() + &self.preferences_text()
    }

    /// The original on-device file format, shared with older Store and app builds.
    pub fn base_text(&self) -> String {
        let rotation = match self.rotation {
            Rotation::Upright => "upright",
            Rotation::UpsideDown => "upside-down",
        };
        let sleep = if self.sleep_screen_launcher { "launcher" } else { "kindle" };
        let on = |b: bool| if b { "on" } else { "off" };
        format!(
            "rotation={rotation}\nauto_rotation={}\nsleep_screen={sleep}\ndark_mode={}\nanimation={}\nstore_daily_check={}\n",
            on(self.auto_rotation),
            on(self.dark_mode),
            on(self.animation),
            u8::from(self.store_daily_check),
        )
    }

    pub fn preferences_text(&self) -> String {
        format!(
            "large_text={}\nhigh_contrast={}\nlow_power={}\n",
            u8::from(self.large_text),
            u8::from(self.high_contrast),
            u8::from(self.low_power),
        ) + &self.device_model.as_ref().map_or_else(String::new, |id| format!("device_model={id}\n"))
            + &self.device_name.as_ref().map_or_else(String::new, |name| format!("device_name={name}\n"))
            + &format!("screen_timeout={}\n", self.screen_timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let s = Settings {
            rotation: Rotation::UpsideDown,
            auto_rotation: false,
            sleep_screen_launcher: true,
            dark_mode: true,
            animation: false,
            large_text: true,
            high_contrast: true,
            low_power: true,
            store_daily_check: false,
            device_model: None,
            device_name: None,
            screen_timeout: 0,
        };
        assert_eq!(Settings::parse(&s.to_text()), s);
        assert!(Settings::default().animation && !Settings::default().dark_mode);
        assert!(Settings::parse("animation=yes\n").animation, "only off turns it off");
        assert_eq!(Settings::parse("junk\nrotation=sideways\n"), Settings::default());
    }

    #[test]
    fn auto_rotation_is_on_unless_the_file_says_off() {
        assert!(Settings::default().auto_rotation);
        assert!(Settings::parse("rotation=upright\n").auto_rotation);
        assert!(!Settings::parse("auto_rotation=off\n").auto_rotation);
        assert!(Settings::parse("auto_rotation=on\n").auto_rotation);
    }

    #[test]
    fn store_daily_check_defaults_to_on_and_can_be_turned_off() {
        assert!(Settings::parse("rotation=upright\n").store_daily_check);
        assert!(Settings::parse("store_daily_check=maybe\n").store_daily_check);
        assert!(!Settings::parse("store_daily_check=0\n").store_daily_check);
        assert!(Settings::parse("store_daily_check=0\nstore_daily_check=1\n").store_daily_check);
    }

    #[test]
    fn older_preferences_keep_the_normal_appearance_and_power_policy() {
        let original = Settings::parse("dark_mode=on\nanimation=off\nstore_daily_check=0\n");
        assert!(original.dark_mode);
        assert!(!original.animation && !original.store_daily_check);
        assert!(!original.large_text && !original.high_contrast && !original.low_power);
    }

    #[test]
    fn an_older_store_rewrite_preserves_newer_preferences() {
        let directory = std::env::temp_dir().join(format!("launcher-settings-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("settings.txt");
        let owner = Settings { large_text: true, high_contrast: true, low_power: true, ..Settings::default() };
        fs::write(&path, owner.base_text()).unwrap();
        fs::write(Settings::preferences_path(&path), owner.preferences_text()).unwrap();

        // The old Store parses only the legacy file and rewrites its recognized keys.
        let mut legacy = Settings::parse(&fs::read_to_string(&path).unwrap());
        legacy.store_daily_check = false;
        fs::write(&path, legacy.base_text()).unwrap();

        let loaded = Settings::load(&path);
        assert_eq!(loaded, Settings { store_daily_check: false, ..owner });
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn turns_and_letters() {
        assert_eq!(Rotation::UpsideDown.turn(), 2);
        assert_eq!(Rotation::Upright.letter(), "U");
        assert_eq!(Rotation::Upright.flipped(), Rotation::UpsideDown);
    }
}

#[cfg(test)]
mod screen_timeout_tests {
    use super::*;

    #[test]
    fn the_timeout_cycles_through_the_choices_and_wraps_to_fifteen_seconds() {
        assert_eq!(next_screen_timeout(15), 30);
        assert_eq!(next_screen_timeout(600), 0, "ten minutes is followed by never");
        assert_eq!(next_screen_timeout(0), 15, "never wraps to the shortest");
        assert_eq!(next_screen_timeout(42), 15, "an unknown value starts the cycle again");
    }

    #[test]
    fn the_labels_read_as_the_owner_sees_them() {
        assert_eq!(screen_timeout_label(15), "15 s");
        assert_eq!(screen_timeout_label(120), "2 min");
        assert_eq!(screen_timeout_label(0), "Never");
    }

    #[test]
    fn the_timeout_is_saved_in_the_preferences_and_read_back() {
        let mut s = Settings::default();
        assert_eq!(s.screen_timeout, 600, "the stock ten minutes by default");
        s.screen_timeout = 300;
        let back = Settings::parse(&s.preferences_text());
        assert_eq!(back.screen_timeout, 300);
        assert_eq!(Settings::parse("screen_timeout=7\n").screen_timeout, 600, "only the listed values are read");
    }
}
