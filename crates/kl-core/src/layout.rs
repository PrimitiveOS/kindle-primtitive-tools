//! Screen geometry, shared by drawing and hit testing so a tap lands on what is drawn. All values are
//! device pixels of the upright portrait screen (1264 × 1680 on the Oasis 3).

use crate::wallpaper::WallpaperTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && y >= self.y as f32 && x < (self.x + self.w) as f32 && y < (self.y + self.h) as f32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    /// The rectangle shrunk by `d` on every side.
    pub fn inset(&self, d: i32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2 * d).max(0), (self.h - 2 * d).max(0))
    }
}

/// The steps of the opening zoom: `n` rectangles growing from `from` (the tapped icon) to `to` (the screen),
/// eased out (big steps first), the last one `to`.
pub fn zoom_steps(from: Rect, to: Rect, n: usize) -> Vec<Rect> {
    let lerp = |a: i32, b: i32, t: f32| a + ((b - a) as f32 * t).round() as i32;
    (1..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let t = 1.0 - (1.0 - t) * (1.0 - t);
            Rect::new(lerp(from.x, to.x, t), lerp(from.y, to.y, t), lerp(from.w, to.w, t), lerp(from.h, to.h, t))
        })
        .collect()
}

/// The pages of Settings. The Settings list shows them in groups; a page is sections of setting rows, each with a
/// header, a card, and a footer that explains it (Apple's grouped lists, adapted to e-ink).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Wifi,
    Connection,
    FileTransfer,
    Display,
    Wallpaper,
    Accessibility,
    Battery,
    Usage,
    Sleep,
    Apps,
    InstalledApps,
    AppVisibility,
    Store,
    Storage,
    Device,
    ChooseModel,
    DateTime,
    About,
}

/// The number of models the choice page lists.
const MODEL_COUNT: usize = crate::models::MODELS.len();

/// One row per model, in the order of `models::MODELS`.
static MODEL_ROWS: [Row; MODEL_COUNT] = model_rows();

const fn model_rows() -> [Row; MODEL_COUNT] {
    let mut rows = [Row::Model(0); MODEL_COUNT];
    let mut i = 0;
    while i < MODEL_COUNT {
        rows[i] = Row::Model(i as u8);
        i += 1;
    }
    rows
}

/// The choice page's one section: every supported model.
static CHOOSE_MODEL_GROUPS: [Group; 1] = [Group {
    header: "Supported Kindles",
    rows: &MODEL_ROWS,
    footer: "Choose your model only if the launcher cannot detect it. Installs depend only on the processor (armhf).",
}];

/// One section of a page: its header, its rows in one card, and its footer.
pub struct Group {
    pub header: &'static str,
    pub rows: &'static [Row],
    pub footer: &'static str,
}

impl Page {
    pub const ALL: [Page; 16] = [
        Page::Battery, Page::Display, Page::Wallpaper, Page::Accessibility, Page::Sleep, Page::Wifi,
        Page::Connection, Page::FileTransfer, Page::Apps, Page::InstalledApps, Page::AppVisibility,
        Page::Store, Page::Storage, Page::Device, Page::DateTime, Page::About,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Wifi => "Wi-Fi & network",
            Page::Connection => "SSH & DNS",
            Page::FileTransfer => "Wi-Fi file transfer",
            Page::Display => "Display",
            Page::Wallpaper => "Wallpaper",
            Page::Accessibility => "Accessibility",
            Page::Battery => "Battery",
            Page::Usage => "Usage, last 7 days",
            Page::Sleep => "Sleep",
            Page::Apps => "Apps",
            Page::InstalledApps => "Installed apps",
            Page::AppVisibility => "App visibility",
            Page::Store => "Store",
            Page::Storage => "Storage",
            Page::Device => "Device",
            Page::ChooseModel => "Choose your Kindle",
            Page::DateTime => "Date and time",
            Page::About => "About",
        }
    }

    /// The sections of the page, top to bottom. About has none: its rows are key and value text.
    pub fn groups(self) -> &'static [Group] {
        match self {
            Page::Wifi => &[Group { header: "Wi-Fi", rows: &[Row::Wifi], footer: "Turn Wi-Fi off in an airplane, or to save battery." }],
            Page::Connection => &[
                Group {
                    header: "SSH",
                    rows: &[Row::Ssh, Row::SshPassword, Row::SshAddress, Row::SshCheck],
                    footer: "Off by default. Connect from a computer on the same Wi-Fi, with the address shown and the password you set. Quick settings' SSH switch does the same.",
                },
                Group {
                    header: "DNS",
                    rows: &[Row::PublicDns],
                    footer: "On uses Cloudflare and Google (1.1.1.1, 8.8.8.8) instead of the network's servers. Off puts the network's own settings back.",
                },
            ],
            Page::Display => &[
                Group { header: "Brightness", rows: &[Row::Frontlight, Row::Warmth], footer: "" },
                Group { header: "Appearance", rows: &[Row::DarkMode, Row::WallpaperLink], footer: "Dark mode lightens text against a dark background." },
                Group { header: "Motion", rows: &[Row::Animation], footer: "A card zooms out of the icon when an app opens." },
                Group { header: "Screen", rows: &[Row::Rotation], footer: "Turns with the Kindle while Auto is on. Tap the value to set it by hand." },
            ],
            Page::Wallpaper => &[
                Group {
                    header: "Launcher",
                    rows: &[Row::Wallpaper(WallpaperTarget::Home), Row::Wallpaper(WallpaperTarget::Sleep)],
                    footer: "The Sleep Screen picture is used when the Kindle goes to sleep with Launcher open.",
                },
            ],
            Page::Accessibility => &[
                Group {
                    header: "Text and contrast",
                    rows: &[Row::LargeText, Row::HighContrast, Row::TextPreview],
                    footer: "These options apply to Launcher screens.",
                },
                Group {
                    header: "Motion",
                    rows: &[Row::Animation],
                    footer: "You can also change this from Quick Settings.",
                },
            ],
            Page::Usage => &[
                Group {
                    header: "Last 7 days",
                    rows: &[Row::UsageScreen, Row::UsageApp(0), Row::UsageApp(1), Row::UsageApp(2), Row::UsageApp(3)],
                    footer: "Estimated from CPU time and time in front. Not a measurement of battery.",
                },
            ],
            Page::Battery => &[
                Group { header: "Current charge", rows: &[Row::BatteryReading, Row::LowPower, Row::UsageLink], footer: "Low power mode dims the front light when enabled and pauses automatic Store checks." },
                Group { header: "Power settings", rows: &[Row::ScreenTimeout, Row::BatteryDisplay, Row::BatteryWifi, Row::StoreSettings], footer: "" },
            ],
            Page::Sleep => &[Group {
                header: "Sleep screen",
                rows: &[Row::SleepScreen],
                footer: "Use Launcher pictures, or return to the Kindle screensaver after opening the Kindle UI or restarting.",
            }],
            Page::Apps => &[
                Group { header: "Management", rows: &[Row::ManageInstalled], footer: "" },
                Group { header: "Home screen", rows: &[Row::AppVisibility, Row::Hidden, Row::GridColumns, Row::GridRows], footer: "Long-press an app on the home screen to move it or change its icon." },
            ],
            Page::Store => &[Group {
                header: "Catalog",
                rows: &[Row::StoreDailyCheck, Row::StoreClearCache],
                footer: "The Store checks for updates over Wi-Fi when it opens.",
            }],
            Page::Storage => &[
                Group { header: "Kindle user storage", rows: &[Row::StorageUsage], footer: "" },
                Group { header: "Manage storage", rows: &[Row::ManageInstalled, Row::StoreSettings], footer: "Use Store settings to clear the saved catalog." },
            ],
            Page::Device => &[
                Group { header: "Device name", rows: &[Row::DeviceName], footer: "Shown on the Device information page." },
                Group { header: "Kindle model", rows: &[Row::DeviceModel], footer: "The launcher detects your Kindle. If it cannot, choose it from the list." },
                Group { header: "Startup", rows: &[Row::Autostart], footer: "Choose whether Launcher opens when the Kindle starts." },
                Group { header: "Launcher", rows: &[Row::Restart], footer: "Starts the launcher again, as after an install. Apps and settings stay." },
                Group { header: "Kindle", rows: &[Row::DeviceAbout, Row::KindleUi], footer: "Opening the Kindle UI takes about a minute." },
            ],
            Page::DateTime => &[
                Group { header: "Kindle clock", rows: &[Row::LocalTime, Row::LocalDate, Row::SyncClock], footer: "Sync uses Cloudflare’s time service over Wi-Fi." },
                Group { header: "Time zone", rows: &[Row::TimeZone], footer: "Launcher follows the Kindle’s time zone." },
            ],
            Page::ChooseModel => &CHOOSE_MODEL_GROUPS,
            Page::About | Page::FileTransfer | Page::InstalledApps | Page::AppVisibility => &[],
        }
    }
}

/// The Settings list: its groups, each a header, a card of pages, and nothing under it.
const SETTINGS_GROUPS: &[(&str, &[Page])] = &[
    ("Power and display", &[Page::Battery, Page::Display, Page::Accessibility, Page::Sleep]),
    ("Network", &[Page::Wifi, Page::Connection, Page::FileTransfer]),
    ("Applications", &[Page::Apps, Page::Store]),
    ("System", &[Page::Storage, Page::Device, Page::DateTime, Page::About]),
];

/// A group of the Settings list: its header, its card, and the page rows in it.
pub type HomeSection = (&'static str, Rect, Vec<(Page, Rect)>);

/// A laid-out section of a page.
pub struct Section {
    pub header: &'static str,
    pub footer: &'static str,
    pub header_y: i32,
    pub card: Rect,
    pub footer_y: i32,
    pub rows: Vec<SettingsRow>,
}

/// Controls on the Settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Done,
    /// Open the Device name page.
    DeviceName,
    /// Choose model `index` of `models::MODELS` on the Choose your Kindle page.
    PickModel(u8),
    LightDown,
    LightUp,
    WarmDown,
    WarmUp,
    Wifi,
    Rotation,
    RotationAuto,
    Ssh,
    SshPassword,
    SshCheck,
    PublicDns,
    Autostart,
    ShowHidden,
    OpenPage(Page),
    OpenWallpaperPicker(WallpaperTarget),
    ManageInstalled,
    GridColumns,
    GridRows,
    LowPower,
    StoreDailyCheck,
    StoreClearCache,
    SyncClock,
    RefreshTimeZone,
    SleepScreen,
    KindleUi,
    /// Settings > Device > Launcher: restart the launcher.
    Restart,
    LargeText,
    HighContrast,
    MenuEarlier,
    MenuLater,
    MenuIcon,
    MenuHide,
    ConfirmCancel,
    ConfirmOpen,
    PowerCancel,
    PowerKindleUi,
    PowerRestart,
    PowerOff,
    DarkMode,
    Animation,
    ScreenTimeout,
    /// The Wi-Fi password screen: show or hide the password; join.
    PasswordShow,
    PasswordJoin,
    /// The network screen: connect, or forget a saved network.
    NetConnect,
    NetForget,
}

/// The cards of quick settings, in two columns: toggles, and All settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickTile {
    Airplane,
    AutoRotate,
    DarkMode,
    Animation,
    /// The screen timeout, cycled through the choices (its label shows the current one).
    ScreenTimeout,
    Ssh,
    AllSettings,
    /// The power menu: Cancel, Restart, Power off (what a long press of the power button gives on the Kindle).
    Power,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// The owner's name for this Kindle; opens the Device name page.
    DeviceName,
    /// The Kindle model in use (detected, or chosen); opens the Choose your Kindle page.
    DeviceModel,
    /// Model `index` of `models::MODELS` on the Choose your Kindle page.
    Model(u8),
    Frontlight,
    Warmth,
    DarkMode,
    WallpaperLink,
    /// Opens the usage page: screen time and the most used apps, for the last seven days.
    UsageLink,
    Wallpaper(WallpaperTarget),
    Wifi,
    Rotation,
    Animation,
    /// The screen timeout: how long the screen stays on before the Kindle sleeps (quick settings cycle it too).
    ScreenTimeout,
    /// The Screen on line of the battery usage summary (the last seven days).
    UsageScreen,
    /// An app's line of the usage summary, the most used first (0 to 3).
    UsageApp(u8),
    Ssh,
    SshPassword,
    SshAddress,
    SshCheck,
    PublicDns,
    Autostart,
    Hidden,
    AppVisibility,
    ManageInstalled,
    GridColumns,
    GridRows,
    BatteryReading,
    LowPower,
    BatteryDisplay,
    BatteryWifi,
    StoreSettings,
    StorageUsage,
    DeviceAbout,
    StoreDailyCheck,
    StoreClearCache,
    SleepScreen,
    KindleUi,
    Restart,
    LargeText,
    HighContrast,
    TextPreview,
    LocalTime,
    LocalDate,
    TimeZone,
    SyncClock,
}

impl Row {
    pub const ALL: [Row; 46] = [
        Row::Frontlight,
        Row::Warmth,
        Row::DarkMode,
        Row::WallpaperLink,
        Row::UsageLink,
        Row::Wallpaper(WallpaperTarget::Home),
        Row::Wallpaper(WallpaperTarget::Sleep),
        Row::Wifi,
        Row::Rotation,
        Row::Animation,
        Row::ScreenTimeout,
        Row::UsageScreen,
        Row::UsageApp(0),
        Row::UsageApp(1),
        Row::UsageApp(2),
        Row::UsageApp(3),
        Row::Ssh,
        Row::SshPassword,
        Row::SshAddress,
        Row::SshCheck,
        Row::PublicDns,
        Row::Autostart,
        Row::Hidden,
        Row::AppVisibility, Row::ManageInstalled, Row::GridColumns, Row::GridRows,
        Row::BatteryReading, Row::LowPower, Row::BatteryDisplay, Row::BatteryWifi, Row::StoreSettings,
        Row::StorageUsage, Row::DeviceAbout,
        Row::StoreDailyCheck,
        Row::StoreClearCache,
        Row::SleepScreen,
        Row::KindleUi,
        Row::Restart,
        Row::LargeText,
        Row::HighContrast,
        Row::TextPreview,
        Row::LocalTime, Row::LocalDate, Row::TimeZone, Row::SyncClock,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Frontlight => "Front light",
            Row::Warmth => "Warmth",
            Row::DarkMode => "Dark mode",
            Row::WallpaperLink => "Wallpaper",
            Row::UsageLink => "Usage, last 7 days",
            Row::Wallpaper(target) => target.title(),
            Row::Animation => "Opening animation",
            Row::ScreenTimeout => "Sleep after",
            Row::UsageScreen => "Screen on",
            Row::UsageApp(_) => "App",
            Row::Wifi => "Wi-Fi",
            Row::Rotation => "Rotation",
            Row::Ssh => "SSH server (port 2222)",
            Row::SshPassword => "Password",
            Row::SshAddress => "Connect with",
            Row::SshCheck => "Check the server",
            Row::PublicDns => "Public DNS",
            Row::Autostart => "Start with the Kindle",
            Row::Hidden => "Hidden apps",
            Row::AppVisibility => "App visibility",
            Row::ManageInstalled => "Installed apps",
            Row::GridColumns => "Grid columns",
            Row::GridRows => "Grid rows",
            Row::BatteryReading => "Battery level",
            Row::LowPower => "Low power mode",
            Row::BatteryDisplay => "Front light",
            Row::BatteryWifi => "Wi-Fi",
            Row::StoreSettings => "Store settings",
            Row::StorageUsage => "Used storage",
            Row::DeviceAbout => "Device information",
            Row::DeviceModel => "Kindle model",
            Row::DeviceName => "Device name",
            Row::Model(i) => crate::models::MODELS[i as usize].name,
            Row::StoreDailyCheck => "Check for updates daily",
            Row::StoreClearCache => "Clear saved catalog",
            Row::SleepScreen => "Sleep screen",
            Row::KindleUi => "Kindle UI",
            Row::Restart => "Restart the launcher",
            Row::LargeText => "Larger text",
            Row::HighContrast => "Higher contrast",
            Row::TextPreview => "Preview",
            Row::LocalTime => "Local time",
            Row::LocalDate => "Date",
            Row::TimeZone => "Current time zone",
            Row::SyncClock => "Internet time",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRow {
    pub row: Row,
    pub rect: Rect,
    pub buttons: Vec<(Button, Rect)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub cols: i32,
    pub rows: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallpaperPreviewLayout {
    pub picture: Rect,
    pub choice_label: Rect,
    pub opacity_label: Rect,
    pub opacity_down: Rect,
    pub opacity_up: Rect,
    pub opacity_track: Rect,
    pub apply: Rect,
}

/// The status bar and the quick settings header have the same height, so their clock, Wi-Fi and battery line up.
pub const STATUS_H: i32 = 120;
/// The row of page dots (and notices) at the bottom of the home screen.
pub const FOOTER_H: i32 = 90;
pub const MARGIN: i32 = 40;
/// The home grid's side margin.
const GRID_SIDE: i32 = 24;
/// Icons are rounded squares of one size: at most this many pixels.
const ICON_MAX: i32 = 184;
/// Gap between an icon and its name.
const LABEL_GAP: i32 = 12;
/// Label text size and line height under icons (one line).
pub const LABEL_SIZE: f32 = 30.0;
pub const LABEL_LINE: i32 = 40;
/// Quick settings, as Primitive Browser's panel: a sheet from the top edge with the clock, Wi-Fi and battery, two
/// level rows of 25 steps (− and + at the ends), a grid of cards and a Close handle.
const QUICK_HEAD_H: i32 = STATUS_H;
/// The row of round icon buttons, with their small captions.
const QUICK_BUTTONS_H: i32 = 170;
/// The diameter of a round icon button.
const QUICK_BUTTON_D: i32 = 100;
const QUICK_ROW_H: i32 = 156;
const QUICK_HANDLE_H: i32 = 72;
/// The system section under the level rows: a header, then one row each for CPU, memory and storage.
const QUICK_SYSTEM_TOP: i32 = 56;
const QUICK_STAT_H: i32 = 76;
const QUICK_SIDE: i32 = 28;
const QUICK_NUDGE: i32 = 72;
/// The level rows' range: 25 steps, 0 to 24 (the Kindle's frontlight and warmth levels).
pub const QUICK_LEVEL_MAX: u32 = 24;
/// The app menu (long press): a box beside the icon, one row per action under the app's name.
const MENU_W: i32 = 600;
const MENU_ROW_H: i32 = 100;
const TITLE_H: i32 = 130;
/// One row of the Extensions list.
const LIST_ROW_H: i32 = 120;
/// One setting row (a switch, a value, or two round buttons), and one Settings list row (title and value).
const ROW_H: i32 = 120;
const HOME_ROW_H: i32 = 120;
/// Settings sections: the first header's top, a header, the space after a footer, and the footer's height.
const SECTION_TOP: i32 = 24;
const SECTION_HEAD_H: i32 = 56;
const SECTION_GAP: i32 = 24;
/// The current network's card (one row), and one network of the scan list.
const CURRENT_H: i32 = 112;
/// One row of the Wi-Fi scan list (pixels).
pub const NETWORK_H: i32 = 104;
/// The switch of a toggle row: its size and its inset from the row's right edge.
pub const SWITCH_W: i32 = 112;
pub const SWITCH_H: i32 = 64;
const SWITCH_INSET: i32 = 40;
/// The round − and + buttons of a level row.
const ROUND_D: i32 = 72;

/// The space a section's footer takes (zero when it has none), with the gap after the section.
fn section_foot(footer: &str) -> i32 {
    if footer.is_empty() { SECTION_GAP } else { 76 + SECTION_GAP }
}

fn setting_level_nudges(row: Rect) -> (Rect, Rect) {
    let y = row.y + (row.h - ROUND_D) / 2;
    let minus = Rect::new(row.x + row.w / 3, y, ROUND_D, ROUND_D);
    let plus = Rect::new(row.right() - SWITCH_INSET - ROUND_D, y, ROUND_D, ROUND_D);
    (minus, plus)
}

/// The directly selectable part of a front-light or warmth row in Settings.
pub fn setting_level_bar(row: Rect) -> Rect {
    let (minus, plus) = setting_level_nudges(row);
    let x = minus.right() + 12;
    Rect::new(x, minus.y, (plus.x - 12 - x).max(0), minus.h)
}

fn level_step(bar: Rect, x: f32, y: f32) -> Option<u32> {
    let zone = Rect::new(bar.x, bar.y - 16, bar.w, bar.h + 32);
    if !zone.contains(x, y) {
        return None;
    }
    let t = ((x - bar.x as f32) / bar.w as f32).clamp(0.0, 0.9999);
    Some((t * (QUICK_LEVEL_MAX + 1) as f32) as u32)
}

/// The keyboard's Go key on the password screen.
const KEYBOARD_GO: &str = "Join";

/// The Wi-Fi page's own area: the current network, and the scan list under its "Networks" header.
pub struct WifiArea {
    pub current_header: i32,
    pub current: Rect,
    pub networks_header: i32,
    pub scan: Rect,
    pub list: Rect,
    /// How many networks fit in the list.
    pub visible: usize,
}

/// The buttons of a setting row, in the row's rectangle: a switch on the right for on/off rows, round buttons for a
/// level, the whole row for a row that opens or runs something, and the left part of a row with a switch for the
/// rotation (the value turns the screen by hand).
fn row_buttons(l: &Layout, row: Row, r: Rect) -> Vec<(Button, Rect)> {
    let switch = Rect::new(r.right() - SWITCH_INSET - SWITCH_W, r.y + (r.h - SWITCH_H) / 2, SWITCH_W, SWITCH_H);
    let (minus, plus) = setting_level_nudges(r);
    let _ = l;
    match row {
        Row::Frontlight => vec![(Button::LightDown, minus), (Button::LightUp, plus)],
        Row::Warmth => vec![(Button::WarmDown, minus), (Button::WarmUp, plus)],
        Row::Rotation => vec![
            (Button::Rotation, Rect::new(r.x, r.y, r.w - SWITCH_W - 2 * SWITCH_INSET, r.h)),
            (Button::RotationAuto, switch),
        ],
        Row::Hidden => vec![(Button::ShowHidden, Rect::new(r.right() - SWITCH_INSET - 200, r.y + (r.h - SWITCH_H) / 2, 200, SWITCH_H))],
        Row::AppVisibility => vec![(Button::OpenPage(Page::AppVisibility), r)],
        Row::WallpaperLink => vec![(Button::OpenPage(Page::Wallpaper), r)],
        Row::UsageLink => vec![(Button::OpenPage(Page::Usage), r)],
        Row::Wallpaper(target) => vec![(Button::OpenWallpaperPicker(target), r)],
        Row::ManageInstalled => vec![(Button::ManageInstalled, r)],
        Row::GridColumns => vec![(Button::GridColumns, r)],
        Row::GridRows => vec![(Button::GridRows, r)],
        Row::LowPower => vec![(Button::LowPower, r)],
        Row::BatteryDisplay => vec![(Button::OpenPage(Page::Display), r)],
        Row::BatteryWifi => vec![(Button::OpenPage(Page::Wifi), r)],
        Row::StoreSettings => vec![(Button::OpenPage(Page::Store), r)],
        Row::DeviceAbout => vec![(Button::OpenPage(Page::About), r)],
        Row::DeviceModel => vec![(Button::OpenPage(Page::ChooseModel), r)],
        Row::DeviceName => vec![(Button::DeviceName, r)],
        Row::Model(i) => vec![(Button::PickModel(i), r)],
        Row::BatteryReading | Row::StorageUsage => vec![],
        Row::StoreDailyCheck => vec![(Button::StoreDailyCheck, r)],
        Row::StoreClearCache => vec![(Button::StoreClearCache, r)],
        Row::KindleUi => vec![(Button::KindleUi, r)],
        Row::Restart => vec![(Button::Restart, r)],
        Row::Wifi => vec![(Button::Wifi, r)],
        Row::DarkMode => vec![(Button::DarkMode, r)],
        Row::Animation => vec![(Button::Animation, r)],
        Row::ScreenTimeout => vec![(Button::ScreenTimeout, r)],
        Row::UsageScreen | Row::UsageApp(_) => vec![],
        Row::Ssh => vec![(Button::Ssh, r)],
        Row::SshPassword => vec![(Button::SshPassword, r)],
        Row::SshAddress => vec![],
        Row::SshCheck => vec![(Button::SshCheck, r)],
        Row::PublicDns => vec![(Button::PublicDns, r)],
        Row::Autostart => vec![(Button::Autostart, r)],
        Row::SleepScreen => vec![(Button::SleepScreen, r)],
        Row::LargeText => vec![(Button::LargeText, r)],
        Row::HighContrast => vec![(Button::HighContrast, r)],
        Row::TextPreview => vec![],
        Row::LocalTime | Row::LocalDate => vec![],
        Row::SyncClock => vec![(Button::SyncClock, r)],
        Row::TimeZone => vec![(Button::RefreshTimeZone, r)],
    }
}
const BUTTON_H: i32 = 100;
/// The store's tab bar, its list rows and its information rows.
const STORE_TAB_BAR_H: i32 = 130;
const STORE_ROW_H: i32 = 170;
const STORE_INFO_ROW_H: i32 = 100;

/// Where the product page's About text starts: under the screenshots when the app has them, else right under the strip.
fn store_about_top(shots: usize) -> i32 {
    if shots > 0 {
        1874
    } else {
        1290
    }
}

impl Layout {
    pub fn new(width: u32, height: u32, cols: u32, rows: u32) -> Layout {
        Layout { width: width as i32, height: height as i32, cols: cols.clamp(3, 5) as i32, rows: rows.clamp(3, 6) as i32 }
    }

    pub fn screen(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn status(&self) -> Rect {
        Rect::new(0, 0, self.width, STATUS_H)
    }

    /// The app grid, between the status bar and the page dots.
    pub fn grid(&self) -> Rect {
        let top = STATUS_H + 20;
        Rect::new(GRID_SIDE, top, self.width - 2 * GRID_SIDE, self.home_footer().y - top)
    }

    /// The bottom bar of list screens (Extensions): page controls.
    pub fn footer(&self) -> Rect {
        Rect::new(0, self.height - 100, self.width, 100)
    }

    /// The row of page dots at the bottom of the home screen. Notices are shown here too.
    pub fn home_footer(&self) -> Rect {
        Rect::new(0, self.height - 20 - FOOTER_H, self.width, FOOTER_H)
    }

    /// The side of every icon: as large as the cell allows, at most `ICON_MAX`.
    pub fn icon_size(&self) -> i32 {
        let grid = self.grid();
        let (cw, ch) = (grid.w / self.cols, grid.h / self.rows);
        ICON_MAX.min(cw - 2 * MARGIN).min(ch - LABEL_GAP - LABEL_LINE - 16)
    }


    pub fn per_page(&self) -> usize {
        (self.cols * self.rows) as usize
    }

    pub fn pages(&self, count: usize) -> usize {
        count.div_ceil(self.per_page()).max(1)
    }

    /// The cell of slot `slot` (0-based on the page, row by row).
    pub fn cell(&self, slot: usize) -> Rect {
        let grid = self.grid();
        let (col, row) = (slot as i32 % self.cols, slot as i32 / self.cols);
        let (cw, ch) = (grid.w / self.cols, grid.h / self.rows);
        Rect::new(grid.x + col * cw, grid.y + row * ch, cw, ch)
    }

    /// The icon in a cell: a square of `icon_size`, centred with its name under it.
    pub fn icon(&self, slot: usize) -> Rect {
        let cell = self.cell(slot);
        let s = self.icon_size();
        let top = cell.y + (cell.h - s - LABEL_GAP - LABEL_LINE) / 2;
        Rect::new(cell.x + (cell.w - s) / 2, top, s, s)
    }

    /// The app's name under its icon: one line.
    pub fn label(&self, slot: usize) -> Rect {
        let cell = self.cell(slot);
        let icon = self.icon(slot);
        Rect::new(cell.x + 6, icon.bottom() + LABEL_GAP, cell.w - 12, LABEL_LINE)
    }

    /// The slot under a point on the home screen.
    pub fn slot_at(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.per_page()).find(|&s| self.cell(s).contains(x, y))
    }

    pub fn title(&self) -> Rect {
        Rect::new(0, 0, self.width, TITLE_H)
    }

    pub fn done_button(&self) -> Rect {
        Rect::new(self.width - MARGIN - 200, (TITLE_H - BUTTON_H) / 2, 200, BUTTON_H)
    }

    /// The sections of a Settings page, laid out one under another, each with its header and footer.
    pub fn page_sections(&self, page: Page) -> Vec<Section> {
        let w = self.width - 2 * MARGIN;
        let mut y = TITLE_H + SECTION_TOP;
        page.groups()
            .iter()
            .map(|g| {
                let header_y = y;
                y += SECTION_HEAD_H;
                let card = Rect::new(MARGIN, y, w, g.rows.len() as i32 * ROW_H);
                let rows = g
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(i, &row)| {
                        let rect = Rect::new(card.x, card.y + i as i32 * ROW_H, card.w, ROW_H);
                        SettingsRow { row, rect, buttons: row_buttons(self, row, rect) }
                    })
                    .collect();
                let footer_y = card.bottom() + 10;
                y = card.bottom() + section_foot(g.footer);
                Section { header: g.header, footer: g.footer, header_y, card, footer_y, rows }
            })
            .collect()
    }

    /// The y just under a page's sections (where a page's own content, such as the Wi-Fi list, can start).
    pub fn page_end(&self, page: Page) -> i32 {
        let mut y = TITLE_H + SECTION_TOP;
        for g in page.groups() {
            y += SECTION_HEAD_H + g.rows.len() as i32 * ROW_H + section_foot(g.footer);
        }
        y
    }

    pub fn settings_viewport(&self) -> Rect {
        Rect::new(0, TITLE_H, self.width, (self.height - TITLE_H - SECTION_GAP).max(0))
    }

    pub fn settings_scroll_max(&self) -> i32 {
        let groups_height: i32 = SETTINGS_GROUPS.iter()
            .map(|(_, pages)| SECTION_HEAD_H + pages.len() as i32 * HOME_ROW_H + SECTION_GAP)
            .sum();
        (SECTION_TOP + groups_height - self.settings_viewport().h).max(0)
    }

    /// The Settings list: the groups of pages, each a header and a card with one row per page.
    pub fn home_sections(&self, scroll: i32) -> Vec<HomeSection> {
        let w = self.width - 2 * MARGIN;
        let mut y = TITLE_H + SECTION_TOP - scroll;
        SETTINGS_GROUPS
            .iter()
            .map(|&(header, pages)| {
                y += SECTION_HEAD_H;
                let card = Rect::new(MARGIN, y, w, pages.len() as i32 * HOME_ROW_H);
                let rows = pages.iter().enumerate().map(|(i, &p)| (p, Rect::new(card.x, card.y + i as i32 * HOME_ROW_H, card.w, HOME_ROW_H))).collect();
                y = card.bottom() + SECTION_GAP;
                (header, card, rows)
            })
            .collect()
    }

    /// The Settings list's rows at this scroll position, including rows outside the viewport.
    pub fn settings_home_rows(&self, scroll: i32) -> Vec<(Page, Rect)> {
        self.home_sections(scroll).into_iter().flat_map(|(_, _, rows)| rows).collect()
    }

    /// The visible Settings list row under a point.
    pub fn settings_home_at(&self, scroll: i32, x: f32, y: f32) -> Option<Page> {
        if !self.settings_viewport().contains(x, y) {
            return None;
        }
        self.settings_home_rows(scroll).into_iter().find(|(_, r)| r.contains(x, y)).map(|(p, _)| p)
    }

    pub fn visibility_viewport(&self) -> Rect {
        // Keep space below the list for a save error.
        Rect::new(0, TITLE_H, self.width, (self.height - TITLE_H - 92).max(0))
    }

    pub fn visibility_row(&self, index: usize, scroll: i32) -> Rect {
        let row = self.list_row(index);
        Rect::new(row.x, row.y - scroll, row.w, row.h)
    }

    pub fn visibility_scroll_max(&self, count: usize) -> i32 {
        let count = i32::try_from(count).unwrap_or(i32::MAX);
        (count.saturating_mul(LIST_ROW_H).saturating_add(20) - self.visibility_viewport().h).max(0)
    }

    pub fn visibility_at(&self, scroll: i32, x: f32, y: f32) -> Option<usize> {
        let first = self.visibility_row(0, scroll);
        if !self.visibility_viewport().contains(x, y)
            || x < first.x as f32 || x >= first.right() as f32 || y < first.y as f32
        {
            return None;
        }
        Some(((y - first.y as f32) / first.h as f32) as usize)
    }

    pub fn installed_remove_button(&self, row: Rect) -> Rect {
        let width = 226;
        let height = 68;
        Rect::new(row.right() - 24 - width, row.y + (row.h - height) / 2, width, height)
    }

    /// A device-proportioned picture above the preview's opacity and Apply controls.
    pub fn wallpaper_preview(&self) -> WallpaperPreviewLayout {
        let width = self.width - 2 * MARGIN;
        let choice_label = Rect::new(MARGIN, TITLE_H + 8, width, 54);
        let apply_w = width.min(460);
        let apply = Rect::new((self.width - apply_w) / 2, self.height - MARGIN - BUTTON_H, apply_w, BUTTON_H);
        let opacity_y = apply.y - BUTTON_H - 20;
        let opacity_label = Rect::new(MARGIN, opacity_y - 56, width, 48);
        let opacity_down = Rect::new(MARGIN, opacity_y, BUTTON_H, BUTTON_H);
        let opacity_up = Rect::new(self.width - MARGIN - BUTTON_H, opacity_y, BUTTON_H, BUTTON_H);
        let track_x = opacity_down.right() + 16;
        let opacity_track = Rect::new(track_x, opacity_y, opacity_up.x - 16 - track_x, BUTTON_H);

        let picture_y = choice_label.bottom() + 16;
        let max_picture_h = (opacity_label.y - 20 - picture_y).max(0);
        let picture_w = (max_picture_h * self.width / self.height).min(width);
        let picture_h = picture_w * self.height / self.width;
        let picture = Rect::new((self.width - picture_w) / 2, picture_y, picture_w, picture_h);
        WallpaperPreviewLayout { picture, choice_label, opacity_label, opacity_down, opacity_up, opacity_track, apply }
    }

    /// The Wi-Fi page's own area under its Wi-Fi section: the current network, then the scan list.
    pub fn wifi_area(&self) -> WifiArea {
        let top = self.page_end(Page::Wifi);
        let w = self.width - 2 * MARGIN;
        let current_header = top;
        let current = Rect::new(MARGIN, top + SECTION_HEAD_H, w, CURRENT_H);
        let networks_header = current.bottom() + SECTION_GAP;
        let list_top = networks_header + SECTION_HEAD_H;
        // The list ends above the Wi-Fi page's notice line (the result of a join or a forget).
        let bottom = self.height - 110;
        let list = Rect::new(MARGIN, list_top, w, (bottom - list_top).max(NETWORK_H));
        let scan = Rect::new(self.width - MARGIN - 200, networks_header - 4, 200, 52);
        WifiArea { current_header, current, networks_header, scan, list, visible: (list.h / NETWORK_H).max(1) as usize }
    }

    /// Network row `i` of the visible scan list.
    pub fn network_row(&self, i: usize) -> Rect {
        let list = self.wifi_area().list;
        Rect::new(list.x, list.y + i as i32 * NETWORK_H, list.w, NETWORK_H)
    }

    /// The visible network row under a point (its index within the window), if any.
    pub fn network_at(&self, x: f32, y: f32) -> Option<usize> {
        let visible = self.wifi_area().visible;
        (0..visible).find(|&i| self.network_row(i).contains(x, y))
    }

    /// Quick settings: a sheet from the top edge.
    pub fn quick_panel(&self) -> Rect {
        let h = QUICK_HEAD_H + QUICK_BUTTONS_H + 2 * QUICK_ROW_H + QUICK_SYSTEM_TOP + 3 * QUICK_STAT_H + 20 + QUICK_HANDLE_H;
        Rect::new(0, 0, self.width, h)
    }

    /// The clock and date, top left (the status bar and quick settings).
    pub fn header_clock(&self) -> Rect {
        Rect::new(QUICK_SIDE, 0, 400, QUICK_HEAD_H)
    }

    /// The battery, top right (the status bar and quick settings).
    pub fn header_battery(&self) -> Rect {
        Rect::new(self.width - QUICK_SIDE - 128, (QUICK_HEAD_H - 56) / 2, 128, 56)
    }

    /// The Wi-Fi icon, left of the battery (the status bar and quick settings): a tap in quick settings turns Wi-Fi on
    /// or off.
    pub fn header_wifi(&self) -> Rect {
        let b = self.header_battery();
        Rect::new(b.x - 26 - 64, (QUICK_HEAD_H - 64) / 2, 64, 64)
    }

    /// Level row `i` (0 brightness, 1 warmth), under the icon buttons.
    pub fn quick_row(&self, i: usize) -> Rect {
        Rect::new(0, QUICK_HEAD_H + QUICK_BUTTONS_H + i as i32 * QUICK_ROW_H, self.width, QUICK_ROW_H)
    }

    /// The − button of level row `i`.
    pub fn quick_minus(&self, i: usize) -> Rect {
        let r = self.quick_row(i);
        Rect::new(QUICK_SIDE, r.y + 74, QUICK_NUDGE, QUICK_NUDGE)
    }

    /// The + button of level row `i`.
    pub fn quick_plus(&self, i: usize) -> Rect {
        let r = self.quick_row(i);
        Rect::new(self.width - QUICK_SIDE - QUICK_NUDGE, r.y + 74, QUICK_NUDGE, QUICK_NUDGE)
    }

    /// The 25 steps of level row `i`, between the buttons.
    pub fn quick_bar(&self, i: usize) -> Rect {
        let minus = self.quick_minus(i);
        let gap = 12;
        Rect::new(minus.right() + gap, minus.y, self.width - 2 * (QUICK_SIDE + QUICK_NUDGE + gap), QUICK_NUDGE)
    }

    /// The level (0 to `QUICK_LEVEL_MAX`) of the step under (`x`, `y`) on row `i`'s bar, or `None` off the bar.
    pub fn quick_step(&self, i: usize, x: f32, y: f32) -> Option<u32> {
        level_step(self.quick_bar(i), x, y)
    }

    /// The "System" header above the figures.
    pub fn quick_system_header(&self) -> i32 {
        self.quick_row(1).bottom() + 8
    }

    /// System row `i` (0 CPU, 1 memory, 2 storage) under the level rows.
    pub fn quick_stat_row(&self, i: usize) -> Rect {
        let top = self.quick_row(1).bottom() + QUICK_SYSTEM_TOP;
        Rect::new(0, top + i as i32 * QUICK_STAT_H, self.width, QUICK_STAT_H)
    }

    /// The six round icon buttons in one row, under the header: Airplane, Auto-rotate, Dark mode, Animation, SSH,
    /// All settings. Each slot is a column of the row; the circle is `QUICK_BUTTON_D` wide, centred in it.
    pub fn quick_buttons(&self) -> [(QuickTile, Rect); 8] {
        let slot = (self.width - 2 * QUICK_SIDE) / 8;
        let at = |i: i32| Rect::new(QUICK_SIDE + i * slot, QUICK_HEAD_H, slot, QUICK_BUTTONS_H);
        [
            (QuickTile::Airplane, at(0)),
            (QuickTile::AutoRotate, at(1)),
            (QuickTile::DarkMode, at(2)),
            (QuickTile::Animation, at(3)),
            (QuickTile::ScreenTimeout, at(4)),
            (QuickTile::Ssh, at(5)),
            (QuickTile::AllSettings, at(6)),
            (QuickTile::Power, at(7)),
        ]
    }

    /// The circle of a quick settings button, inside its slot.
    pub fn quick_button_circle(&self, slot: Rect) -> Rect {
        let d = QUICK_BUTTON_D;
        Rect::new(slot.x + (slot.w - d) / 2, slot.y + 8, d, d)
    }

    /// The Close handle at the foot of quick settings.
    pub fn quick_handle(&self) -> Rect {
        let p = self.quick_panel();
        Rect::new(0, p.bottom() - QUICK_HANDLE_H, p.w, QUICK_HANDLE_H)
    }

    /// The app menu's box for an icon at `anchor` with `actions` rows under the name row: below the icon
    /// when it fits, else above it; inside the screen.
    pub fn menu_box(&self, anchor: Rect, actions: usize) -> Rect {
        let h = (actions as i32 + 1) * MENU_ROW_H;
        let x = (anchor.x + anchor.w / 2 - MENU_W / 2).clamp(MARGIN, self.width - MARGIN - MENU_W);
        // Below the icon's name, so the name stays readable.
        let below = anchor.bottom() + LABEL_GAP + LABEL_LINE + 16;
        let y = if below + h <= self.height - 20 { below } else { (anchor.y - 24 - h).max(STATUS_H) };
        Rect::new(x, y, MENU_W, h)
    }

    /// Row `i` of the app menu's actions (row 0 of the box is the app's name).
    pub fn menu_row(&self, anchor: Rect, actions: usize, i: usize) -> Rect {
        let b = self.menu_box(anchor, actions);
        Rect::new(b.x, b.y + (i as i32 + 1) * MENU_ROW_H, b.w, MENU_ROW_H)
    }

    /// Rows of the Extensions list on one page (between the title and the footer).
    pub fn list_rows_per_page(&self) -> usize {
        ((self.footer().y - TITLE_H - 20) / LIST_ROW_H).max(1) as usize
    }

    /// Row `i` of the Extensions list on the current page.
    pub fn list_row(&self, i: usize) -> Rect {
        Rect::new(MARGIN, TITLE_H + 20 + i as i32 * LIST_ROW_H, self.width - 2 * MARGIN, LIST_ROW_H)
    }

    pub fn list_row_at(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.list_rows_per_page()).find(|&i| self.list_row(i).contains(x, y))
    }

    /// Pages of the Extensions list for `n` items (at least one).
    pub fn list_pages(&self, n: usize) -> usize {
        n.div_ceil(self.list_rows_per_page()).max(1)
    }

    /// The button of a Settings page under a point (its rows only), or Done.
    pub fn settings_button_at(&self, page: Page, x: f32, y: f32) -> Option<Button> {
        if self.done_button().inset(-20).contains(x, y) {
            return Some(Button::Done);
        }
        self.page_sections(page)
            .into_iter()
            .flat_map(|s| s.rows)
            .flat_map(|r| r.buttons)
            .find(|(_, rect)| rect.inset(-12).contains(x, y))
            .map(|(b, _)| b)
    }

    pub fn display_level_at(&self, x: f32, y: f32) -> Option<(Row, u32)> {
        self.page_sections(Page::Display)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter(|setting| matches!(setting.row, Row::Frontlight | Row::Warmth))
            .find_map(|setting| level_step(setting_level_bar(setting.rect), x, y).map(|level| (setting.row, level)))
    }

    /// The password field: the network's password, shown as dots unless "Show password" is on.
    pub fn password_field(&self) -> Rect {
        Rect::new(MARGIN, TITLE_H + 110, self.width - 2 * MARGIN, 100)
    }

    /// The "Show password" row under the field, with its switch.
    pub fn password_show(&self) -> Rect {
        let f = self.password_field();
        Rect::new(f.x, f.bottom() + 14, f.w, 80)
    }

    /// Where the password screen's notice sits: above the keyboard.
    pub fn password_notice(&self) -> Rect {
        Rect::new(MARGIN, self.keyboard_area().y - 80, self.width - 2 * MARGIN, 60)
    }

    /// The keyboard's area: the bottom of the screen, as tall as Primitive Browser's (the shared `style`).
    pub fn keyboard_area(&self) -> Rect {
        let h = primitive_keyboard::style::AREA_H as i32;
        Rect::new(0, self.height - h, self.width, h)
    }

    /// Every key of a keyboard layer with its rectangle and definition: the shared layout (`primitive-keyboard`), the
    /// same rows, sizes and gaps as Primitive Browser's keyboard, at the bottom of the screen.
    pub fn keyboard_keys(&self, layer: primitive_keyboard::Layer) -> Vec<(primitive_keyboard::KeyDef, Rect)> {
        self.keyboard_keys_with(layer, KEYBOARD_GO)
    }

    /// The keys with another label on the Go key ("Next" on the network name screen).
    pub fn keyboard_keys_with(&self, layer: primitive_keyboard::Layer, go: &str) -> Vec<(primitive_keyboard::KeyDef, Rect)> {
        let area = self.keyboard_area();
        primitive_keyboard::layout(layer, go, area.w as u32, area.h as u32)
            .into_iter()
            .map(|(key, r)| (key, Rect::new(area.x + r.x, area.y + r.y, r.w, r.h)))
            .collect()
    }

    /// The key under a point on a keyboard layer.
    pub fn keyboard_key_at(&self, layer: primitive_keyboard::Layer, x: f32, y: f32) -> Option<primitive_keyboard::Key> {
        self.keyboard_keys(layer).into_iter().find(|(_, r)| r.inset(-4).contains(x, y)).map(|(k, _)| k.key)
    }

    /// The Wi-Fi page's "Other…" button, left of Scan: join a network that is not in the list (a hidden one).
    pub fn wifi_other(&self) -> Rect {
        let scan = self.wifi_area().scan;
        Rect::new(scan.x - 20 - 200, scan.y, 200, scan.h)
    }

    /// The Connect and Forget buttons of the network screen (Forget only for a saved network).
    pub fn net_connect(&self) -> Rect {
        // Under the network's card: its title area, 40 px, three rows of 96 px, 40 px.
        Rect::new(MARGIN, TITLE_H + 40 + 3 * 96 + 40, self.width - 2 * MARGIN, 100)
    }

    pub fn net_forget(&self) -> Rect {
        let c = self.net_connect();
        Rect::new(c.x, c.bottom() + 28, c.w, 100)
    }

    /// The Wi-Fi page's scan button under a point.
    pub fn scan_at(&self, x: f32, y: f32) -> bool {
        self.wifi_area().scan.inset(-12).contains(x, y)
    }

    pub fn confirm_box(&self) -> Rect {
        Rect::new(MARGIN * 2, self.height / 2 - 300, self.width - 4 * MARGIN, 600)
    }

    /// The power menu's box: the same place as the Kindle UI dialog.
    pub fn power_box(&self) -> Rect {
        self.confirm_box()
    }

    /// The power menu's four buttons, in a row at the bottom of its box: Cancel, Kindle UI, Restart, Power off.
    pub fn power_buttons(&self) -> [(Button, Rect); 4] {
        let b = self.power_box();
        let gap = 20;
        let bw = (b.w - 80 - 3 * gap) / 4;
        let y = b.bottom() - 40 - BUTTON_H;
        let x = |i: i32| b.x + 40 + i * (bw + gap);
        [
            (Button::PowerCancel, Rect::new(x(0), y, bw, BUTTON_H)),
            (Button::PowerKindleUi, Rect::new(x(1), y, bw, BUTTON_H)),
            (Button::PowerRestart, Rect::new(x(2), y, bw, BUTTON_H)),
            (Button::PowerOff, Rect::new(x(3), y, bw, BUTTON_H)),
        ]
    }

    // ---- Store (App Store style): a large title, a floating tab bar at the bottom, and product pages that scroll.

    /// The floating tab bar, with its four tabs.
    pub fn store_tabs(&self) -> Rect {
        Rect::new(40, self.height - 24 - STORE_TAB_BAR_H, self.width - 80, STORE_TAB_BAR_H)
    }

    /// Tab `i`: an equal part of the bar, inset a little.
    pub fn store_tab(&self, i: usize) -> Rect {
        let t = self.store_tabs();
        let w = t.w / crate::store::StoreTab::ALL.len() as i32;
        Rect::new(t.x + i as i32 * w + 10, t.y + 10, w - 20, t.h - 20)
    }

    pub fn store_tab_at(&self, x: f32, y: f32) -> Option<usize> {
        (0..crate::store::StoreTab::ALL.len()).find(|&i| self.store_tab(i).contains(x, y))
    }

    /// The Done button, at the top right of every store tab: it returns to the home screen.
    pub fn store_done(&self) -> Rect {
        Rect::new(self.width - 40 - 200, 62, 200, 84)
    }

    /// The Refresh button, at the top of every store tab, left of Done.
    pub fn store_refresh(&self) -> Rect {
        Rect::new(self.width - 40 - 200 - 24 - 240, 62, 240, 84)
    }

    /// The large title at the top of a store tab ("Today", "Apps", "Updates").
    pub fn store_title(&self) -> Rect {
        Rect::new(40, 50, self.width - 80, 110)
    }

    pub fn store_settings_button(&self) -> Rect {
        let refresh = self.store_refresh();
        Rect::new(refresh.x - 134, refresh.y - 8, 100, 100)
    }

    pub fn store_settings_box(&self) -> Rect {
        self.confirm_box()
    }

    pub fn store_daily_check_button(&self) -> Rect {
        let panel = self.store_settings_box();
        Rect::new(panel.x + 40, panel.y + 140, panel.w - 80, 120)
    }

    pub fn store_clear_button(&self) -> Rect {
        let panel = self.store_settings_box();
        Rect::new(panel.x + 40, panel.y + 280, panel.w - 80, BUTTON_H)
    }

    pub fn store_collection_card(&self, index: usize) -> Rect {
        let left = self.store_feature().x;
        let width = (self.width - 104) / 2;
        let top = self.store_feature().bottom() + 120;
        Rect::new(left + (index % 2) as i32 * (width + 24), top + (index / 2) as i32 * 248, width, 224)
    }

    pub fn store_category_cell(&self, index: usize) -> Rect {
        let panel = self.store_sheet();
        let width = (panel.w - 104) / 2;
        Rect::new(panel.x + 40 + (index % 2) as i32 * (width + 24), panel.y + 116 + (index / 2) as i32 * 100, width, 100)
    }

    pub fn store_toolbar_button(&self) -> Rect {
        Rect::new(self.width - 40 - 260, 200, 260, 100)
    }

    pub fn store_toolbar_row(&self, i: usize) -> Rect {
        let row = self.store_list_row(i);
        Rect::new(row.x, row.y + 120, row.w, row.h)
    }

    /// Where a tab's content ends: above the tab bar.
    pub fn store_body_bottom(&self) -> i32 {
        self.store_tabs().y - 12
    }

    /// Today's featured card: the first app, large, in black.
    pub fn store_feature(&self) -> Rect {
        Rect::new(40, 200, self.width - 80, 470)
    }

    /// "See All" beside the Apps heading under the featured card (it goes to the Apps tab).
    pub fn store_see_all(&self) -> Rect {
        let y = self.store_feature().bottom() + 40;
        Rect::new(self.width - 40 - 260, y, 260, 60)
    }

    /// Today's rows, below the Apps heading.
    pub fn store_today_row(&self, i: usize) -> Rect {
        let top = self.store_feature().bottom() + 120;
        Rect::new(40, top + i as i32 * STORE_ROW_H, self.width - 80, STORE_ROW_H)
    }

    /// Unscrolled row `i` of the Apps or Updates list.
    pub fn store_list_row(&self, i: usize) -> Rect {
        Rect::new(40, 200 + i as i32 * STORE_ROW_H, self.width - 80, STORE_ROW_H)
    }

    pub fn store_search_field(&self) -> Rect {
        Rect::new(40, 200, self.width - 80, 104)
    }

    pub fn store_search_row(&self, i: usize) -> Rect {
        Rect::new(40, self.store_search_field().bottom() + 16 + i as i32 * STORE_ROW_H, self.width - 80, STORE_ROW_H)
    }

    pub fn store_row_action(row: Rect) -> Rect {
        Rect::new(row.right() - 190, row.y + (row.h - 76) / 2, 190, 76)
    }

    // Product page. The header (back) stays; everything below it scrolls by `scroll` pixels.

    pub fn store_back(&self) -> Rect {
        Rect::new(40, 50, 260, 90)
    }

    /// The icon, centred.
    pub fn store_hero(&self, scroll: i32) -> Rect {
        Rect::new(self.width / 2 - 170, 190 - scroll, 340, 340)
    }

    /// The name, centred, under the icon.
    pub fn store_name(&self, scroll: i32) -> Rect {
        Rect::new(40, 570 - scroll, self.width - 80, 70)
    }

    pub fn store_subtitle(&self, scroll: i32) -> Rect {
        Rect::new(40, 650 - scroll, self.width - 80, 50)
    }

    /// The action pill (GET, UPDATE, OPEN, VIEW), centred.
    pub fn store_action(&self, scroll: i32) -> Rect {
        Rect::new(self.width / 2 - 160, 770 - scroll, 320, 100)
    }

    /// The developer line under the pill.
    pub fn store_developer(&self, scroll: i32) -> Rect {
        Rect::new(40, 895 - scroll, self.width - 80, 40)
    }

    /// Product summary cells for version, size, firmware, and downloads.
    pub fn store_info_cells(&self, scroll: i32) -> [Rect; 4] {
        let w = (self.width - 80) / 4;
        let y = 980 - scroll;
        [0, 1, 2, 3].map(|i| Rect::new(40 + i * w, y, w, 150))
    }

    /// A 632 × 840 screenshot shown at 540 px high.
    pub fn store_shot(&self, scroll: i32) -> Rect {
        let h = 540;
        let w = h * 632 / 840;
        Rect::new((self.width - w) / 2, 1190 - scroll, w, h)
    }

    pub fn store_shot_button(&self, scroll: i32, forward: bool) -> Rect {
        let shot = self.store_shot(scroll);
        let width = 200;
        let x = if forward { shot.right() + 48 } else { shot.x - 48 - width };
        Rect::new(x, shot.y + (shot.h - BUTTON_H) / 2, width, BUTTON_H)
    }

    /// Where the About text starts: below the screenshots when there are any (`shots` 1 or 2), else up under the strip.
    /// Its heading sits 74 px above it.
    pub fn store_about(&self, scroll: i32, shots: usize) -> Rect {
        Rect::new(40, store_about_top(shots) - scroll, self.width - 80, 300)
    }

    /// Information row `i` (developer, licence, category, firmware, tested on, repository, then the changes).
    pub fn store_version_row(&self, i: usize, scroll: i32, shots: usize) -> Rect {
        let gap = if i > 0 { 76 } else { 0 };
        let top = store_about_top(shots) + 418 + gap + i as i32 * 210 - scroll;
        Rect::new(40, top, self.width - 80, 210)
    }

    fn store_information_top(shots: usize, versions: usize) -> i32 {
        store_about_top(shots) + 426 + if versions == 0 { 0 } else {
            versions as i32 * 210 + 70 + if versions > 1 { 76 } else { 0 }
        }
    }

    pub fn store_info_row(&self, i: usize, scroll: i32, shots: usize, versions: usize) -> Rect {
        let top = Self::store_information_top(shots, versions) - scroll;
        Rect::new(40, top + i as i32 * STORE_INFO_ROW_H, self.width - 80, STORE_INFO_ROW_H)
    }

    /// The height of a product page's content (for the scroll limit), with `rows` information rows.
    pub fn store_page_height(&self, rows: usize, shots: usize, versions: usize) -> i32 {
        Self::store_information_top(shots, versions) + rows as i32 * STORE_INFO_ROW_H + 130
    }

    pub fn confirm_buttons(&self) -> [(Button, Rect); 2] {
        let b = self.confirm_box();
        let y = b.bottom() - 40 - BUTTON_H;
        [
            (Button::ConfirmCancel, Rect::new(b.x + 40, y, 300, BUTTON_H)),
            (Button::ConfirmOpen, Rect::new(b.right() - 40 - 400, y, 400, BUTTON_H)),
        ]
    }

    pub fn store_sheet(&self) -> Rect {
        let height = 760;
        Rect::new(MARGIN * 2, (self.height - height) / 2, self.width - 4 * MARGIN, height)
    }

    pub fn store_sheet_buttons(&self) -> [Rect; 2] {
        let b = self.store_sheet();
        let y = b.bottom() - 40 - BUTTON_H;
        let width = (b.w - 104) / 2;
        [Rect::new(b.x + 40, y, width, BUTTON_H), Rect::new(b.right() - 40 - width, y, width, BUTTON_H)]
    }

    pub fn store_requirements_nav(&self) -> [Rect; 2] {
        let b = self.store_sheet().inset(48);
        let y = b.y + 484;
        [Rect::new(b.x, y, 220, 85), Rect::new(b.right() - 220, y, 220, 85)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oasis() -> Layout {
        Layout::new(1264, 1680, 4, 5)
    }

    #[test]
    fn icons_are_equal_squares_on_an_even_grid() {
        let l = oasis();
        assert_eq!(l.per_page(), 20);
        assert!(l.cell(0).y > STATUS_H);
        assert!(l.cell(19).bottom() <= l.home_footer().y);
        assert!(l.home_footer().bottom() <= l.height);
        let s = l.icon_size();
        assert!(s >= 160, "icons {s} px");
        for slot in 0..20 {
            let (icon, cell, label) = (l.icon(slot), l.cell(slot), l.label(slot));
            assert_eq!((icon.w, icon.h), (s, s));
            assert!(icon.x >= cell.x && icon.right() <= cell.right());
            assert!(label.bottom() <= cell.bottom(), "label inside cell {slot}");
            // Columns and rows line up: the same x in a column, the same y in a row.
            assert_eq!(icon.x, l.icon(slot % 4).x);
            assert_eq!(icon.y, l.icon(slot / 4 * 4).y);
        }
    }

    #[test]
    fn the_menu_box_opens_below_or_above_the_icon_inside_the_screen() {
        let l = oasis();
        let top = l.icon(0);
        let b = l.menu_box(top, 5);
        assert!(b.y > top.bottom() && b.x >= MARGIN && b.right() <= l.width - MARGIN);
        let last = l.icon(19);
        let b = l.menu_box(last, 5);
        assert!(b.bottom() < last.y, "above an icon in the last row: {b:?}");
        assert_eq!(l.menu_row(last, 5, 0).y, b.y + MENU_ROW_H);
    }

    #[test]
    fn the_zoom_grows_from_the_icon_to_the_screen() {
        let l = oasis();
        let icon = l.icon(5);
        let steps = zoom_steps(icon, l.screen(), 3);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[2], l.screen());
        assert!(steps[0].w > icon.w && steps[0].w < steps[1].w);
        // Eased out: the first step covers more than a third of the way.
        assert!(steps[0].w - icon.w > (l.width - icon.w) / 3);
    }

    #[test]
    fn quick_settings_fit_on_the_screen() {
        let l = oasis();
        let p = l.quick_panel();
        assert!(p.bottom() < l.height - 400, "panel {p:?}");
        for (_, slot) in l.quick_buttons() {
            let c = l.quick_button_circle(slot);
            assert!(c.x >= 0 && c.right() <= l.width && c.bottom() <= l.quick_row(0).y, "button {c:?}");
        }
        assert!(l.quick_bar(1).bottom() <= l.quick_row(1).bottom());
        assert!(l.quick_plus(0).right() <= l.width - QUICK_SIDE);
        assert!(l.header_wifi().right() < l.header_battery().x);
        assert!(l.quick_stat_row(2).bottom() <= l.quick_handle().y, "the system rows sit above the handle");
        assert!(l.quick_stat_row(0).y > l.quick_row(1).bottom());
    }

    #[test]
    fn hit_testing_finds_slots() {
        let l = oasis();
        let c = l.cell(5);
        assert_eq!(l.slot_at((c.x + 5) as f32, (c.y + 5) as f32), Some(5));
        assert_eq!(l.slot_at(10.0, 10.0), None);
        assert_eq!(l.pages(0), 1);
        assert_eq!(l.pages(20), 1);
        assert_eq!(l.pages(21), 2);
    }

    #[test]
    fn the_battery_and_usage_pages_fit_without_scrolling() {
        let l = oasis();
        for page in [Page::Battery, Page::Usage] {
            let sections = l.page_sections(page);
            let last = sections.last().unwrap();
            assert!(last.card.bottom() < l.height - 300, "{page:?}: the last section must be on screen (bottom {}, height {})", last.card.bottom(), l.height);
        }
    }

    #[test]
    fn settings_pages_fit_and_buttons_hit() {
        let l = oasis();
        let sections = l.page_sections(Page::Display);
        assert_eq!(sections.len(), 4);
        let last = sections.last().unwrap();
        assert!(last.card.bottom() < l.height - 300, "display sections fit");
        let rows: Vec<_> = sections.iter().flat_map(|s| s.rows.iter()).collect();
        let (b, r) = rows[0].buttons[1];
        assert_eq!(l.settings_button_at(Page::Display, (r.x + 10) as f32, (r.y + 10) as f32), Some(b));
        let d = l.done_button();
        assert_eq!(l.settings_button_at(Page::Display, (d.x + 1) as f32, (d.y + 1) as f32), Some(Button::Done));
        let top = l.settings_home_rows(0);
        assert_eq!(top.len() + 3, Page::ALL.len()); // App visibility, Installed apps and Wallpaper are nested.
        let viewport = l.settings_viewport();
        assert!(top.last().unwrap().1.bottom() > viewport.bottom());
        let scroll = l.settings_scroll_max();
        let bottom = l.settings_home_rows(scroll);
        let (page, r) = *bottom.last().unwrap();
        assert_eq!(page, Page::About);
        assert!(r.bottom() <= viewport.bottom());
        assert_eq!(l.settings_home_at(scroll, (r.x + 5) as f32, (r.y + 5) as f32), Some(Page::About));
        assert_eq!(l.settings_home_at(scroll, 50.0, (viewport.y - 1) as f32), None);
        assert_eq!(l.visibility_at(200, 50.0, (l.visibility_viewport().y - 1) as f32), None);
        assert_eq!(l.visibility_at(200, 50.0, l.visibility_viewport().y as f32), Some(1));
    }

    #[test]
    fn the_wifi_page_has_room_for_a_scan_list() {
        let l = oasis();
        let area = l.wifi_area();
        assert!(area.current.y > l.page_end(Page::Wifi), "current network below the Wi-Fi section");
        assert!(area.visible >= 6, "visible {}", area.visible);
        assert!(area.list.bottom() <= l.height - 30);
        assert!(l.network_at((area.list.x + 5) as f32, (area.list.y + 5) as f32) == Some(0));
        assert!(l.scan_at((area.scan.x + 5) as f32, (area.scan.y + 5) as f32));
    }
}
