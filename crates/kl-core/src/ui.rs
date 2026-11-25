//! The launcher's UI state machine. Input comes in as gestures in screen coordinates (already turned
//! for the rotation); the result is an `Action` for the driver: redraw, launch an app, change a device
//! setting, or leave for the Kindle UI.

use std::collections::HashMap;

use crate::{
    apps::{extensions_app, sort_key, App, IconSource, Launch},
    kual::MenuItem,
    battery::{self, Warning},
    layout::{Button, Layout, Page, QuickTile, Rect},
    settings::Rotation,
    store::StoreTab,
    wallpaper::{WallpaperChoice, WallpaperDraft, WallpaperSettings, WallpaperTarget},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Settings,
    /// "Open the Kindle UI?" over the previous screen.
    ConfirmKindle,
    /// Restart the launcher after a double press of the power key (the open app is closed).
    ConfirmRestart,
    /// The power menu: Cancel, Restart, Power off (opened by the Power tile in quick settings).
    PowerMenu,
    Sleep,
    /// Edit one app (long press): a box beside its icon to move it, change its icon or hide it. The app is
    /// `Ui::menu`.
    AppMenu,
    /// The KUAL extensions list, with submenus.
    Extensions,
    /// Quick settings, dropped down from the status bar.
    QuickSettings,
    /// One page of Settings (Settings > page; Back returns to the Settings list).
    Page(Page),
    WallpaperPicker(WallpaperTarget),
    WallpaperPreview,
    /// One network from the Wi-Fi scan: connect, or forget a saved one.
    NetworkDetail,
    /// The password of a new secured network, typed on the on-screen keyboard.
    Password,
    /// The name of a network that is not in the list (a hidden network), typed on the keyboard.
    NetworkName,
    /// The SSH server's root password, typed on the keyboard.
    SshPassword,
    /// The owner's name for this Kindle, typed on the keyboard.
    DeviceName,
    /// Primitive Store: the list of the catalog's apps.
    Store,
    /// Primitive Store: one app's page (`Ui::store_open`).
    StoreApp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    Tap(f32, f32),
    LongTap(f32, f32),
    /// A swipe: the distance moved, and the height where the finger went down (pixels).
    Swipe { dx: f32, dy: f32, start_y: f32 },
    PageForward,
    PageBack,
    /// `powerd` is going to the screensaver.
    Sleep,
    /// `powerd` left the screensaver.
    Wake,
    /// Both page-turn buttons were held for two seconds: confirm, then restart the launcher.
    RestartHold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCmd {
    /// Choose model `index` of `models::MODELS` (saved in the settings).
    PickModel(u8),
    Light(i32),
    Warmth(i32),
    ToggleWifi,
    ToggleRotation,
    ToggleSsh,
    /// Settings > DNS: public DNS on or off (the network's servers back).
    ToggleDns,
    /// Power menu: restart the Kindle (`reboot`), or turn it off (`poweroff`).
    Restart,
    PowerOff,
    /// Settings > Device > Launcher: start the launcher again (the supervisor starts it, as after an install).
    RestartLauncher,
    /// Close the open app, then restart the launcher.
    QuitAppAndRestart,
    /// Primitive Store: install the entry at this index of `Ui::store` (its position in the list).
    StoreInstall(usize),
    /// Primitive Store: download the catalog again (the Refresh button).
    StoreRefresh,
    /// Primitive Store: remove the installed entry at this index (after a second tap on Remove).
    StoreRemove(usize),
    /// Open a Store-installed entry through the launcher.
    StoreOpen(usize),
    StoreUpdateAll,
    StoreToggleDailyCheck,
    StoreClearCache,
    ToggleAutostart,
    ToggleSleepScreen,
    ToggleLowPower,
    OpenInstalledApps,
    /// Uninstall the confirmed app at this index of the current installed list.
    RemoveInstalledApp(usize),
    ToggleDarkMode,
    ToggleAnimation,
    /// Step the screen timeout to the next choice (15 s, 30 s … never).
    CycleScreenTimeout,
    ToggleLargeText,
    ToggleHighContrast,
    SyncClock,
    RefreshTimeZone,
    /// Quick settings' sliders: a level from 0 to 24.
    SetLight(u32),
    SetWarmth(u32),
    /// Quick settings > Auto-rotate: off keeps the screen as it is now; on follows the turn again.
    ToggleAutoRotation,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    /// Draw the current screen. `full` asks for a flashing full refresh (screen changes that clear ghosting).
    Redraw { full: bool },
    /// Show the pressed tile, then hand the app to the supervisor.
    Launch(App),
    /// Leave: the supervisor starts the stock Kindle UI.
    KindleMode,
    /// Change a device setting, then redraw.
    Device(DeviceCmd),
    /// Hide (true) or show (false) apps by id: written to apps.toml, then redraw.
    SetHidden(Vec<(String, bool)>),
    SetGrid { columns: u32, rows: u32 },
    /// Save the grid order: (id, position) for every app the user can move. Written to apps.toml.
    SetOrder(Vec<(String, i64)>),
    /// Set (`Some`) or clear (`None`) an app's icon file. Written to apps.toml; the driver reloads the tile.
    SetIcon { id: String, icon: Option<String> },
    PreviewWallpaper { target: WallpaperTarget, choice: WallpaperChoice },
    SetWallpaper { target: WallpaperTarget, choice: WallpaperChoice, opacity: u8 },
    /// Run an app while the launcher stays in front (background mode); the driver shows its last output line.
    RunBackground(App),
    /// Scan for Wi-Fi networks (the Wi-Fi page's Scan button, or opening the page). The driver sends the result back.
    ScanWifi,
    /// Join a network. `saved`: the Kindle has a profile for it, so only the connection is asked for; otherwise the
    /// profile is saved first (with the password, for a secured network). The password is never logged.
    JoinWifi { name: String, secured: bool, password: Option<String>, saved: bool },
    /// Forget a saved network (delete its profile).
    ForgetWifi(String),
    /// Set the root password the SSH server checks, from the SSH password screen. The driver gives it to the
    /// device on stdin only: it is not logged, kept, or put in a command line.
    SetSshPassword(String),
    /// Save the owner's device name (validated: 1 to 32 characters, no control characters).
    SetDeviceName(String),
    /// Check the SSH server (its process, port and firewall): the driver shows the result in the notice.
    CheckSsh,
}

/// The Kindle's USB state: nothing plugged in, the cable charging the Kindle, or the drive shown on a computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Usb {
    #[default]
    Unplugged,
    Charging,
    Drive,
}

impl Usb {
    /// From the charger's `online` (the cable is in) and whether the user storage (`/mnt/us`) is mounted on the Kindle:
    /// while the drive is shown on a computer, the Kindle has it unmounted.
    pub fn from_device(cable_in: bool, userstore_mounted: bool) -> Usb {
        match (cable_in, userstore_mounted) {
            (false, _) => Usb::Unplugged,
            (true, true) => Usb::Charging,
            (true, false) => Usb::Drive,
        }
    }
}

/// Physical display controls available on this Kindle. The desktop preview supports all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplaySupport {
    pub front_light: bool,
    pub warmth: bool,
    pub auto_rotation: bool,
}

impl Default for DisplaySupport {
    fn default() -> Self {
        Self { front_light: true, warmth: true, auto_rotation: true }
    }
}

/// What the status strip and Settings show. Filled by the driver.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub clock: String,
    pub date: String,
    pub clock_year: Option<i32>,
    pub utc_offset: Option<i32>,
    pub clock_sync_available: bool,
    pub battery: Option<u32>,
    pub charging: bool,
    pub power: battery::Reading,
    pub battery_history: Vec<battery::Sample>,
    pub battery_now: u64,
    pub low_power: bool,
    /// The USB state (see `Usb`).
    pub usb: Usb,
    pub wifi_on: bool,
    /// "connected", "not connected", "off", or empty when unknown.
    pub wifi_state: String,
    /// Signal strength while connected: (level, out of), from `signalStrength` ("4/5").
    pub wifi_signal: Option<(u32, u32)>,
    pub light: u32,
    pub warmth: u32,
    pub display_support: DisplaySupport,
    pub large_text: bool,
    pub high_contrast: bool,
    pub ssh: bool,
    /// The address to connect to while SSH runs, e.g. "ssh -p 2222 root@192.168.1.28"; empty when off or unknown.
    pub ssh_address: String,
    /// Whether the root password for SSH has been set from the launcher (or by the browser before it).
    pub ssh_password_set: bool,
    /// Public DNS (1.1.1.1 and 8.8.8.8) is in use.
    pub public_dns: bool,
    /// False while the launcher is off at boot (the DISABLE file exists).
    pub autostart: bool,
    /// The launcher's own sleep screen is on (the stock screensaver is unloaded).
    pub sleep_launcher: bool,
    /// The screen follows the device's turn.
    pub auto_rotation: bool,
    /// White on black.
    pub dark_mode: bool,
    /// The opening zoom is on.
    pub animation: bool,
    /// Seconds the screen stays on before the Kindle sleeps (0 = never).
    pub screen_timeout: u32,
    /// The battery usage summary lines: index 0 is the screen line, 1 to 4 the most used apps (name, value).
    pub usage: Vec<(String, String)>,
    pub rotation: Rotation,
    /// Free memory for the About text, e.g. "MemAvailable 312 MB".
    pub memory: String,
    /// The Wi-Fi address (About), or "not connected".
    pub ip: String,
    /// CPU, memory and storage, shown in quick settings (read by the driver).
    pub system: primitive_quick::Stats,
    /// Free storage on the Kindle's user partition (About), e.g. "2.1 GB free".
    pub storage: String,
}

impl Status {
    /// Whether what the Settings screens show from the status differs: Wi-Fi, SSH and its password and address,
    /// public DNS, and the start-up switch.
    pub fn settings_differs(&self, other: &Status) -> bool {
        self.wifi_on != other.wifi_on
            || self.wifi_state != other.wifi_state
            || self.wifi_signal != other.wifi_signal
            || self.display_differs(other)
            || self.ssh != other.ssh
            || self.ssh_address != other.ssh_address
            || self.ssh_password_set != other.ssh_password_set
            || self.public_dns != other.public_dns
            || self.autostart != other.autostart
    }

    pub fn display_differs(&self, other: &Self) -> bool {
        self.display_support != other.display_support
            || self.auto_rotation != other.auto_rotation
            || self.light != other.light
            || self.warmth != other.warmth
            || self.large_text != other.large_text
            || self.high_contrast != other.high_contrast
    }

    /// Whether what the status bar shows differs between two readings (the clock minute, battery, charging,
    /// Wi-Fi and its signal, SSH). The bar is redrawn only then.
    pub fn bar_differs(&self, other: &Status) -> bool {
        self.clock != other.clock
            || self.date != other.date
            || self.battery != other.battery
            || self.charging != other.charging
            || self.wifi_on != other.wifi_on
            || self.wifi_state != other.wifi_state
            || self.wifi_signal != other.wifi_signal
            || self.ssh != other.ssh
    }
}

/// The shortest and longest Wi-Fi password (WPA2-PSK).
pub const MIN_PASSWORD: usize = 8;
pub const MAX_PASSWORD: usize = 63;
/// The longest network name (SSID).
pub const MAX_NAME: usize = 32;
/// The SSH root password: 6 to 64 characters of letters, digits and - _ . @ # % + = ! (no spaces).
pub const MIN_SSH_PASSWORD: usize = 6;
pub const MAX_SSH_PASSWORD: usize = 64;
/// The longest device name the owner can type.
pub const MAX_DEVICE_NAME: usize = 32;

/// Whether `pw` is an acceptable SSH password. Only these characters can reach the device's shell or chpasswd, so
/// nothing in it can change what the launcher runs.
pub fn valid_ssh_password(pw: &str) -> bool {
    (MIN_SSH_PASSWORD..=MAX_SSH_PASSWORD).contains(&pw.chars().count())
        && pw.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@#%+=!".contains(c))
}

/// A horizontal swipe longer than this (pixels) turns the page.
const SWIPE_MIN: f32 = 150.0;
const SETTINGS_SWIPE_MIN: f32 = 40.0;

/// Scroll once per completed gesture, so the e-ink display only draws the final position.
fn scroll_settings_list(input: Input, viewport: Rect, offset: &mut i32, max: i32) -> Action {
    let button_step = (viewport.h - 120).max(120);
    let delta = match input {
        Input::PageForward => button_step,
        Input::PageBack => -button_step,
        Input::Swipe { dx, dy, start_y }
            if dy.abs() >= SETTINGS_SWIPE_MIN && dy.abs() > dx.abs()
                && (viewport.y as f32..viewport.bottom() as f32).contains(&start_y) =>
        {
            (-dy).round() as i32
        }
        _ => return Action::None,
    };
    let next = offset.saturating_add(delta).clamp(0, max);
    if next == *offset {
        return Action::None;
    }
    *offset = next;
    Action::Redraw { full: false }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRequirementReview {
    pub apps: Vec<crate::store::StoreEntry>,
    pub page: usize,
}

impl StoreRequirementReview {
    const APPS_PER_PAGE: usize = 3;

    pub fn pages(&self) -> usize {
        self.apps.len().div_ceil(Self::APPS_PER_PAGE).max(1)
    }

    pub fn visible_apps(&self) -> &[crate::store::StoreEntry] {
        let start = self.page.min(self.pages() - 1) * Self::APPS_PER_PAGE;
        &self.apps[start..(start + Self::APPS_PER_PAGE).min(self.apps.len())]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    pub version: String,
    pub can_remove: bool,
}

pub struct Ui {
    apps: Vec<App>,
    pub layout: Layout,
    pub display_support: DisplaySupport,
    pub page: usize,
    pub settings_scroll: i32,
    pub clock_syncing: bool,
    settings_parents: Vec<Page>,
    pub visibility_scroll: i32,
    installed_apps: Option<Vec<InstalledApp>>,
    installed_remove_confirm: Option<usize>,
    pub installed_scroll: i32,
    pub installed_busy: bool,
    pub wallpapers: WallpaperSettings,
    pub wallpaper_scroll: i32,
    pub wallpaper_preview: Option<WallpaperDraft>,
    pub firmware: Option<String>,
    pub device_profile: Option<String>,
    /// The model in use: its id, its name, and how it was known (`detected`, `selected` or `unknown`).
    pub device_model_id: Option<String>,
    pub device_model_name: String,
    pub device_source: String,
    /// The model has not been run with PrimitiveOS: the screens may not fit, and the launcher says so.
    pub device_untested: bool,
    pub screen: Screen,
    before_sleep: Screen,
    /// The pressed icon, drawn darkened until the app starts.
    pub pressed: Option<Rect>,
    /// A message in the footer (problems in apps.toml, a failed action). Cleared by the next tap.
    pub notice: Option<String>,
    /// Local network address and code for the active file transfer session.
    pub file_transfer: Option<(String, String)>,
    /// Compatible updates shown on the Store tile.
    pub store_update_count: usize,
    /// The app's version, shown on the About page (set by the app: the launcher's own version).
    pub version: String,
    /// The keyboard key under the finger, drawn in the on look until its action runs (see `press_feedback`).
    pub pressed_key: Option<Rect>,
    /// The USB state the screen follows: while the drive is shown, the drive-mode screen is drawn instead.
    pub usb: Usb,
    /// Apps hidden by the user (long press, or Settings > Hidden apps). Kept so they can be shown again.
    hidden_apps: Vec<App>,
    /// Low or critical battery, shown in the footer until the level recovers or the charger is connected.
    pub battery: Warning,
    battery_level: Option<u32>,
    /// The app the app menu edits (index into the visible apps).
    pub menu: usize,
    /// Icon files the app menu offers (PNGs in the launcher's icons folder), set by the driver.
    icon_choices: Vec<String>,
    /// Each app's own icon, for "Change icon" to come back to.
    original_icons: HashMap<String, IconSource>,
    /// KUAL extension menus (the Extensions tile), the open submenu path, and the list page.
    extensions: Vec<MenuItem>,
    ext_path: Vec<usize>,
    pub ext_page: usize,
    /// Store catalog, list position, and the app open on its product page.
    pub store: Vec<crate::store::StoreEntry>,
    pub store_list_scroll: i32,
    pub store_open: Option<usize>,
    /// The tab shown, and how far the product page is scrolled.
    pub store_tab: crate::store::StoreTab,
    pub store_scroll: i32,
    pub store_shot_index: usize,
    pub store_query: primitive_keyboard::Editor,
    pub store_search_keyboard: bool,
    /// The catalog entry on the confirmation or progress sheet.
    pub store_sheet: Option<usize>,
    pub store_requirements: Option<StoreRequirementReview>,
    pub store_progress: Option<String>,
    pub store_progress_percent: Option<u8>,
    pub store_settings_open: bool,
    pub store_daily_check: bool,
    pub store_clear_confirm: bool,
    pub store_batch_confirm: bool,
    pub store_batch_running: bool,
    pub store_category_picker: bool,
    pub store_category: Option<String>,
    pub store_collections: Vec<kl_store::catalog::Collection>,
    pub store_collection: Option<String>,
    /// A catalog refresh is running (the button shows Updating).
    pub store_busy: bool,
    /// The app being installed from the store, if any (its page shows Installing, and its action waits).
    pub store_installing: Option<String>,
    pub store_removing: bool,
    /// The app awaiting confirmation in the Remove dialog.
    pub store_remove_armed: Option<String>,
    /// Background jobs still running (shown in the status bar).
    pub running: usize,
    /// The Wi-Fi page: the networks from the last scan, whether a scan is running, and the list's scroll.
    pub wifi: crate::wifi::WifiInfo,
    pub wifi_scanning: bool,
    pub wifi_scroll: usize,
    /// The network the network screen or password screen is for.
    pub wifi_target: Option<crate::wifi::Network>,
    /// The password being typed (the shared editor: text, caret, keyboard layer), and whether it is shown.
    pub password: primitive_keyboard::Editor,
    pub show_password: bool,
    /// The name of a hidden network being typed, and whether the password screen is for a hidden network (an empty
    /// password then joins it as an open network).
    pub network_name: primitive_keyboard::Editor,
    pub wifi_hidden: bool,
    /// The SSH password being typed (kept here only until Set; never logged).
    pub ssh_password: primitive_keyboard::Editor,
    /// The device name being typed, and the saved name and the About page's values.
    pub device_name_entry: primitive_keyboard::Editor,
    pub device_name: Option<String>,
    pub series: Option<String>,
    pub mac: Option<String>,
    pub panel: Option<String>,
}

impl Ui {
    /// `apps` is the merged list; hidden apps are dropped here.
    pub fn new(apps: Vec<App>, layout: Layout) -> Ui {
        let original_icons = apps.iter().map(|a| (a.id.clone(), a.icon.clone())).collect();
        let (hidden_apps, apps): (Vec<App>, Vec<App>) = apps.into_iter().partition(|a| a.hidden);
        Ui {
            apps,
            hidden_apps,
            layout,
            display_support: DisplaySupport::default(),
            page: 0,
            settings_scroll: 0,
            clock_syncing: false,
            settings_parents: Vec::new(),
            visibility_scroll: 0,
            installed_apps: None,
            installed_remove_confirm: None,
            installed_scroll: 0,
            installed_busy: false,
            wallpapers: WallpaperSettings::default(),
            wallpaper_scroll: 0,
            wallpaper_preview: None,
            firmware: None,
            device_profile: None,
            device_model_id: None,
            device_model_name: String::new(),
            device_source: "unknown".into(),
            device_untested: false,
            screen: Screen::Home,
            before_sleep: Screen::Home,
            pressed: None,
            notice: None,
            file_transfer: None,
            store_update_count: 0,
            version: String::new(),
            pressed_key: None,
            usb: Usb::Unplugged,
            battery: Warning::None,
            battery_level: None,
            menu: 0,
            icon_choices: Vec::new(),
            original_icons,
            extensions: Vec::new(),
            ext_path: Vec::new(),
            ext_page: 0,
            store: Vec::new(),
            store_list_scroll: 0,
            store_open: None,
            store_tab: crate::store::StoreTab::Today,
            store_scroll: 0,
            store_shot_index: 0,
            store_query: primitive_keyboard::Editor::new(48),
            store_search_keyboard: false,
            store_sheet: None,
            store_requirements: None,
            store_progress: None,
            store_progress_percent: None,
            store_settings_open: false,
            store_daily_check: true,
            store_clear_confirm: false,
            store_batch_confirm: false,
            store_batch_running: false,
            store_category_picker: false,
            store_category: None,
            store_collections: Vec::new(),
            store_collection: None,
            store_busy: false,
            store_installing: None,
            store_removing: false,
            store_remove_armed: None,
            running: 0,
            wifi: crate::wifi::WifiInfo::default(),
            wifi_scanning: false,
            wifi_scroll: 0,
            wifi_target: None,
            password: primitive_keyboard::Editor::new(MAX_PASSWORD),
            show_password: false,
            network_name: primitive_keyboard::Editor::new(MAX_NAME),
            wifi_hidden: false,
            ssh_password: primitive_keyboard::Editor::new(MAX_SSH_PASSWORD),
            device_name_entry: primitive_keyboard::Editor::new(MAX_DEVICE_NAME),
            device_name: None,
            series: None,
            mac: None,
            panel: None,
        }
    }

    /// A background job ended (the driver got its last line).
    pub fn background_done(&mut self) {
        self.running = self.running.saturating_sub(1);
    }

    /// KUAL extension menus. When there are any, the Extensions tile joins the grid (with the built-ins).
    /// Replace the app list (the user storage came back: the apps are read again from /mnt/us).
    pub fn set_apps(&mut self, apps: Vec<App>) {
        self.original_icons = apps.iter().map(|app| (app.id.clone(), app.icon.clone())).collect();
        (self.hidden_apps, self.apps) = apps.into_iter().partition(|app| app.hidden);
        self.visibility_scroll = self.visibility_scroll.min(self.visibility_scroll_max());
        self.page = 0;
        self.pressed = None;
    }

    pub fn set_extensions(&mut self, menu: Vec<MenuItem>) {
        self.extensions = menu;
        self.ext_path.clear();
        self.ext_page = 0;
        let present = self.apps.iter().any(|a| a.launch == Launch::Extensions);
        if !self.extensions.is_empty() && !present {
            self.apps.push(extensions_app());
            self.apps.sort_by_cached_key(sort_key);
        } else if self.extensions.is_empty() && present {
            self.apps.retain(|a| a.launch != Launch::Extensions);
        }
    }

    /// The items at the open level of the Extensions list.
    pub fn ext_items(&self) -> &[MenuItem] {
        let mut items = self.extensions.as_slice();
        for &i in &self.ext_path {
            match items.get(i) {
                Some(item) => items = &item.children,
                None => return &[],
            }
        }
        items
    }

    /// The Extensions list's title: "Extensions", or the open submenu's name.
    pub fn ext_title(&self) -> String {
        let mut items = self.extensions.as_slice();
        let mut title = "Extensions".to_string();
        for &i in &self.ext_path {
            if let Some(item) = items.get(i) {
                title = item.name.clone();
                items = &item.children;
            }
        }
        title
    }

    /// How deep the open submenu is (0: the top of the list).
    pub fn ext_depth(&self) -> usize {
        self.ext_path.len()
    }

    pub fn ext_pages(&self) -> usize {
        self.layout.list_pages(self.ext_items().len())
    }

    /// The Wi-Fi page's networks, from the last scan. Set by the driver.
    pub fn set_wifi(&mut self, info: crate::wifi::WifiInfo) {
        let current = info.current.as_ref().map(|c| c.name.clone());
        let networks = info.networks.into_iter().filter(|n| Some(&n.name) != current.as_ref()).collect();
        self.wifi = crate::wifi::WifiInfo { current: info.current, networks };
        // The network screen shows the network as it is now (saved or not, connected or not).
        if let Some(name) = self.wifi_target.as_ref().map(|t| t.name.clone()) {
            let fresh = self.wifi.current.iter().chain(self.wifi.networks.iter()).find(|n| n.name == name).cloned();
            if let Some(fresh) = fresh {
                self.wifi_target = Some(fresh);
            }
        }
        self.wifi_scanning = false;
        self.wifi_scroll = self.wifi_scroll.min(self.wifi_max_scroll());
    }

    /// How far the scan list can scroll: its networks, less the rows that fit.
    fn wifi_max_scroll(&self) -> usize {
        self.wifi.networks.len().saturating_sub(self.layout.wifi_area().visible)
    }

    /// Background mode: say it started; the driver runs it and shows its last line when it ends.
    fn run_background(&mut self, app: App) -> Action {
        self.notice = Some(format!("Running {}…", app.name));
        self.running += 1;
        Action::RunBackground(app)
    }

    /// Replaces the store's entries (from the catalog cache and the installed record).
    pub fn set_store(&mut self, entries: Vec<crate::store::StoreEntry>) {
        self.store = entries;
        self.store_list_scroll = 0;
        self.store_open = None;
        self.store_scroll = 0;
        self.store_shot_index = 0;
        self.store_sheet = None;
        self.store_requirements = None;
        self.store_remove_armed = None;
    }

    /// Refresh entry data without replacing the open product with whichever app now occupies its old index.
    pub fn update_store(&mut self, entries: Vec<crate::store::StoreEntry>) {
        let open = self.store_open.and_then(|i| self.store.get(i)).map(|entry| entry.id.as_str());
        self.store_open = open.and_then(|id| entries.iter().position(|entry| entry.id == id));
        if self.store_open.is_none() {
            if self.screen == Screen::StoreApp {
                self.screen = Screen::Store;
            }
            if self.before_sleep == Screen::StoreApp {
                self.before_sleep = Screen::Store;
            }
            self.store_sheet = None;
            self.store_requirements = None;
            self.store_remove_armed = None;
        } else if self.store_sheet.is_some() {
            self.store_sheet = self.store_open;
        }
        self.store = entries;
        if let Some(entry) = self.store_open.and_then(|index| self.store.get(index)) {
            self.store_shot_index = self.store_shot_index.min(entry.screenshots.saturating_sub(1));
        }
        self.store_scroll = self.store_scroll.min(self.store_scroll_max());
        self.clamp_store_list_scroll();
    }

    /// Today's featured app: the first entry.
    pub fn store_featured(&self) -> Option<usize> {
        (!self.store.is_empty()).then_some(0)
    }

    /// Today's rows under the featured app: the next two entries.
    pub fn store_today_rows(&self) -> Vec<usize> {
        (1..self.store.len().min(3)).collect()
    }

    pub fn store_today_collections(&self) -> Vec<&kl_store::catalog::Collection> {
        self.store_collections
            .iter()
            .filter(|collection| collection.apps.iter().any(|id| self.store.iter().any(|app| &app.id == id)))
            .collect()
    }

    pub fn store_categories(&self) -> Vec<&str> {
        self.store.iter()
            .flat_map(|entry| entry.categories.iter().map(String::as_str))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .take(9)
            .collect()
    }

    pub fn store_selected_collection(&self) -> Option<&kl_store::catalog::Collection> {
        let id = self.store_collection.as_deref()?;
        self.store_collections.iter().find(|collection| collection.id == id)
    }

    /// The Apps tab, with an optional category or collection selected on the device.
    pub fn store_apps(&self) -> Vec<usize> {
        let collection = self.store_selected_collection();
        self.store.iter().enumerate()
            .filter(|(_, app)| {
                self.store_category.as_ref().is_none_or(|category| app.categories.contains(category))
                    && collection.is_none_or(|collection| collection.apps.contains(&app.id))
            })
            .map(|(i, _)| i).collect()
    }

    /// Search by app title, summary, developer or category. An empty query shows the whole catalog.
    pub fn store_search(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return (0..self.store.len()).collect();
        }
        self.store
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                [entry.name.as_str(), &entry.subtitle, &entry.developer]
                    .into_iter()
                    .chain(entry.categories.iter().map(String::as_str))
                    .any(|field| field.to_lowercase().contains(&query))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The Updates tab: the entries installed from the store, with or without an update ready.
    pub fn store_updates(&self) -> Vec<usize> {
        self.store
            .iter()
            .enumerate()
            .filter(|(_, e)| !matches!(e.status, crate::store::Status::NotInstalled))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn store_pending_updates(&self) -> impl Iterator<Item = &crate::store::StoreEntry> {
        self.store.iter().filter(|app| {
            app.id != crate::store::LAUNCHER_ID && matches!(app.status, crate::store::Status::Update { .. })
        })
    }

    pub fn store_has_toolbar(&self) -> bool {
        use crate::store::StoreTab;
        match self.store_tab {
            StoreTab::Apps => self.store_selected_collection().is_some() || self.store.iter().any(|entry| !entry.categories.is_empty()),
            StoreTab::Updates => self.store_pending_updates().next().is_some(),
            _ => false,
        }
    }

    fn store_operation_running(&self) -> bool {
        self.store_installing.is_some() || self.store_batch_running
    }

    /// The entries in the current tab, in display order.
    pub fn store_list_indices(&self) -> Vec<usize> {
        match self.store_tab {
            crate::store::StoreTab::Updates => self.store_updates(),
            crate::store::StoreTab::Search => self.store_search(&self.store_query.string()),
            _ => self.store_apps(),
        }
    }

    fn store_unscrolled_row(&self, i: usize) -> Rect {
        if self.store_tab == StoreTab::Search {
            self.layout.store_search_row(i)
        } else if self.store_has_toolbar() {
            self.layout.store_toolbar_row(i)
        } else {
            self.layout.store_list_row(i)
        }
    }

    fn store_scroll_rect(&self, rect: Rect) -> Rect {
        Rect { y: rect.y - self.store_list_scroll, ..rect }
    }

    pub fn store_list_row(&self, i: usize) -> Rect {
        self.store_scroll_rect(self.store_unscrolled_row(i))
    }

    pub fn store_today_feature(&self) -> Rect {
        self.store_scroll_rect(self.layout.store_feature())
    }

    pub fn store_today_see_all(&self) -> Rect {
        self.store_scroll_rect(self.layout.store_see_all())
    }

    pub fn store_today_row(&self, i: usize) -> Rect {
        self.store_scroll_rect(self.layout.store_today_row(i))
    }

    pub fn store_today_collection_card(&self, i: usize) -> Rect {
        self.store_scroll_rect(self.layout.store_collection_card(i))
    }

    pub fn store_list_notice(&self) -> Option<&str> {
        self.notice.as_deref().or(self.store_progress.as_deref().filter(|_| self.store_busy))
    }

    /// The portion of the screen that scrolls. The title, toolbar, tabs and keyboard stay outside it.
    pub fn store_list_viewport(&self) -> Rect {
        let top = if self.store_tab == StoreTab::Search {
            self.layout.store_search_field().bottom() + 8
        } else if self.store_has_toolbar() {
            self.layout.store_toolbar_button().bottom() + 8
        } else {
            self.layout.store_title().bottom() + 20
        };
        let mut bottom = if self.store_search_keyboard {
            self.layout.keyboard_area().y - 12
        } else {
            self.layout.store_body_bottom()
        };
        if self.store_list_notice().is_some() {
            bottom -= 68;
        }
        Rect::new(0, top, self.layout.width, (bottom - top).max(0))
    }

    pub fn store_list_scroll_max(&self) -> i32 {
        let bottom = if self.store_tab == StoreTab::Today {
            let collections = self.store_today_collections().len();
            if collections != 0 {
                self.layout.store_collection_card(collections - 1).bottom()
            } else if let Some(last) = self.store_today_rows().len().checked_sub(1) {
                self.layout.store_today_row(last).bottom()
            } else {
                self.store_featured().map_or(0, |_| self.layout.store_see_all().bottom())
            }
        } else {
            self.store_list_indices().len().checked_sub(1).map_or(0, |last| self.store_unscrolled_row(last).bottom())
        };
        (bottom - self.store_list_viewport().bottom()).max(0)
    }

    pub fn clamp_store_list_scroll(&mut self) {
        if self.store_list_scroll > 0 {
            self.store_list_scroll = self.store_list_scroll.min(self.store_list_scroll_max());
        }
    }

    fn store_entries_in(&self, viewport: Rect) -> Vec<(Rect, usize)> {
        let intersects = |r: Rect| r.y < viewport.bottom() && r.bottom() > viewport.y;
        if viewport.h <= 0 {
            return Vec::new();
        }
        if self.store_tab == StoreTab::Today {
            let first = self.store_featured().map(|i| (self.store_today_feature(), i));
            let rows = if self.store_today_collections().is_empty() { self.store_today_rows() } else { Vec::new() };
            return first.into_iter().chain(rows.into_iter().enumerate().map(|(i, index)| {
                (self.store_today_row(i), index)
            })).filter(|(rect, _)| intersects(*rect)).collect();
        }
        let indices = self.store_list_indices();
        let row = self.store_list_row(0);
        let start = (viewport.y - row.y).div_euclid(row.h).max(0) as usize;
        let end = ((viewport.bottom() - 1 - row.y).div_euclid(row.h) + 1).max(0) as usize;
        (start..end.min(indices.len()))
            .map(|i| (Rect { y: row.y + i as i32 * row.h, ..row }, indices[i]))
            .collect()
    }

    pub fn store_visible_entries(&self) -> Vec<(Rect, usize)> {
        self.store_entries_in(self.store_list_viewport())
    }

    /// Preload at most one viewport in each direction, in order of distance from the visible rows.
    pub fn store_nearby_entries(&self) -> Vec<usize> {
        let viewport = self.store_list_viewport();
        let range = Rect { y: viewport.y - viewport.h, h: viewport.h * 3, ..viewport };
        let mut nearby = self.store_entries_in(range);
        nearby.retain(|(r, _)| r.bottom() <= viewport.y || r.y >= viewport.bottom());
        nearby.sort_by_key(|(r, _)| {
            if r.y >= viewport.bottom() { (r.y - viewport.bottom(), false) }
            else { (viewport.y - r.bottom(), true) }
        });
        let mut indices: Vec<_> = nearby.into_iter().map(|(_, i)| i).collect();
        if self.store_tab == StoreTab::Today {
            let row = self.layout.store_toolbar_row(0);
            let apps = ((self.layout.store_body_bottom() - row.y) / row.h).max(0) as usize;
            let visible = self.store_visible_entries();
            for i in 0..self.store.len().min(apps) {
                if !indices.contains(&i) && !visible.iter().any(|(_, index)| *index == i) {
                    indices.push(i);
                }
            }
        }
        indices
    }

    /// The sections of a product page that determine its scroll limit.
    fn store_info_count(&self, index: usize) -> (usize, usize, usize) {
        self.store.get(index).map_or((0, 0, 0), |e| (e.page_rows().len(), e.screenshots, e.versions.len()))
    }

    pub fn open_store(&mut self) -> Action {
        self.set_store_tab(StoreTab::Today);
        self.go(Screen::Store)
    }

    fn scroll_store_list(&mut self, distance: i32, max: i32) -> Action {
        let next = self.store_list_scroll.saturating_add(distance).clamp(0, max);
        if next == self.store_list_scroll {
            return Action::None;
        }
        self.store_list_scroll = next;
        Action::Redraw { full: false }
    }

    /// Move through long lists, keeping some overlap, or switch tabs when everything fits.
    fn store_page_key(&mut self, step: i32, viewport: Rect) -> Action {
        let max = self.store_list_scroll_max();
        if max > 0 {
            let row = self.layout.store_list_row(0).h;
            let distance = (viewport.h / row - 1).max(1) * row;
            return self.scroll_store_list(step * distance, max);
        }
        if self.store_search_keyboard {
            return Action::None;
        }
        let tabs = StoreTab::ALL;
        let current = tabs.iter().position(|t| *t == self.store_tab).unwrap_or(0) as i32;
        let next = current + step;
        if next < 0 || next >= tabs.len() as i32 {
            return Action::None;
        }
        self.set_store_tab(tabs[next as usize])
    }

    fn store_open_entry(&mut self, index: usize) -> Action {
        if self.store_search_keyboard {
            self.store_search_keyboard = false;
            self.clamp_store_list_scroll();
        }
        self.store_open = Some(index);
        self.store_requirements = None;
        self.store_scroll = 0;
        self.store_shot_index = 0;
        self.store_remove_armed = None;
        self.go(Screen::StoreApp)
    }

    fn store_tap_entry(&mut self, index: usize, row: Rect, x: f32, y: f32) -> Action {
        if Layout::store_row_action(row).inset(-12).contains(x, y) {
            let entry = &self.store[index];
            if entry.can_open() {
                return self.store_app_action(index);
            }
            if entry.can_install() {
                self.store_open_entry(index);
                return self.store_app_action(index);
            }
        }
        self.store_open_entry(index)
    }

    fn store_app_action(&mut self, index: usize) -> Action {
        use crate::store::{Kind, Status};
        let entry = &self.store[index];
        match (entry.kind, &entry.status) {
            (Kind::Link, _) => {
                self.notice = Some(format!("{} is on its own website: {}", entry.name, entry.repo));
                Action::Redraw { full: false }
            }
            _ if self.store_operation_running() => Action::None,
            (Kind::Install, _) if entry.can_open() => Action::Device(DeviceCmd::StoreOpen(index)),
            (Kind::Install, Status::Installed(_)) => Action::None,
            (Kind::Install, _) if !entry.can_install() => {
                self.notice = Some(entry.status_line());
                Action::Redraw { full: false }
            }
            (Kind::Install, _) => {
                self.store_sheet = Some(index);
                Action::Redraw { full: false }
            }
        }
    }

    fn set_store_tab(&mut self, tab: crate::store::StoreTab) -> Action {
        self.store_tab = tab;
        self.store_list_scroll = 0;
        self.store_search_keyboard = false;
        self.store_collection = None;
        self.store_category = None;
        Action::Redraw { full: false }
    }

    fn confirm_clear_store_cache(&mut self, input: Input) -> Action {
        let [(_, cancel), (_, clear)] = self.layout.confirm_buttons();
        match input {
            Input::Tap(x, y) if clear.contains(x, y) => {
                self.store_clear_confirm = false;
                if self.store_busy {
                    self.notice = Some("Wait for the store to finish updating.".to_string());
                    Action::Redraw { full: true }
                } else {
                    Action::Device(DeviceCmd::StoreClearCache)
                }
            }
            Input::PageBack => {
                self.store_clear_confirm = false;
                Action::Redraw { full: true }
            }
            Input::Tap(x, y) if cancel.contains(x, y) || !self.layout.store_settings_box().contains(x, y) => {
                self.store_clear_confirm = false;
                Action::Redraw { full: true }
            }
            _ => Action::None,
        }
    }

    fn store_list_popup(&mut self, input: Input) -> Option<Action> {
        let l = self.layout;
        let tap = match input {
            Input::Tap(x, y) => Some((x, y)),
            _ => None,
        };
        if self.store_settings_open {
            if self.store_clear_confirm {
                return Some(self.confirm_clear_store_cache(input));
            }
            let [_, (_, done)] = l.confirm_buttons();
            if let Some((x, y)) = tap {
                if l.store_daily_check_button().contains(x, y) {
                    return Some(Action::Device(DeviceCmd::StoreToggleDailyCheck));
                } else if l.store_clear_button().contains(x, y) {
                    if self.store_busy {
                        self.notice = Some("Wait for the store to finish updating.".to_string());
                    } else {
                        self.store_clear_confirm = true;
                    }
                    return Some(Action::Redraw { full: false });
                } else if done.contains(x, y) || !l.store_settings_box().contains(x, y) {
                    self.store_settings_open = false;
                    return Some(Action::Redraw { full: true });
                }
            }
            if input == Input::PageBack {
                self.store_settings_open = false;
                return Some(Action::Redraw { full: true });
            }
            return Some(Action::None);
        }
        if self.store_batch_confirm {
            let [cancel, update] = l.store_sheet_buttons();
            if self.store_batch_running {
                return Some(Action::None);
            }
            if tap.is_some_and(|(x, y)| update.contains(x, y)) {
                if self.store_busy {
                    self.notice = Some("Wait for the store to finish updating.".to_string());
                    return Some(Action::Redraw { full: false });
                }
                self.store_batch_running = true;
                return Some(Action::Device(DeviceCmd::StoreUpdateAll));
            }
            if input == Input::PageBack || tap.is_some_and(|(x, y)| cancel.contains(x, y) || !l.store_sheet().contains(x, y)) {
                self.store_batch_confirm = false;
                return Some(Action::Redraw { full: true });
            }
            return Some(Action::None);
        }
        if self.store_category_picker {
            let [_, done] = l.store_sheet_buttons();
            if let Some((x, y)) = tap {
                let categories = self.store_categories();
                if let Some(i) = (0..=categories.len()).find(|&i| l.store_category_cell(i).contains(x, y)) {
                    self.store_category = i.checked_sub(1).map(|i| categories[i].to_string());
                    self.store_list_scroll = 0;
                    self.store_category_picker = false;
                    return Some(Action::Redraw { full: true });
                }
            }
            if input == Input::PageBack || tap.is_some_and(|(x, y)| done.contains(x, y) || !l.store_sheet().contains(x, y)) {
                self.store_category_picker = false;
                return Some(Action::Redraw { full: true });
            }
            return Some(Action::None);
        }
        None
    }

    fn store_keyboard(&mut self, input: Input) -> Option<Action> {
        let l = self.layout;
        let mut open = self.store_search_keyboard;
        let action = match input {
            Input::PageBack if open => {
                open = false;
                Action::Redraw { full: true }
            }
            Input::Tap(x, y) if l.store_search_field().contains(x, y) && !open => {
                open = true;
                Action::Redraw { full: false }
            }
            Input::Tap(x, y) if open && l.keyboard_area().contains(x, y) => {
                let key = l.keyboard_key_at(self.store_query.layer, x, y)?;
                match self.store_query.press(key) {
                    primitive_keyboard::Press::Go => {
                        open = false;
                        Action::Redraw { full: true }
                    }
                    primitive_keyboard::Press::Edited => {
                        self.store_list_scroll = 0;
                        Action::Redraw { full: false }
                    }
                    _ => Action::Redraw { full: false },
                }
            }
            _ => return None,
        };
        if open != self.store_search_keyboard {
            self.store_search_keyboard = open;
            self.clamp_store_list_scroll();
        }
        Some(action)
    }

    /// A swipe moves the list within its viewport. The tab bar, title controls and keyboard keep their own targets.
    fn store_list(&mut self, input: Input, viewport: Rect) -> Action {
        let l = self.layout;
        if let Some(action) = self.store_list_popup(input) {
            return action;
        }
        if self.store_tab == StoreTab::Search {
            if let Some(action) = self.store_keyboard(input) {
                return action;
            }
        }
        match input {
            Input::PageForward => self.store_page_key(1, viewport),
            Input::PageBack => self.store_page_key(-1, viewport),
            Input::Swipe { dx, dy, start_y } if dy.abs() >= 40.0 && dy.abs() > dx.abs()
                && (viewport.y as f32..viewport.bottom() as f32).contains(&start_y) =>
            {
                self.scroll_store_list((-dy).round() as i32, self.store_list_scroll_max())
            }
            Input::Tap(x, y) => {
                if l.store_done().inset(-16).contains(x, y) {
                    if self.store_operation_running() {
                        return Action::None;
                    }
                    return self.go(Screen::Home);
                }
                if l.store_refresh().inset(-16).contains(x, y) {
                    if self.store_busy || self.store_operation_running() {
                        return Action::None;
                    }
                    self.store_busy = true;
                    return Action::Device(DeviceCmd::StoreRefresh);
                }
                if l.store_settings_button().contains(x, y) {
                    self.store_search_keyboard = false;
                    self.clamp_store_list_scroll();
                    self.store_settings_open = true;
                    return Action::Redraw { full: false };
                }
                if let Some(i) = l.store_tab_at(x, y).filter(|_| !self.store_search_keyboard) {
                    let tab = StoreTab::ALL[i];
                    if tab == self.store_tab {
                        return Action::None;
                    }
                    return self.set_store_tab(tab);
                }
                if self.store_has_toolbar() && l.store_toolbar_button().contains(x, y) {
                    match self.store_tab {
                        StoreTab::Apps if self.store_collection.is_some() => {
                            self.store_collection = None;
                            self.store_list_scroll = 0;
                        }
                        StoreTab::Apps => self.store_category_picker = true,
                        StoreTab::Updates => self.store_batch_confirm = true,
                        _ => {}
                    }
                    return Action::Redraw { full: false };
                }
                if !viewport.contains(x, y) {
                    return Action::None;
                }
                if self.store_tab == StoreTab::Today {
                    if let Some(index) = self.store_featured() {
                        if self.store_today_feature().contains(x, y) {
                            return self.store_open_entry(index);
                        }
                        if self.store_today_see_all().inset(-20).contains(x, y) {
                            return self.set_store_tab(StoreTab::Apps);
                        }
                    }
                    if let Some(id) = self.store_today_collections().into_iter().enumerate()
                        .find(|(i, _)| self.store_today_collection_card(*i).contains(x, y))
                        .map(|(_, collection)| collection.id.clone())
                    {
                        let action = self.set_store_tab(StoreTab::Apps);
                        self.store_collection = Some(id);
                        return action;
                    }
                }
                let Some((row, index)) = self.store_entries_in(viewport).into_iter()
                    .find(|(row, _)| row.contains(x, y)) else { return Action::None };
                self.store_tap_entry(index, row, x, y)
            }
            _ => Action::None,
        }
    }

    /// A product page with install and remove confirmations.
    fn store_app(&mut self, input: Input) -> Action {
        let l = self.layout;
        let Some(index) = self.store_open else { return self.go(Screen::Store) };
        let Some(entry) = self.store.get(index).cloned() else { return self.go(Screen::Store) };
        if self.store_sheet.is_some() {
            if self.store_installing.is_some() {
                return Action::None;
            }
            let box_rect = l.store_sheet();
            let [cancel, install] = l.store_sheet_buttons();
            if let Some(review) = &mut self.store_requirements {
                if let Input::Tap(x, y) = input {
                    let [prev, next] = l.store_requirements_nav();
                    if prev.contains(x, y) || next.contains(x, y) {
                        if prev.contains(x, y) {
                            review.page = review.page.saturating_sub(1);
                        } else {
                            review.page = (review.page + 1).min(review.pages() - 1);
                        }
                        return Action::Redraw { full: false };
                    }
                }
            }
            match input {
                Input::Tap(x, y) if install.contains(x, y) => {
                    if self.store_busy {
                        self.notice = Some("Wait for the store to finish updating.".to_string());
                        return Action::Redraw { full: false };
                    }
                    self.store_installing = Some(entry.id.clone());
                    return Action::Device(DeviceCmd::StoreInstall(index));
                }
                Input::PageBack => self.store_sheet = None,
                Input::Tap(x, y) if cancel.contains(x, y) || !box_rect.contains(x, y) => self.store_sheet = None,
                _ => return Action::None,
            }
            self.store_requirements = None;
            return Action::Redraw { full: true };
        }
        if self.store_remove_armed.is_some() {
            let [(_, cancel), (_, remove)] = l.confirm_buttons();
            match input {
                Input::Tap(x, y) if remove.contains(x, y) => {
                    self.store_remove_armed = None;
                    return Action::Device(DeviceCmd::StoreRemove(index));
                }
                Input::PageBack => self.store_remove_armed = None,
                Input::Tap(x, y) if cancel.contains(x, y) || !l.confirm_box().contains(x, y) => self.store_remove_armed = None,
                _ => return Action::None,
            }
            return Action::Redraw { full: true };
        }
        let max = self.store_scroll_max();
        match input {
            Input::Swipe { dx, dy, .. } if dy.abs() > SWIPE_MIN && dy.abs() > dx.abs() * 1.5 => {
                let next = (self.store_scroll - dy as i32).clamp(0, max);
                if next == self.store_scroll {
                    return Action::None;
                }
                self.store_scroll = next;
                Action::Redraw { full: false }
            }
            Input::PageForward | Input::PageBack => {
                // A page button scrolls the product page by a screen; Back at the top returns to the store.
                let step = if input == Input::PageForward { 1 } else { -1 };
                let next = (self.store_scroll + step * (l.height - 200)).clamp(0, max);
                if next == self.store_scroll {
                    return if step < 0 { self.go(Screen::Store) } else { Action::None };
                }
                self.store_scroll = next;
                Action::Redraw { full: false }
            }
            Input::Tap(x, y) => {
                if l.store_back().inset(-20).contains(x, y) {
                    return self.go(Screen::Store);
                }
                let content_visible = (170.0..(l.height - 120) as f32).contains(&y);
                if content_visible && entry.screenshots > 1 {
                    if self.store_shot_index > 0 && l.store_shot_button(self.store_scroll, false).contains(x, y) {
                        self.store_shot_index -= 1;
                        return Action::Redraw { full: false };
                    }
                    if self.store_shot_index + 1 < entry.screenshots && l.store_shot_button(self.store_scroll, true).contains(x, y) {
                        self.store_shot_index += 1;
                        return Action::Redraw { full: false };
                    }
                }
                if content_visible && l.store_action(self.store_scroll).inset(-12).contains(x, y) {
                    return self.store_app_action(index);
                }
                // The last row of an installed app opens its Remove confirmation.
                let rows = entry.page_rows();
                if entry.can_remove() {
                    let last = rows.len() - 1;
                    let row = l.store_info_row(last, self.store_scroll, entry.screenshots, entry.versions.len());
                    if content_visible && row.contains(x, y) {
                        self.store_remove_armed = Some(entry.id.clone());
                        return Action::Redraw { full: false };
                    }
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    /// The largest scroll of the open product page (zero when it fits the screen).
    pub fn store_scroll_max(&self) -> i32 {
        self.store_open.map_or(0, |i| {
            let (rows, shots, versions) = self.store_info_count(i);
            (self.layout.store_page_height(rows, shots, versions) - self.layout.height).max(0)
        })
    }

    pub fn open_extensions(&mut self) -> Action {
        self.ext_path.clear();
        self.ext_page = 0;
        self.go(Screen::Extensions)
    }

    /// The Extensions list: open a submenu, run an item, page, or go back up.
    fn extensions_list(&mut self, input: Input) -> Action {
        let l = self.layout;
        let turn = |u: &mut Ui, step: i32| -> Action {
            let next = u.ext_page as i32 + step;
            if next < 0 || next >= u.ext_pages() as i32 {
                return Action::None;
            }
            u.ext_page = next as usize;
            Action::Redraw { full: false }
        };
        match input {
            Input::PageForward => turn(self, 1),
            Input::PageBack => turn(self, -1),
            Input::Swipe { dx, dy, .. } if dx.abs() > SWIPE_MIN && dx.abs() > dy.abs() * 1.5 => turn(self, if dx < 0.0 { 1 } else { -1 }),
            Input::Tap(x, y) => {
                if l.done_button().inset(-20).contains(x, y) {
                    if self.ext_path.pop().is_some() {
                        self.ext_page = 0;
                        return Action::Redraw { full: false };
                    }
                    return self.go(Screen::Home);
                }
                if l.footer().contains(x, y) {
                    return turn(self, if x < (l.width / 2) as f32 { -1 } else { 1 });
                }
                let Some(row) = l.list_row_at(x, y) else { return Action::None };
                let index = self.ext_page * l.list_rows_per_page() + row;
                let Some(item) = self.ext_items().get(index).cloned() else { return Action::None };
                if item.is_submenu() {
                    self.ext_path.push(index);
                    self.ext_page = 0;
                    return Action::Redraw { full: false };
                }
                match item.app() {
                    Some(app) if app.background => self.run_background(app),
                    Some(app) => Action::Launch(app),
                    None => Action::None,
                }
            }
            _ => Action::None,
        }
    }

    /// The icon files "Change icon" cycles through. A dark form (`<name>-dark.png` beside `<name>.png`) is not a
    /// choice of its own: it comes with its light form.
    pub fn set_icon_choices(&mut self, choices: Vec<String>) {
        let dark_forms: std::collections::HashSet<String> =
            choices.iter().filter_map(|path| crate::apps::dark_form_path(path)).collect();
        self.icon_choices = choices.into_iter().filter(|path| !dark_forms.contains(path)).collect();
    }

    /// The Wi-Fi scan list on the Wi-Fi page (networks other than the current one).
    pub fn wifi_networks(&self) -> &[crate::wifi::Network] {
        &self.wifi.networks
    }

    /// The app the app menu edits.
    pub fn menu_app(&self) -> Option<&App> {
        self.apps.get(self.menu)
    }

    /// The icon of the app at `index`, when it is on the current page.
    pub fn icon_rect(&self, index: usize) -> Option<Rect> {
        let l = self.layout;
        (index < self.apps.len() && index / l.per_page() == self.page).then(|| l.icon(index % l.per_page()))
    }

    /// The app menu's actions for the app it edits.
    pub fn menu_actions(&self) -> Vec<Button> {
        match self.menu_app() {
            Some(app) if !is_builtin(app) => vec![Button::MenuEarlier, Button::MenuLater, Button::MenuIcon, Button::MenuHide],
            _ => Vec::new(),
        }
    }

    /// Where the app menu opens: beside the edited app's icon.
    pub fn menu_anchor(&self) -> Rect {
        self.icon_rect(self.menu).unwrap_or_else(|| {
            let s = self.layout.icon_size();
            Rect::new((self.layout.width - s) / 2, self.layout.grid().y, s, s)
        })
    }

    /// Record a battery reading. Returns true when the warning changed (the screen needs a redraw). A critical
    /// battery on the home screen goes to the sleep screen, which shows the warning; the power button wakes it.
    pub fn set_battery(&mut self, level: Option<u32>, charging: bool) -> bool {
        let next = battery::assess(level, charging, self.battery);
        self.battery_level = level;
        let changed = next != self.battery;
        self.battery = next;
        if changed && next == Warning::Critical && self.screen == Screen::Home {
            self.before_sleep = Screen::Home;
            self.screen = Screen::Sleep;
            self.pressed = None;
        }
        changed
    }

    /// The battery text for the footer, if a warning is active.
    pub fn battery_message(&self) -> Option<String> {
        battery::message(self.battery, self.battery_level)
    }

    pub fn apps(&self) -> &[App] {
        &self.apps
    }

    pub fn hidden_count(&self) -> usize {
        self.hidden_apps.len()
    }

    fn visibility_apps(&self) -> Vec<&App> {
        let mut apps: Vec<_> = self.apps.iter().chain(&self.hidden_apps).filter(|app| !is_builtin(app)).collect();
        apps.sort_by_cached_key(|app| (app.name.to_lowercase(), app.id.clone()));
        apps
    }

    pub fn visibility_len(&self) -> usize {
        self.apps.iter().chain(&self.hidden_apps).filter(|app| !is_builtin(app)).count()
    }

    pub fn visibility_scroll_max(&self) -> i32 {
        self.layout.visibility_scroll_max(self.visibility_len())
    }

    pub fn set_installed_apps(&mut self, apps: Option<Vec<InstalledApp>>) {
        self.installed_apps = apps;
        self.installed_remove_confirm = None;
        self.installed_scroll = self.installed_scroll.clamp(0, self.installed_scroll_max());
    }

    pub fn installed_apps(&self) -> Option<&[InstalledApp]> {
        self.installed_apps.as_deref()
    }

    pub fn installed_app(&self, index: usize) -> Option<&InstalledApp> {
        self.installed_apps()?.get(index)
    }

    pub fn installed_confirmation(&self) -> Option<&InstalledApp> {
        self.installed_app(self.installed_remove_confirm?)
    }

    pub fn installed_scroll_max(&self) -> i32 {
        self.layout.visibility_scroll_max(self.installed_apps().map_or(0, |apps| apps.len()))
    }

    pub fn wallpaper_scroll_max(&self, target: WallpaperTarget) -> i32 {
        self.layout.visibility_scroll_max(self.wallpapers.choices(target).count())
    }

    pub fn set_wallpapers(&mut self, wallpapers: WallpaperSettings) {
        self.wallpapers = wallpapers;
        let screen = if self.screen == Screen::Sleep { self.before_sleep } else { self.screen };
        let target = match screen {
            Screen::WallpaperPicker(target) => Some(target),
            Screen::WallpaperPreview => {
                self.wallpaper_preview.as_mut().map(|draft| {
                    if let WallpaperChoice::File(name) = &draft.choice {
                        if !self.wallpapers.images.contains(name) {
                            draft.error = Some("This picture is no longer available.".into());
                        }
                    }
                    draft.target
                })
            }
            _ => None,
        };
        if let Some(target) = target {
            self.wallpaper_scroll = self.wallpaper_scroll.min(self.wallpaper_scroll_max(target));
        }
    }

    /// Return from a cancelled or successfully applied preview, keeping the picker position.
    pub fn finish_wallpaper_preview(&mut self) -> Action {
        let Some(draft) = self.wallpaper_preview.take() else { return Action::None };
        self.go(Screen::WallpaperPicker(draft.target))
    }

    pub fn visible_visibility_apps(&self) -> impl Iterator<Item = (usize, &App)> {
        let viewport = self.layout.visibility_viewport();
        self.visibility_apps().into_iter().enumerate().filter(move |(index, _)| {
            let row = self.layout.visibility_row(*index, self.visibility_scroll);
            row.bottom() > viewport.y && row.y < viewport.bottom()
        })
    }

    /// Reconcile an optimistic visibility change if its write fails.
    pub fn restore_visibility(&mut self, changes: &[(String, bool)]) {
        for (id, hidden) in changes {
            let (from, to) = if *hidden { (&mut self.hidden_apps, &mut self.apps) } else { (&mut self.apps, &mut self.hidden_apps) };
            if let Some(i) = from.iter().position(|app| &app.id == id) {
                to.push(App { hidden: !hidden, ..from.remove(i) });
            }
        }
        self.apps.sort_by_cached_key(sort_key);
        self.page = self.page.min(self.pages() - 1);
    }

    pub fn set_grid(&mut self, columns: u32, rows: u32) {
        self.layout = Layout::new(self.layout.width as u32, self.layout.height as u32, columns, rows);
        self.page = self.page.min(self.pages() - 1);
        self.pressed = None;
    }

    /// The app (index into the visible apps) whose icon is under a point on the home screen.
    fn app_at(&self, x: f32, y: f32) -> Option<usize> {
        let l = self.layout;
        let index = self.page * l.per_page() + l.slot_at(x, y)?;
        (index < self.apps.len()).then_some(index)
    }

    /// Long press on an icon: open its app menu beside it. Settings and the Kindle UI stay as they are.
    fn home_long_tap(&mut self, x: f32, y: f32) -> Action {
        let Some(index) = self.app_at(x, y) else { return Action::None };
        if is_builtin(&self.apps[index]) {
            self.notice = Some(format!("{} stays in place", self.apps[index].name));
            return Action::Redraw { full: false };
        }
        self.menu = index;
        self.go(Screen::AppMenu)
    }

    /// The app menu: one of its actions, or a tap outside the box to close it.
    fn app_menu(&mut self, input: Input) -> Action {
        match input {
            Input::PageBack => self.leave_menu(),
            Input::Tap(x, y) => {
                let (anchor, actions) = (self.menu_anchor(), self.menu_actions());
                let hit = (0..actions.len()).find(|&i| self.layout.menu_row(anchor, actions.len(), i).contains(x, y));
                match hit.map(|i| actions[i]) {
                    Some(Button::MenuEarlier) => self.move_app(-1),
                    Some(Button::MenuLater) => self.move_app(1),
                    Some(Button::MenuIcon) => self.next_icon(),
                    Some(Button::MenuHide) => {
                        let action = self.hide_at(self.menu);
                        self.screen = Screen::Home;
                        action
                    }
                    _ if !self.layout.menu_box(anchor, actions.len()).contains(x, y) => self.leave_menu(),
                    _ => Action::None,
                }
            }
            _ => Action::None,
        }
    }

    /// Back to the home screen.
    fn leave_menu(&mut self) -> Action {
        self.page = self.page.min(self.pages().saturating_sub(1));
        self.go(Screen::Home)
    }

    /// Move the edited app one place. The new order of every movable app is saved, so it holds after apps are
    /// added or removed. Built-ins stay at the end. The page follows the app, so the menu stays beside it.
    fn move_app(&mut self, step: i64) -> Action {
        let to = self.menu as i64 + step;
        if to < 0 || to >= self.apps.len() as i64 || is_builtin(&self.apps[to as usize]) {
            self.notice = Some(if step < 0 { "Already first".into() } else { "Already last".into() });
            return Action::Redraw { full: false };
        }
        let to = to as usize;
        self.apps.swap(self.menu, to);
        self.menu = to;
        self.page = to / self.layout.per_page();
        let mut order = Vec::new();
        for (i, app) in self.apps.iter_mut().filter(|a| !is_builtin(a)).enumerate() {
            app.order = Some(i as i64 + 1);
            order.push((app.id.clone(), i as i64 + 1));
        }
        Action::SetOrder(order)
    }

    /// Change icon: the next file in the icons folder, then back to the app's own icon.
    fn next_icon(&mut self) -> Action {
        if self.icon_choices.is_empty() {
            self.notice = Some("No icons: add PNG files to /mnt/us/launcher/icons".into());
            return Action::Redraw { full: false };
        }
        let Some(app) = self.apps.get(self.menu) else { return Action::None };
        let current = match &app.icon {
            IconSource::File(path) => self.icon_choices.iter().position(|c| c == path),
            _ => None,
        };
        let next = match current {
            Some(i) if i + 1 < self.icon_choices.len() => Some(self.icon_choices[i + 1].clone()),
            Some(_) => None,
            None => Some(self.icon_choices[0].clone()),
        };
        let id = app.id.clone();
        let icon = match &next {
            Some(path) => IconSource::File(path.clone()),
            None => self.original_icons.get(&id).cloned().unwrap_or(IconSource::None),
        };
        self.apps[self.menu].icon = icon;
        Action::SetIcon { id, icon: next }
    }

    /// Hide the app at `index` (app menu > Hide).
    fn hide_at(&mut self, index: usize) -> Action {
        if index >= self.apps.len() || is_builtin(&self.apps[index]) {
            return Action::None;
        }
        let app = self.apps.remove(index);
        let id = app.id.clone();
        self.notice = Some(format!("Hidden: {}. Show it again in Settings › Apps.", app.name));
        self.hidden_apps.push(App { hidden: true, ..app });
        self.page = self.page.min(self.pages().saturating_sub(1));
        Action::SetHidden(vec![(id, true)])
    }

    /// Settings > Hidden apps > Show all.
    pub fn show_all(&mut self) -> Action {
        if self.hidden_apps.is_empty() {
            return Action::None;
        }
        let back: Vec<App> = self.hidden_apps.drain(..).map(|a| App { hidden: false, ..a }).collect();
        let changes = back.iter().map(|a| (a.id.clone(), false)).collect();
        self.notice = Some(format!("Showing {} app(s) again", back.len()));
        self.apps.extend(back);
        self.apps.sort_by_cached_key(sort_key);
        Action::SetHidden(changes)
    }

    pub fn pages(&self) -> usize {
        self.layout.pages(self.apps.len())
    }

    /// The apps on the current page, with their slots.
    pub fn page_apps(&self) -> impl Iterator<Item = (usize, &App)> {
        let per = self.layout.per_page();
        self.apps.iter().skip(self.page * per).take(per).enumerate()
    }

    fn turn_page(&mut self, step: i32) -> Action {
        let next = self.page as i32 + step;
        if next < 0 || next >= self.pages() as i32 {
            return Action::None;
        }
        self.page = next as usize;
        Action::Redraw { full: false }
    }

    fn go(&mut self, screen: Screen) -> Action {
        self.screen = screen;
        self.pressed = None;
        Action::Redraw { full: false }
    }

    fn open_settings(&mut self) -> Action {
        self.settings_scroll = 0;
        self.settings_parents.clear();
        self.go(Screen::Settings)
    }

    fn back_from_setting(&mut self) -> Action {
        let target = self.settings_parents.pop().map_or(Screen::Settings, Screen::Page);
        self.go(target)
    }

    /// A tap on a key of the keyboard marks the key as pressed, so the driver can show it before the key's action
    /// runs. Returns whether a key was marked; `handle` clears the mark.
    pub fn press_feedback(&mut self, input: Input) -> bool {
        let Input::Tap(x, y) = input else { return false };
        let keys = match self.screen {
            Screen::Password => self.layout.keyboard_keys(self.password.layer),
            Screen::NetworkName => self.layout.keyboard_keys_with(self.network_name.layer, "Next"),
            Screen::SshPassword => self.layout.keyboard_keys_with(self.ssh_password.layer, "Set"),
            Screen::DeviceName => self.layout.keyboard_keys_with(self.device_name_entry.layer, "Save"),
            Screen::Store if self.store_search_keyboard && !self.store_settings_open => self.layout.keyboard_keys_with(self.store_query.layer, "Search"),
            _ => return false,
        };
        match keys.into_iter().find(|(_, r)| r.contains(x, y)) {
            Some((_, r)) => {
                self.pressed_key = Some(r);
                true
            }
            None => false,
        }
    }

    pub fn handle(&mut self, input: Input) -> Action {
        self.pressed_key = None;
        // While the drive is shown on a computer, the screen takes no taps: the Kindle's files are changing.
        if self.usb == Usb::Drive && !matches!(input, Input::Sleep | Input::Wake) {
            return Action::None;
        }
        match input {
            Input::RestartHold => {
                self.go(Screen::ConfirmRestart);
                return Action::Redraw { full: true };
            }
            Input::Sleep => {
                if self.screen != Screen::Sleep {
                    self.before_sleep = self.screen;
                }
                self.screen = Screen::Sleep;
                self.pressed = None;
                return Action::Redraw { full: true };
            }
            Input::Wake => {
                // `outOfScreenSaver` and `exitingScreenSaver` both arrive: redraw once.
                if self.screen != Screen::Sleep {
                    return Action::None;
                }
                self.screen = self.before_sleep;
                return Action::Redraw { full: true };
            }
            _ if self.screen == Screen::Sleep => return Action::None,
            _ => {}
        }
        if self.installed_busy {
            return Action::None;
        }
        // Hit-test against the frame the user saw if this input also dismisses a notice.
        let store_viewport = if self.screen == Screen::Store { self.store_list_viewport() } else { Rect::default() };
        let had_notice = self.notice.take().is_some();
        let action = match self.screen {
            Screen::Home => self.home(input),
            Screen::Settings => self.settings_home(input),
            Screen::Page(page) => self.settings_page(input, page),
            Screen::WallpaperPicker(target) => self.wallpaper_picker(input, target),
            Screen::WallpaperPreview => self.wallpaper_preview(input),
            Screen::NetworkDetail => self.network_detail(input),
            Screen::Password => self.password_screen(input),
            Screen::NetworkName => self.network_name_screen(input),
            Screen::SshPassword => self.ssh_password_screen(input),
            Screen::DeviceName => self.device_name_screen(input),
            Screen::ConfirmKindle => self.confirm(input),
            Screen::ConfirmRestart => self.confirm_restart(input),
            Screen::PowerMenu => self.power_menu(input),
            Screen::AppMenu => self.app_menu(input),
            Screen::Extensions => self.extensions_list(input),
            Screen::Store => self.store_list(input, store_viewport),
            Screen::StoreApp => self.store_app(input),
            Screen::QuickSettings => self.quick(input),
            Screen::Sleep => Action::None,
        };
        if had_notice && self.screen == Screen::Store {
            self.clamp_store_list_scroll();
        }
        if had_notice && action == Action::None {
            Action::Redraw { full: false }
        } else {
            action
        }
    }

    fn home(&mut self, input: Input) -> Action {
        let l = self.layout;
        match input {
            Input::PageForward => self.turn_page(1),
            Input::PageBack => self.turn_page(-1),
            Input::Swipe { dx, dy, start_y } if dy > SWIPE_MIN && dy.abs() > dx.abs() * 1.5 && start_y <= l.status().bottom() as f32 + 40.0 => {
                // Swipe down from the top edge: quick settings, as Primitive Browser.
                self.go(Screen::QuickSettings)
            }
            Input::Swipe { dx, dy, .. } if dx.abs() > SWIPE_MIN && dx.abs() > dy.abs() * 1.5 => {
                self.turn_page(if dx < 0.0 { 1 } else { -1 })
            }
            Input::LongTap(x, y) => self.home_long_tap(x, y),
            Input::Tap(x, y) => {
                if l.home_footer().contains(x, y) {
                    return self.turn_page(if x < (l.width / 2) as f32 { -1 } else { 1 });
                }
                let Some(index) = self.app_at(x, y) else { return Action::None };
                let app = self.apps[index].clone();
                match app.launch {
                    Launch::Settings => self.open_settings(),
                    Launch::KindleMode => self.go(Screen::ConfirmKindle),
                    Launch::Extensions => self.open_extensions(),
                    _ if app.background => self.run_background(app),
                    _ => {
                        self.pressed = self.icon_rect(index);
                        Action::Launch(app)
                    }
                }
            }
            _ => Action::None,
        }
    }

    /// Quick settings: a button, a level button or step, or the Wi-Fi icon. A swipe up closes it (a tap does not).
    fn quick(&mut self, input: Input) -> Action {
        let l = self.layout;
        match input {
            Input::PageBack => self.go(Screen::Home),
            Input::Swipe { dx, dy, .. } if dy < -SWIPE_MIN && dy.abs() > dx.abs() => self.go(Screen::Home),
            Input::Tap(x, y) => {
                // Quick settings is closed by a swipe up (or the back key), not by a tap.
                if !l.quick_panel().contains(x, y) || l.quick_handle().contains(x, y) {
                    return Action::None;
                }
                if l.header_wifi().inset(-12).contains(x, y) {
                    return Action::Device(DeviceCmd::ToggleWifi);
                }
                if l.header_battery().inset(-12).contains(x, y) {
                    self.settings_scroll = 0;
                    self.settings_parents.clear();
                    return self.go(Screen::Page(Page::Battery));
                }
                if let Some((tile, _)) = l.quick_buttons().into_iter().find(|(_, slot)| slot.contains(x, y)) {
                    return match tile {
                        QuickTile::Airplane => Action::Device(DeviceCmd::ToggleWifi),
                        QuickTile::AutoRotate if self.display_support.auto_rotation => Action::Device(DeviceCmd::ToggleAutoRotation),
                        QuickTile::AutoRotate => Action::None,
                        QuickTile::DarkMode => Action::Device(DeviceCmd::ToggleDarkMode),
                        QuickTile::Animation => Action::Device(DeviceCmd::ToggleAnimation),
                        QuickTile::ScreenTimeout => Action::Device(DeviceCmd::CycleScreenTimeout),
                        QuickTile::Ssh => Action::Device(DeviceCmd::ToggleSsh),
                        QuickTile::AllSettings => self.open_settings(),
                        QuickTile::Power => self.go(Screen::PowerMenu),
                    };
                }
                for row in 0..2 {
                    let supported = if row == 0 { self.display_support.front_light } else { self.display_support.warmth };
                    if !supported {
                        continue;
                    }
                    let level = |step: i32| if row == 0 { DeviceCmd::Light(step) } else { DeviceCmd::Warmth(step) };
                    if l.quick_minus(row).inset(-10).contains(x, y) {
                        return Action::Device(level(-1));
                    }
                    if l.quick_plus(row).inset(-10).contains(x, y) {
                        return Action::Device(level(1));
                    }
                    if let Some(to) = l.quick_step(row, x, y) {
                        return Action::Device(if row == 0 { DeviceCmd::SetLight(to) } else { DeviceCmd::SetWarmth(to) });
                    }
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    /// The network screen: Connect (or Enter the password for a new secured network), Forget for a saved one, and
    /// Back to the Wi-Fi page.
    fn network_detail(&mut self, input: Input) -> Action {
        let Some(net) = self.wifi_target.clone() else { return self.go(Screen::Settings) };
        match input {
            Input::PageBack => self.go(Screen::Page(Page::Wifi)),
            Input::Tap(x, y) => {
                if self.layout.done_button().inset(-20).contains(x, y) {
                    return self.go(Screen::Page(Page::Wifi));
                }
                if self.layout.net_connect().inset(-12).contains(x, y) {
                    if net.secured && !net.known {
                        self.password = primitive_keyboard::Editor::new(MAX_PASSWORD);
                        self.show_password = false;
                        self.wifi_hidden = false;
                        return self.go(Screen::Password);
                    }
                    return self.join(net.name, net.secured, None, net.known);
                }
                if net.known && self.layout.net_forget().inset(-12).contains(x, y) {
                    return Action::ForgetWifi(net.name);
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    /// The password screen: type the password on the keyboard (its Go key joins), show or hide it. Back returns to
    /// the network screen.
    /// The SSH password: type it on the keyboard, then Set gives it to the driver. Back returns to the SSH & DNS page.
    /// The password is not shown after this screen and is never logged.
    fn device_name_screen(&mut self, input: Input) -> Action {
        match input {
            Input::PageBack => self.go(Screen::Page(Page::Device)),
            Input::Tap(x, y) => {
                if self.layout.done_button().inset(-20).contains(x, y) {
                    return self.go(Screen::Page(Page::Device));
                }
                let Some(key) = self.layout.keyboard_key_at(self.device_name_entry.layer, x, y) else { return Action::None };
                if self.device_name_entry.press(key) != primitive_keyboard::Press::Go {
                    return Action::Redraw { full: false };
                }
                let name = self.device_name_entry.string().trim().to_string();
                if !valid_device_name(&name) {
                    self.notice = Some(format!("Use 1 to {MAX_DEVICE_NAME} letters, digits or spaces."));
                    return Action::Redraw { full: false };
                }
                self.device_name_entry = primitive_keyboard::Editor::new(MAX_DEVICE_NAME);
                self.notice = Some("Saving the name…".into());
                self.screen = Screen::Page(Page::Device);
                Action::SetDeviceName(name)
            }
            _ => Action::None,
        }
    }

    fn ssh_password_screen(&mut self, input: Input) -> Action {
        match input {
            Input::PageBack => self.go(Screen::Page(Page::Connection)),
            Input::Tap(x, y) => {
                if self.layout.done_button().inset(-20).contains(x, y) {
                    return self.go(Screen::Page(Page::Connection));
                }
                let Some(key) = self.layout.keyboard_key_at(self.ssh_password.layer, x, y) else { return Action::None };
                if self.ssh_password.press(key) != primitive_keyboard::Press::Go {
                    return Action::Redraw { full: false };
                }
                let password = self.ssh_password.string();
                if !valid_ssh_password(&password) {
                    self.notice = Some(format!("Use {MIN_SSH_PASSWORD} to {MAX_SSH_PASSWORD} letters, digits or - _ . @ # % + = !, no spaces."));
                    return Action::Redraw { full: false };
                }
                self.ssh_password = primitive_keyboard::Editor::new(MAX_SSH_PASSWORD);
                self.notice = Some("Setting the SSH password…".into());
                self.screen = Screen::Page(Page::Connection);
                Action::SetSshPassword(password)
            }
            _ => Action::None,
        }
    }

    fn password_screen(&mut self, input: Input) -> Action {
        let Some(net) = self.wifi_target.clone() else { return self.go(Screen::Settings) };
        let back = if self.wifi_hidden { Screen::NetworkName } else { Screen::NetworkDetail };
        match input {
            Input::PageBack => self.go(back),
            Input::Tap(x, y) => {
                if self.layout.done_button().inset(-20).contains(x, y) {
                    return self.go(back);
                }
                if self.layout.password_show().inset(-12).contains(x, y) {
                    self.show_password = !self.show_password;
                    return Action::Redraw { full: false };
                }
                let Some(key) = self.layout.keyboard_key_at(self.password.layer, x, y) else { return Action::None };
                match self.password.press(key) {
                    primitive_keyboard::Press::Go => {
                        if self.wifi_hidden && self.password.text.is_empty() {
                            return self.join(net.name, false, None, false);
                        }
                        if self.password.text.len() < MIN_PASSWORD {
                            self.notice = Some(format!("A Wi-Fi password has at least {MIN_PASSWORD} characters."));
                            return Action::Redraw { full: false };
                        }
                        let password = self.password.string();
                        self.join(net.name, true, Some(password), false)
                    }
                    _ => Action::Redraw { full: false },
                }
            }
            _ => Action::None,
        }
    }

    /// The name of a network that is not in the list: type it, then Next goes to its password (an empty password
    /// joins it as an open network). Back returns to the Wi-Fi page.
    fn network_name_screen(&mut self, input: Input) -> Action {
        match input {
            Input::PageBack => self.go(Screen::Page(Page::Wifi)),
            Input::Tap(x, y) => {
                if self.layout.done_button().inset(-20).contains(x, y) {
                    return self.go(Screen::Page(Page::Wifi));
                }
                let Some(key) = self.layout.keyboard_key_at(self.network_name.layer, x, y) else { return Action::None };
                if self.network_name.press(key) == primitive_keyboard::Press::Go {
                    let name = self.network_name.string().trim().to_string();
                    if name.is_empty() {
                        self.notice = Some("Type the network's name.".into());
                        return Action::Redraw { full: false };
                    }
                    self.wifi_target = Some(crate::wifi::Network { name, level: 0, max: 5, secured: true, known: false });
                    self.wifi_hidden = true;
                    self.password = primitive_keyboard::Editor::new(MAX_PASSWORD);
                    self.show_password = false;
                    return self.go(Screen::Password);
                }
                Action::Redraw { full: false }
            }
            _ => Action::None,
        }
    }

    /// Start joining: back on the Wi-Fi page while it connects (the driver reports the result).
    fn join(&mut self, name: String, secured: bool, password: Option<String>, saved: bool) -> Action {
        self.password = primitive_keyboard::Editor::new(MAX_PASSWORD);
        self.notice = Some(format!("Connecting to {name}…"));
        self.wifi_scanning = true;
        self.screen = Screen::Page(Page::Wifi);
        self.pressed = None;
        Action::JoinWifi { name, secured, password, saved }
    }

    /// The result of a join, shown in the footer.
    pub fn set_join_result(&mut self, ok: bool, message: String) {
        self.notice = Some(if ok { message } else { format!("Could not connect: {message}") });
    }

    /// Settings: the list of pages. A tap on a page opens it; Done (or the back key) goes to the home screen.
    fn settings_home(&mut self, input: Input) -> Action {
        match input {
            Input::PageBack if self.settings_scroll == 0 => self.go(Screen::Home),
            Input::Tap(x, y) => {
                if self.layout.done_button().inset(-20).contains(x, y) {
                    return self.go(Screen::Home);
                }
                match self.layout.settings_home_at(self.settings_scroll, x, y) {
                    Some(page) => {
                        self.settings_parents.clear();
                        self.go(Screen::Page(page))
                    }
                    None => Action::None,
                }
            }
            _ => scroll_settings_list(input, self.layout.settings_viewport(), &mut self.settings_scroll, self.layout.settings_scroll_max()),
        }
    }

    fn app_visibility(&mut self, input: Input) -> Action {
        match input {
            Input::PageBack if self.visibility_scroll == 0 => self.back_from_setting(),
            Input::Tap(x, y) if self.layout.done_button().inset(-20).contains(x, y) => self.back_from_setting(),
            Input::Tap(x, y) => {
                let Some(index) = self.layout.visibility_at(self.visibility_scroll, x, y) else { return Action::None };
                let Some((id, hidden)) = self.visibility_apps().get(index).map(|app| (app.id.clone(), app.hidden)) else {
                    return Action::None;
                };
                if hidden {
                    let Some(index) = self.hidden_apps.iter().position(|app| app.id == id) else { return Action::None };
                    self.apps.push(App { hidden: false, ..self.hidden_apps.remove(index) });
                    self.apps.sort_by_cached_key(sort_key);
                    return Action::SetHidden(vec![(id, false)]);
                }
                let Some(index) = self.apps.iter().position(|app| app.id == id) else { return Action::None };
                self.hide_at(index)
            }
            _ => {
                let max = self.visibility_scroll_max();
                scroll_settings_list(input, self.layout.visibility_viewport(), &mut self.visibility_scroll, max)
            }
        }
    }

    fn installed_apps_page(&mut self, input: Input) -> Action {
        let l = self.layout;
        if let Some(index) = self.installed_remove_confirm {
            let [(_, cancel), (_, uninstall)] = l.confirm_buttons();
            match input {
                Input::Tap(x, y) if uninstall.inset(-12).contains(x, y) => {
                    self.installed_remove_confirm = None;
                    return Action::Device(DeviceCmd::RemoveInstalledApp(index));
                }
                Input::Tap(x, y) if !l.confirm_box().contains(x, y) || cancel.inset(-12).contains(x, y) => {}
                Input::PageBack => {}
                _ => return Action::None,
            }
            self.installed_remove_confirm = None;
            return Action::Redraw { full: false };
        }

        match input {
            Input::PageBack if self.installed_scroll == 0 => self.back_from_setting(),
            Input::Tap(x, y) if l.done_button().inset(-20).contains(x, y) => self.back_from_setting(),
            Input::Tap(x, y) => {
                let Some(index) = l.visibility_at(self.installed_scroll, x, y) else { return Action::None };
                let row = l.visibility_row(index, self.installed_scroll);
                if !self.installed_app(index).is_some_and(|app| app.can_remove)
                    || !l.installed_remove_button(row).contains(x, y)
                {
                    return Action::None;
                }
                self.installed_remove_confirm = Some(index);
                Action::Redraw { full: false }
            }
            _ => {
                let max = self.installed_scroll_max();
                scroll_settings_list(input, l.visibility_viewport(), &mut self.installed_scroll, max)
            }
        }
    }

    fn wallpaper_picker(&mut self, input: Input, target: WallpaperTarget) -> Action {
        match input {
            Input::PageBack if self.wallpaper_scroll == 0 => self.go(Screen::Page(Page::Wallpaper)),
            Input::Tap(x, y) if self.layout.done_button().inset(-20).contains(x, y) => {
                self.go(Screen::Page(Page::Wallpaper))
            }
            Input::Tap(x, y) => {
                let Some(index) = self.layout.visibility_at(self.wallpaper_scroll, x, y) else { return Action::None };
                let Some(choice) = self.wallpapers.choices(target).nth(index) else { return Action::None };
                self.wallpaper_preview = Some(WallpaperDraft {
                    target,
                    choice: choice.clone(),
                    opacity: self.wallpapers.opacity(target),
                    error: None,
                });
                self.go(Screen::WallpaperPreview);
                Action::PreviewWallpaper { target, choice }
            }
            _ => {
                let max = self.wallpaper_scroll_max(target);
                scroll_settings_list(input, self.layout.visibility_viewport(), &mut self.wallpaper_scroll, max)
            }
        }
    }

    fn wallpaper_preview(&mut self, input: Input) -> Action {
        let (x, y) = match input {
            Input::PageBack => return self.finish_wallpaper_preview(),
            Input::Tap(x, y) if self.layout.done_button().inset(-20).contains(x, y) => {
                return self.finish_wallpaper_preview();
            }
            Input::Tap(x, y) => (x, y),
            _ => return Action::None,
        };
        let layout = self.layout.wallpaper_preview();
        let Some(draft) = &mut self.wallpaper_preview else { return Action::None };
        if layout.apply.contains(x, y) && draft.can_apply() {
            return Action::SetWallpaper {
                target: draft.target,
                choice: draft.choice.clone(),
                opacity: draft.opacity,
            };
        }
        if !draft.has_opacity() {
            return Action::None;
        }

        const OPACITY_STEP: u8 = 10;
        let opacity = if layout.opacity_down.contains(x, y) {
            draft.opacity.saturating_sub(OPACITY_STEP)
        } else if layout.opacity_up.contains(x, y) {
            draft.opacity.saturating_add(OPACITY_STEP).min(100)
        } else if layout.opacity_track.contains(x, y) {
            let ratio = (x - layout.opacity_track.x as f32) / layout.opacity_track.w as f32;
            (ratio * (100 / OPACITY_STEP) as f32).round() as u8 * OPACITY_STEP
        } else {
            return Action::None;
        };
        if opacity == draft.opacity {
            return Action::None;
        }
        draft.opacity = opacity;
        Action::Redraw { full: false }
    }

    /// One page of Settings: its rows' buttons. Back (or the back key) returns to the Settings list.
    fn settings_page(&mut self, input: Input, page: Page) -> Action {
        if page == Page::AppVisibility {
            return self.app_visibility(input);
        }
        if page == Page::InstalledApps {
            return self.installed_apps_page(input);
        }
        if page == Page::Display {
            if let Input::Tap(x, y) = input {
                match self.layout.display_level_at(x, y) {
                    Some((crate::layout::Row::Frontlight, level)) if self.display_support.front_light => {
                        return Action::Device(DeviceCmd::SetLight(level));
                    }
                    Some((crate::layout::Row::Warmth, level)) if self.display_support.warmth => {
                        return Action::Device(DeviceCmd::SetWarmth(level));
                    }
                    _ => {}
                }
            }
        }
        if page == Page::Store && self.store_clear_confirm {
            return self.confirm_clear_store_cache(input);
        }
        match input {
            Input::PageBack => self.back_from_setting(),
            Input::Swipe { dy, dx, .. } if page == Page::Wifi && dy.abs() > SWIPE_MIN && dy.abs() > dx.abs() => {
                // The list moves by about as many rows as the swipe covers (at least one).
                let rows = ((dy.abs() / crate::layout::NETWORK_H as f32) as usize).max(1);
                let next = if dy < 0.0 { self.wifi_scroll + rows } else { self.wifi_scroll.saturating_sub(rows) };
                self.wifi_scroll = next.min(self.wifi_max_scroll());
                Action::Redraw { full: false }
            }
            Input::Tap(x, y) if page == Page::Wifi && self.layout.wifi_area().current.contains(x, y) => {
                // The connected network: its screen, where it can be forgotten.
                match self.wifi.current.clone() {
                    Some(net) => {
                        self.wifi_target = Some(net);
                        self.go(Screen::NetworkDetail)
                    }
                    None => Action::None,
                }
            }
            Input::Tap(x, y) if page == Page::Wifi && self.layout.scan_at(x, y) => {
                self.wifi_scanning = true;
                Action::ScanWifi
            }
            Input::Tap(x, y) if page == Page::Wifi && self.layout.wifi_other().inset(-12).contains(x, y) => {
                self.network_name = primitive_keyboard::Editor::new(MAX_NAME);
                self.go(Screen::NetworkName)
            }
            Input::Tap(x, y) if page == Page::Wifi && self.layout.network_at(x, y).is_some() => {
                let index = self.wifi_scroll + self.layout.network_at(x, y).unwrap_or(0);
                match self.wifi.networks.get(index).cloned() {
                    Some(net) => {
                        self.wifi_target = Some(net);
                        self.go(Screen::NetworkDetail)
                    }
                    None => Action::None,
                }
            }
            Input::Tap(x, y) => match self.layout.settings_button_at(page, x, y) {
                Some(Button::Done) => self.back_from_setting(),
                Some(Button::OpenPage(target)) => {
                    self.settings_parents.push(page);
                    self.visibility_scroll = 0;
                    self.go(Screen::Page(target))
                }
                Some(Button::OpenWallpaperPicker(target)) => {
                    self.wallpaper_scroll = 0;
                    self.go(Screen::WallpaperPicker(target))
                }
                Some(Button::ManageInstalled) => {
                    self.settings_parents.push(page);
                    self.installed_scroll = 0;
                    self.set_installed_apps(None);
                    self.go(Screen::Page(Page::InstalledApps));
                    Action::Device(DeviceCmd::OpenInstalledApps)
                }
                Some(Button::LowPower) => Action::Device(DeviceCmd::ToggleLowPower),
                Some(Button::GridColumns) => Action::SetGrid {
                    columns: if self.layout.cols == 5 { 3 } else { self.layout.cols as u32 + 1 },
                    rows: self.layout.rows as u32,
                },
                Some(Button::GridRows) => Action::SetGrid {
                    columns: self.layout.cols as u32,
                    rows: if self.layout.rows == 6 { 3 } else { self.layout.rows as u32 + 1 },
                },
                Some(Button::LightDown) if self.display_support.front_light => Action::Device(DeviceCmd::Light(-1)),
                Some(Button::LightUp) if self.display_support.front_light => Action::Device(DeviceCmd::Light(1)),
                Some(Button::WarmDown) if self.display_support.warmth => Action::Device(DeviceCmd::Warmth(-1)),
                Some(Button::WarmUp) if self.display_support.warmth => Action::Device(DeviceCmd::Warmth(1)),
                Some(Button::Wifi) => Action::Device(DeviceCmd::ToggleWifi),
                Some(Button::Rotation) => Action::Device(DeviceCmd::ToggleRotation),
                Some(Button::Ssh) => Action::Device(DeviceCmd::ToggleSsh),
                Some(Button::PublicDns) => Action::Device(DeviceCmd::ToggleDns),
                Some(Button::SshCheck) => Action::CheckSsh,
                Some(Button::SshPassword) => {
                    self.ssh_password = primitive_keyboard::Editor::new(MAX_SSH_PASSWORD);
                    self.go(Screen::SshPassword)
                }
                Some(Button::DeviceName) => {
                    self.device_name_entry = primitive_keyboard::Editor::new(MAX_DEVICE_NAME);
                    if let Some(name) = self.device_name.clone() {
                        for c in name.chars() {
                            let key = if c == ' ' { primitive_keyboard::Key::Space } else { primitive_keyboard::Key::Char(c) };
                            self.device_name_entry.press(key);
                        }
                    }
                    self.go(Screen::DeviceName)
                }
                Some(Button::Autostart) => Action::Device(DeviceCmd::ToggleAutostart),
                Some(Button::PickModel(index)) => Action::Device(DeviceCmd::PickModel(index)),
                Some(Button::ShowHidden) => self.show_all(),
                Some(Button::StoreDailyCheck) => Action::Device(DeviceCmd::StoreToggleDailyCheck),
                Some(Button::StoreClearCache) => {
                    self.store_clear_confirm = true;
                    Action::Redraw { full: false }
                }
                Some(Button::SyncClock) if !self.clock_syncing => Action::Device(DeviceCmd::SyncClock),
                Some(Button::RefreshTimeZone) => Action::Device(DeviceCmd::RefreshTimeZone),
                Some(Button::SleepScreen) => Action::Device(DeviceCmd::ToggleSleepScreen),
                Some(Button::RotationAuto) if self.display_support.auto_rotation => Action::Device(DeviceCmd::ToggleAutoRotation),
                Some(Button::DarkMode) => Action::Device(DeviceCmd::ToggleDarkMode),
                Some(Button::Animation) => Action::Device(DeviceCmd::ToggleAnimation),
                Some(Button::ScreenTimeout) => Action::Device(DeviceCmd::CycleScreenTimeout),
                Some(Button::LargeText) => Action::Device(DeviceCmd::ToggleLargeText),
                Some(Button::HighContrast) => Action::Device(DeviceCmd::ToggleHighContrast),
                Some(Button::KindleUi) => self.go(Screen::ConfirmKindle),
                Some(Button::Restart) => Action::Device(DeviceCmd::RestartLauncher),
                _ => Action::None,
            },
            _ => Action::None,
        }
    }

    /// The power menu: Restart and Power off run the device's command; Cancel, or a tap outside the box, goes home.
    fn power_menu(&mut self, input: Input) -> Action {
        let Input::Tap(x, y) = input else { return Action::None };
        let [(_, cancel), (_, kindle), (_, restart), (_, off)] = self.layout.power_buttons();
        if kindle.inset(-12).contains(x, y) {
            // Opening the Kindle UI takes about a minute: it is confirmed first.
            self.go(Screen::ConfirmKindle)
        } else if off.inset(-12).contains(x, y) {
            Action::Device(DeviceCmd::PowerOff)
        } else if restart.inset(-12).contains(x, y) {
            Action::Device(DeviceCmd::Restart)
        } else if cancel.inset(-12).contains(x, y) || !self.layout.power_box().contains(x, y) {
            self.go(Screen::Home)
        } else {
            Action::None
        }
    }

    fn confirm_restart(&mut self, input: Input) -> Action {
        let Input::Tap(x, y) = input else { return Action::None };
        let [(_, cancel), (_, restart)] = self.layout.confirm_buttons();
        if restart.inset(-12).contains(x, y) {
            Action::Device(DeviceCmd::QuitAppAndRestart)
        } else if cancel.inset(-12).contains(x, y) || !self.layout.confirm_box().contains(x, y) {
            self.go(Screen::Home)
        } else {
            Action::None
        }
    }

    fn confirm(&mut self, input: Input) -> Action {
        let Input::Tap(x, y) = input else { return Action::None };
        let [(_, cancel), (_, open)] = self.layout.confirm_buttons();
        if open.inset(-12).contains(x, y) {
            Action::KindleMode
        } else if cancel.inset(-12).contains(x, y) || !self.layout.confirm_box().contains(x, y) {
            self.go(Screen::Home)
        } else {
            Action::None
        }
    }
}

/// Settings, the Kindle UI and Extensions: they cannot be moved, hidden or given another icon.
fn is_builtin(app: &App) -> bool {
    matches!(app.launch, Launch::Settings | Launch::KindleMode | Launch::Extensions | Launch::Store)
}

/// A device name the owner can use: 1 to 32 characters, letters, digits and spaces only.
pub fn valid_device_name(name: &str) -> bool {
    let len = name.chars().count();
    (1..=MAX_DEVICE_NAME).contains(&len) && name.chars().all(|c| c.is_alphanumeric() || c == ' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_name_is_1_to_32_letters_digits_and_spaces() {
        assert!(valid_device_name("Doan's Oasis".replace('\'', "").as_str()));
        assert!(valid_device_name("My Kindle 2"));
        assert!(!valid_device_name(""));
        assert!(!valid_device_name(&"a".repeat(33)));
        assert!(!valid_device_name("tab\there"));
    }

    #[test]
    fn date_and_time_actions_are_reachable_and_sync_cannot_overlap_or_run_in_usb_mode() {
        use crate::layout::Row;
        let mut u = ui(22);
        u.open_settings();
        u.handle(Input::Swipe { dx: 0.0, dy: -900.0, start_y: 1200.0 });
        let page = u.layout.settings_home_rows(u.settings_scroll).into_iter()
            .find(|(page, _)| *page == Page::DateTime).unwrap().1;
        u.handle(centre(page));
        assert_eq!(u.screen, Screen::Page(Page::DateTime));
        let row = |row| u.layout.page_sections(Page::DateTime).into_iter()
            .flat_map(|section| section.rows).find(|item| item.row == row).unwrap().rect;
        let (sync, refresh) = (row(Row::SyncClock), row(Row::TimeZone));
        assert_eq!(u.handle(centre(sync)), Action::Device(DeviceCmd::SyncClock));
        u.clock_syncing = true;
        assert_eq!(u.handle(centre(sync)), Action::None);
        assert_eq!(u.handle(centre(refresh)), Action::Device(DeviceCmd::RefreshTimeZone));
        u.usb = Usb::Drive;
        assert_eq!(u.handle(centre(refresh)), Action::None);
    }

    #[test]
    fn wallpaper_picker_scrolls_selects_and_returns_through_nested_settings() {
        use crate::layout::Row;
        let mut u = ui(22);
        let mut wallpapers = WallpaperSettings {
            images: (0..25).map(|i| format!("Image {i:02}.png")).collect(),
            ..WallpaperSettings::default()
        };
        u.set_wallpapers(wallpapers.clone());
        u.open_settings();
        let row = |u: &Ui, page, label| u.layout.page_sections(page).into_iter()
            .flat_map(|section| section.rows).find(|item| item.row == label).unwrap().rect;
        let display = u.layout.settings_home_rows(0).into_iter().find(|(page, _)| *page == Page::Display).unwrap().1;
        u.handle(centre(display));
        u.handle(centre(row(&u, Page::Display, Row::WallpaperLink)));
        assert_eq!(u.screen, Screen::Page(Page::Wallpaper));
        let target = WallpaperTarget::Sleep;
        u.handle(centre(row(&u, Page::Wallpaper, Row::Wallpaper(target))));
        assert_eq!(u.screen, Screen::WallpaperPicker(target));
        assert_eq!(u.wallpaper_scroll, 0);
        assert_eq!(u.handle(centre(u.layout.visibility_row(0, 0))), Action::PreviewWallpaper {
            target,
            choice: WallpaperChoice::Default,
        });
        assert_eq!(u.screen, Screen::WallpaperPreview);
        assert!(!u.wallpaper_preview.as_ref().unwrap().has_opacity());
        u.handle(Input::PageBack);
        assert_eq!(u.handle(centre(u.layout.visibility_row(1, 0))), Action::PreviewWallpaper {
            target,
            choice: WallpaperChoice::Rotate,
        });
        u.handle(Input::PageBack);
        u.handle(Input::Swipe { dx: 0.0, dy: -3000.0, start_y: 1400.0 });
        assert_eq!(u.wallpaper_scroll, u.wallpaper_scroll_max(target));
        assert_eq!(u.handle(centre(u.layout.visibility_row(26, u.wallpaper_scroll))), Action::PreviewWallpaper {
            target,
            choice: WallpaperChoice::File("Image 24.png".into()),
        });
        wallpapers.images.truncate(1);
        u.set_wallpapers(wallpapers);
        assert_eq!(u.wallpaper_scroll, 0);
        assert_eq!(u.handle(centre(u.layout.wallpaper_preview().apply)), Action::None);
        u.handle(Input::PageBack);
        for expected in [Screen::Page(Page::Wallpaper), Screen::Page(Page::Display), Screen::Settings]
        {
            u.handle(centre(u.layout.done_button()));
            assert_eq!(u.screen, expected);
        }
        assert_eq!(u.wallpapers.choices(WallpaperTarget::Startup).count(), 2);
    }

    #[test]
    fn wallpaper_opacity_is_a_per_target_draft_until_apply() {
        let mut u = ui(22);
        let target = WallpaperTarget::Home;
        let choice = WallpaperChoice::File("Pines.png".into());
        let mut wallpapers = WallpaperSettings {
            images: vec!["Pines.png".into()],
            home: choice.clone(),
            ..WallpaperSettings::default()
        };
        wallpapers.set_opacity(target, 60);
        u.set_wallpapers(wallpapers.clone());
        u.screen = Screen::WallpaperPicker(target);
        let picture_row = u.layout.visibility_row(1, 0);
        let controls = u.layout.wallpaper_preview();
        assert_eq!(u.handle(centre(picture_row)), Action::PreviewWallpaper { target, choice: choice.clone() });
        assert_eq!(u.wallpaper_preview.as_ref().unwrap().opacity, 60);
        assert_eq!(u.handle(centre(controls.opacity_down)), Action::Redraw { full: false });
        assert_eq!(u.wallpaper_preview.as_ref().unwrap().opacity, 50);
        u.handle(centre(u.layout.done_button()));
        assert_eq!(u.screen, Screen::WallpaperPicker(target));
        assert_eq!(u.wallpapers, wallpapers);
        assert!(u.wallpaper_preview.is_none());

        u.handle(centre(picture_row));
        assert_eq!(u.handle(centre(controls.opacity_track)), Action::Redraw { full: false });
        for _ in 0..2 {
            assert_eq!(u.handle(centre(controls.opacity_up)), Action::Redraw { full: false });
        }
        assert_eq!(u.handle(centre(controls.apply)), Action::SetWallpaper { target, choice, opacity: 70 });
        assert_eq!(u.wallpapers.opacity(WallpaperTarget::Home), 60);
        assert_eq!(u.wallpapers.opacity(WallpaperTarget::Sleep), 100);
        assert_eq!(u.wallpapers.opacity(WallpaperTarget::Startup), 100);
        u.finish_wallpaper_preview();
        assert_eq!(u.screen, Screen::WallpaperPicker(target));
        assert!(u.wallpaper_preview.is_none());
    }

    #[test]
    fn settings_app_management_and_battery_shortcuts_follow_navigation() {
        use crate::layout::Row;
        let mut u = ui(22);
        u.open_settings();
        let swipe = |dy| Input::Swipe { dx: 0.0, dy, start_y: 1300.0 };
        assert!(matches!(u.handle(swipe(-900.0)), Action::Redraw { .. }));
        let home_scroll = u.settings_scroll;
        let home_row = |ui: &Ui, page| ui.layout.settings_home_rows(ui.settings_scroll)
            .into_iter().find(|(candidate, _)| *candidate == page).unwrap().1;
        let setting_row = |ui: &Ui, page, row| ui.layout.page_sections(page).into_iter()
            .flat_map(|section| section.rows).find(|entry| entry.row == row).unwrap().rect;

        u.handle(centre(home_row(&u, Page::Apps)));
        assert_eq!(u.screen, Screen::Page(Page::Apps));
        let manage = setting_row(&u, Page::Apps, Row::ManageInstalled);
        assert_eq!(u.handle(centre(manage)), Action::Device(DeviceCmd::OpenInstalledApps));
        assert_eq!(u.screen, Screen::Page(Page::InstalledApps));
        assert_eq!(u.handle(Input::PageBack), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Page(Page::Apps));
        let columns = setting_row(&u, Page::Apps, Row::GridColumns);
        assert_eq!(u.handle(centre(columns)), Action::SetGrid { columns: 4, rows: 3 });
        u.set_grid(4, 3);
        assert_eq!(u.layout.cols, 4);

        u.handle(centre(setting_row(&u, Page::Apps, Row::AppVisibility)));
        assert_eq!(u.screen, Screen::Page(Page::AppVisibility));
        let row = u.layout.visibility_row(0, 0);
        let Action::SetHidden(changes) = u.handle(centre(row)) else { panic!("visibility row did not toggle") };
        assert_eq!(u.hidden_count(), 1);
        u.restore_visibility(&changes);
        assert_eq!(u.hidden_count(), 0);
        u.handle(centre(row));
        let Action::SetHidden(reveal) = u.handle(centre(row)) else { panic!("hidden app was not reachable") };
        assert_eq!(reveal, vec![(changes[0].0.clone(), false)]);
        u.handle(swipe(-2000.0));
        assert_eq!(u.visibility_scroll, u.visibility_scroll_max());
        let (index, last) = u.visible_visibility_apps().last().unwrap();
        let id = last.id.clone();
        let last_visible_row = u.layout.visibility_row(index, u.visibility_scroll);
        assert_eq!(u.handle(centre(last_visible_row)), Action::SetHidden(vec![(id, true)]));
        u.handle(centre(u.layout.done_button()));
        assert_eq!(u.screen, Screen::Page(Page::Apps));
        u.handle(Input::PageBack);
        assert_eq!(u.settings_scroll, home_scroll);

        u.go(Screen::QuickSettings);
        u.handle(centre(u.layout.header_battery()));
        assert_eq!(u.screen, Screen::Page(Page::Battery));
        let power = setting_row(&u, Page::Battery, Row::LowPower);
        assert_eq!(u.handle(centre(power)), Action::Device(DeviceCmd::ToggleLowPower));

        u.handle(Input::PageBack);
        u.handle(swipe(-900.0));
        u.handle(centre(home_row(&u, Page::Storage)));
        let manage = setting_row(&u, Page::Storage, Row::ManageInstalled);
        assert_eq!(u.handle(centre(manage)), Action::Device(DeviceCmd::OpenInstalledApps));
        assert_eq!(u.screen, Screen::Page(Page::InstalledApps));
        assert_eq!(u.handle(centre(u.layout.done_button())), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Page(Page::Storage));
    }

    #[test]
    fn installed_apps_scroll_and_uninstall_only_after_confirmation() {
        let mut u = ui(0);
        let layout = u.layout;
        let app = |index| InstalledApp {
            id: format!("app-{index}"),
            name: format!("App {index}"),
            version: "1.0".into(),
            can_remove: index != 0,
        };
        let button = |index, scroll| centre(layout.installed_remove_button(layout.visibility_row(index, scroll)));
        let [(_, cancel), (_, confirm)] = layout.confirm_buttons();

        u.screen = Screen::Page(Page::InstalledApps);
        assert!(u.installed_apps().is_none());
        u.set_installed_apps(Some((0..30).map(app).collect()));
        assert_eq!(u.handle(button(0, 0)), Action::None);
        assert_eq!(u.handle(centre(layout.visibility_row(1, 0))), Action::None);
        assert!(u.installed_confirmation().is_none());
        assert_eq!(u.handle(button(1, 0)), Action::Redraw { full: false });
        assert_eq!(u.installed_confirmation(), u.installed_app(1));
        assert_eq!(u.handle(centre(cancel)), Action::Redraw { full: false });
        assert!(u.installed_confirmation().is_none());
        u.handle(button(1, 0));
        assert_eq!(u.handle(Input::PageBack), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Page(Page::InstalledApps));
        assert!(u.installed_confirmation().is_none());

        assert_eq!(u.handle(Input::Swipe { dx: 0.0, dy: -6000.0, start_y: 600.0 }), Action::Redraw { full: false });
        let offset = u.installed_scroll;
        assert_eq!(offset, u.installed_scroll_max());
        assert!(offset > 0);
        assert_eq!(u.handle(button(0, offset)), Action::None);
        assert_eq!(u.handle(button(29, offset)), Action::Redraw { full: false });
        assert_eq!(u.handle(centre(confirm)), Action::Device(DeviceCmd::RemoveInstalledApp(29)));
        assert!(u.installed_confirmation().is_none());

        u.installed_busy = true;
        u.notice = Some("Uninstalling".into());
        assert_eq!(u.handle(button(29, offset)), Action::None);
        assert_eq!(u.handle(centre(layout.done_button())), Action::None);
        assert_eq!(u.handle(Input::PageBack), Action::None);
        assert_eq!(u.notice.as_deref(), Some("Uninstalling"));
        assert_eq!(u.handle(Input::Sleep), Action::Redraw { full: true });
        assert_eq!(u.screen, Screen::Sleep);
        assert_eq!(u.handle(Input::Wake), Action::Redraw { full: true });
        assert_eq!(u.screen, Screen::Page(Page::InstalledApps));
        u.installed_busy = false;
        u.handle(button(29, offset));
        u.set_installed_apps(Some(vec![app(0)]));
        assert_eq!(u.installed_scroll, 0);
        assert!(u.installed_confirmation().is_none());
        assert_eq!(u.handle(centre(confirm)), Action::None);
    }

    #[test]
    fn settings_scrolls_by_swipe_distance_with_a_fixed_header_and_bounded_offsets() {
        let mut u = ui(0);
        u.open_settings();
        let swipe = |dx, dy, start_y| Input::Swipe { dx, dy, start_y };
        assert_eq!(u.handle(swipe(40.0, -180.0, 900.0)), Action::Redraw { full: false });
        assert_eq!(u.settings_scroll, 180);
        assert_eq!(u.handle(swipe(180.0, -40.0, 900.0)), Action::None);
        assert_eq!(u.handle(swipe(0.0, -250.0, 85.0)), Action::None);
        assert_eq!(u.settings_scroll, 180);
        u.handle(swipe(0.0, 80.0, 950.0));
        assert_eq!(u.settings_scroll, 100);
        u.handle(swipe(0.0, 900.0, 900.0));
        assert_eq!(u.settings_scroll, 0);
        u.handle(Input::PageForward);
        assert_eq!(u.settings_scroll, u.layout.settings_scroll_max());
        assert_eq!(u.handle(Input::Tap(50.0, 80.0)), Action::None);
        u.handle(Input::PageBack);
        assert_eq!(u.settings_scroll, 0);
        u.handle(Input::PageBack);
        assert_eq!(u.screen, Screen::Home);
    }
    use crate::apps::IconSource;

    fn app(id: &str, launch: Launch) -> App {
        App { id: id.into(), name: id.into(), icon: IconSource::None, launch, dir: "/".into(), order: None, hidden: false, background: false }
    }

    fn script(id: &str) -> App {
        app(id, Launch::Script { path: format!("/d/{id}"), hooks: false })
    }

    fn ui(n: usize) -> Ui {
        let mut apps: Vec<App> = (0..n).map(|i| script(&format!("app{i}.sh"))).collect();
        apps.push(app("settings", Launch::Settings));
        apps.push(app("kindle", Launch::KindleMode));
        Ui::new(apps, Layout::new(1264, 1680, 3, 3))
    }

    fn centre(r: crate::Rect) -> Input {
        Input::Tap((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32)
    }

    #[test]
    fn launcher_settings_can_toggle_daily_checks_and_confirm_clearing_the_catalog() {
        let mut u = ui(0);
        u.screen = Screen::Settings;
        u.settings_scroll = u.layout.settings_scroll_max();
        let page = u.layout.settings_home_rows(u.settings_scroll).into_iter().find(|(page, _)| *page == Page::Store).unwrap().1;
        u.handle(centre(page));
        assert_eq!(u.screen, Screen::Page(Page::Store));
        let rows = &u.layout.page_sections(Page::Store)[0].rows;
        assert_eq!(u.handle(centre(rows[0].rect)), Action::Device(DeviceCmd::StoreToggleDailyCheck));
        u.handle(centre(rows[1].rect));
        assert!(u.store_clear_confirm);
        u.handle(Input::PageBack);
        assert_eq!(u.screen, Screen::Page(Page::Store));
        assert!(!u.store_clear_confirm);
        u.handle(centre(rows[1].rect));
        let [_, (_, clear)] = u.layout.confirm_buttons();
        assert_eq!(u.handle(centre(clear)), Action::Device(DeviceCmd::StoreClearCache));
        assert!(!u.store_clear_confirm);
    }

    #[test]
    fn an_installed_store_product_opens_only_with_a_launcher_entry() {
        let mut u = ui(0);
        let mut entry = crate::store::test_entry("example-app", "Example App", crate::store::Status::Installed("1.0.0".into()));
        entry.launchable = true;
        u.set_store(vec![entry]);
        u.open_store();
        u.store_open_entry(0);
        assert_eq!(u.store[0].action_label(), "Open");
        assert_eq!(u.handle(centre(u.layout.store_action(0))), Action::Device(DeviceCmd::StoreOpen(0)));
        u.store[0].launchable = false;
        assert_eq!(u.store[0].action_label(), "Installed");
        assert_eq!(u.handle(centre(u.layout.store_action(0))), Action::None);
    }

    #[test]
    fn store_row_actions_open_an_app_or_show_the_install_review() {
        use crate::store::{Status, StoreTab};
        let mut u = ui(0);
        let mut entry = crate::store::test_entry("example-app", "Example App", Status::Installed("1.0.0".into()));
        entry.launchable = true;
        u.set_store(vec![entry]);
        u.open_store();
        u.set_store_tab(StoreTab::Apps);
        let row = u.store_list_row(0);
        let action = centre(Layout::store_row_action(row));
        assert_eq!(u.handle(action), Action::Device(DeviceCmd::StoreOpen(0)));
        assert_eq!(u.screen, Screen::Store);
        u.handle(centre(row));
        assert_eq!(u.screen, Screen::StoreApp);
        u.open_store();
        u.set_store_tab(StoreTab::Apps);
        u.store[0].status = Status::Update { installed: "1.0.0".into(), latest: "2.0.0".into() };
        u.store[0].latest = Some("2.0.0".into());
        let row = u.store_list_row(0);
        u.handle(centre(Layout::store_row_action(row)));
        assert_eq!(u.screen, Screen::StoreApp);
        assert_eq!(u.store_sheet, Some(0));
        assert!(u.store_installing.is_none());
    }

    #[test]
    fn tapping_a_tile_launches_it_and_marks_it_pressed() {
        let mut u = ui(3);
        let c = u.layout.cell(1);
        match u.handle(centre(c)) {
            Action::Launch(a) => assert_eq!(a.id, "app1.sh"),
            other => panic!("{other:?}"),
        }
        assert_eq!(u.pressed, Some(u.layout.icon(1)), "only the touched icon is marked");
        assert_eq!(u.handle(centre(u.layout.cell(8))), Action::None, "empty slot");
    }

    #[test]
    fn paging_with_buttons_swipes_and_bounds() {
        let u = ui(7); // 9 icons with the built-ins: 1 page of 3 × 3; 10 would make 2
        assert_eq!(u.pages(), 1);
        let mut u2 = ui(8);
        assert_eq!(u2.pages(), 2);
        assert_eq!(u2.handle(Input::PageForward), Action::Redraw { full: false });
        let mut u = ui(10); // 12 icons: 2 pages
        assert_eq!(u.pages(), 2);
        assert_eq!(u.handle(Input::PageBack), Action::None);
        assert_eq!(u.handle(Input::PageForward), Action::Redraw { full: false });
        assert_eq!(u.page, 1);
        assert_eq!(u.handle(Input::PageForward), Action::None);
        assert_eq!(u.handle(Input::Swipe { dx: 300.0, dy: 20.0, start_y: 500.0 }), Action::Redraw { full: false });
        assert_eq!(u.page, 0);
        assert_eq!(u.handle(Input::Swipe { dx: -100.0, dy: 0.0, start_y: 500.0 }), Action::None, "too short");
        assert_eq!(u.page_apps().count(), 9);
    }

    #[test]
    fn hidden_apps_are_not_shown() {
        let mut a = script("x.sh");
        a.hidden = true;
        let u = Ui::new(vec![a, script("y.sh")], Layout::new(1264, 1680, 3, 3));
        assert_eq!(u.apps().len(), 1);
    }

    #[test]
    fn remove_takes_two_taps_on_an_installed_app_and_is_not_offered_otherwise() {
        use crate::store::{test_entry, Status};
        let mut u = ui(0);
        u.set_store(vec![test_entry("a", "Alpha", Status::Installed("1.0.0".into())), test_entry("b", "Beta", Status::NotInstalled)]);
        u.open_store();
        u.store_open = Some(0);
        u.screen = Screen::StoreApp;
        u.store_scroll = u.store_scroll_max();
        let last = u.store[0].page_rows().len() - 1;
        let row = u.layout.store_info_row(last, u.store_scroll, 0, 0);
        assert_eq!(u.handle(centre(row)), Action::Redraw { full: false }, "the first tap asks");
        assert_eq!(u.store_remove_armed.as_deref(), Some("a"));
        let [(_, cancel), (_, remove)] = u.layout.confirm_buttons();
        assert_eq!(u.handle(centre(cancel)), Action::Redraw { full: true });
        u.handle(centre(row));
        assert_eq!(u.handle(centre(remove)), Action::Device(DeviceCmd::StoreRemove(0)));
        // Not installed: no Remove row, so the last row is information.
        assert!(!u.store[1].can_remove());
        assert_ne!(u.store[1].page_rows().last().map(|r| r.0.as_str()), Some(crate::store::StoreEntry::REMOVE_LABEL));
    }

    #[test]
    fn the_page_buttons_move_through_the_store() {
        use crate::store::{test_entry, Status, StoreTab};
        let mut u = ui(0);
        let entries: Vec<_> = (0..30).map(|i| test_entry(&format!("a{i}"), &format!("App {i}"), Status::NotInstalled)).collect();
        u.set_store(entries);
        u.open_store();
        // Everything on Today fits, so a page button moves to the next tab.
        u.handle(Input::PageForward);
        assert_eq!(u.store_tab, StoreTab::Apps);
        // The Apps list is longer than its viewport, so the button scrolls it.
        u.handle(Input::PageForward);
        assert!(u.store_list_scroll > 0);
        // Back on the first tab, Today: the previous tab is not there.
        u.set_store_tab(StoreTab::Today);
        assert_eq!(u.handle(Input::PageBack), Action::None);
        // The product page scrolls a screen, and Back at the top returns to the store.
        u.store_open = Some(0);
        u.screen = Screen::StoreApp;
        u.handle(Input::PageForward);
        assert!(u.store_scroll > 0 || u.store_scroll_max() == 0);
        u.store_scroll = 0;
        u.handle(Input::PageBack);
        assert_eq!(u.screen, Screen::Store);
    }

    #[test]
    fn done_leaves_the_store_from_every_tab() {
        use crate::store::{test_entry, Status, StoreTab};
        let mut u = ui(0);
        u.set_store(vec![test_entry("a", "Alpha", Status::NotInstalled)]);
        for tab in StoreTab::ALL {
            u.open_store();
            u.store_tab = tab;
            u.handle(centre(u.layout.store_done()));
            assert_eq!(u.screen, Screen::Home, "Done leaves the store from {tab:?}");
        }
    }

    #[test]
    fn refresh_asks_the_device_once_while_it_runs() {
        use crate::store::{test_entry, Status};
        let mut u = ui(0);
        u.set_store(vec![test_entry("a", "Alpha", Status::NotInstalled)]);
        u.open_store();
        let button = centre(u.layout.store_refresh());
        assert_eq!(u.handle(button), Action::Device(DeviceCmd::StoreRefresh));
        assert!(u.store_busy);
        assert_eq!(u.handle(button), Action::None, "a second tap waits for the first refresh");
    }

    #[test]
    fn the_store_opens_on_today_and_its_tabs_switch() {
        use crate::store::{test_entry, Status, StoreTab};
        let mut u = ui(0);
        u.set_store(vec![
            test_entry("a", "Alpha", Status::NotInstalled),
            test_entry("b", "Beta", Status::NotInstalled),
            test_entry("c", "Gamma", Status::Installed("1.0.0".into())),
        ]);
        u.open_store();
        assert_eq!(u.screen, Screen::Store);
        assert_eq!(u.store_tab, StoreTab::Today);
        let apps = u.layout.store_tab(1);
        u.handle(centre(apps));
        assert_eq!(u.store_tab, StoreTab::Apps);
        let updates = u.layout.store_tab(2);
        u.handle(centre(updates));
        assert_eq!(u.store_tab, StoreTab::Updates);
        // Only the installed app is in Updates.
        assert_eq!(u.store_updates(), vec![2]);
        assert_eq!(u.handle(centre(u.layout.store_tab(3))), Action::Redraw { full: false });
        assert_eq!(u.store_tab, StoreTab::Search);
    }

    #[test]
    fn store_search_filters_each_catalog_field_and_accepts_keyboard_input() {
        use crate::store::{test_entry, Status, StoreTab};
        use primitive_keyboard::Key;

        let mut u = ui(0);
        let mut first = test_entry("a", "Alpha", Status::NotInstalled);
        first.subtitle = "Web explorer".into();
        first.developer = "Primitive Labs".into();
        first.categories = vec!["internet".into()];
        u.set_store(vec![first, test_entry("b", "Beta", Status::NotInstalled)]);
        for query in ["AlPhA", "  WEB  ", "Labs", "INTERNET"] {
            assert_eq!(u.store_search(query), vec![0]);
        }
        assert_eq!(u.store_search(""), vec![0, 1]);
        assert!(u.store_search("nonexistent").is_empty());
        u.open_store();
        u.handle(centre(u.layout.store_tab(3)));
        assert_eq!(u.store_tab, StoreTab::Search);
        u.handle(centre(u.layout.store_search_field()));
        for key in [Key::Char('b'), Key::Go] {
            let (_, rect) = u.layout.keyboard_keys(u.store_query.layer).into_iter().find(|(def, _)| def.key == key).unwrap();
            u.handle(centre(rect));
        }
        assert!(!u.store_search_keyboard);
        assert_eq!(u.store_query.string(), "b");
        assert_eq!(u.store_search(&u.store_query.string()), vec![0, 1]);
    }

    #[test]
    fn store_settings_and_batch_updates_confirm_before_a_change() {
        use crate::store::{test_entry, Status, StoreTab};

        let mut u = ui(0);
        u.set_store(vec![test_entry("app", "App", Status::Update { installed: "0.9.0".into(), latest: "1.0.0".into() })]);
        u.open_store();
        u.handle(centre(u.layout.store_settings_button()));
        assert!(u.store_settings_open);
        assert_eq!(u.handle(centre(u.layout.store_daily_check_button())), Action::Device(DeviceCmd::StoreToggleDailyCheck));
        u.handle(centre(u.layout.store_clear_button()));
        assert!(u.store_clear_confirm);
        let [(_, cancel), (_, clear)] = u.layout.confirm_buttons();
        assert_eq!(u.handle(centre(cancel)), Action::Redraw { full: true });
        u.handle(centre(u.layout.store_clear_button()));
        assert_eq!(u.handle(centre(clear)), Action::Device(DeviceCmd::StoreClearCache));
        u.handle(centre(clear));
        assert!(!u.store_settings_open);
        u.handle(centre(u.layout.store_tab(2)));
        assert_eq!(u.store_tab, StoreTab::Updates);
        u.handle(centre(u.layout.store_toolbar_button()));
        assert!(u.store_batch_confirm);
        let [cancel, update] = u.layout.store_sheet_buttons();
        assert_eq!(u.handle(centre(cancel)), Action::Redraw { full: true });
        u.handle(centre(u.layout.store_toolbar_button()));
        assert_eq!(u.handle(centre(update)), Action::Device(DeviceCmd::StoreUpdateAll));
        assert!(u.store_batch_running);
        assert_eq!(u.handle(centre(cancel)), Action::None);
    }

    #[test]
    fn store_collections_and_categories_filter_the_apps_tab() {
        use crate::store::{test_entry, Status, StoreTab};
        use kl_store::catalog::Collection;

        let mut u = ui(0);
        let alpha = test_entry("alpha", "Alpha", Status::NotInstalled);
        let mut beta = test_entry("beta", "Beta", Status::NotInstalled);
        beta.categories = vec!["reading".to_string()];
        u.set_store(vec![alpha, beta]);
        u.store_collections = vec![Collection {
            id: "start-here".to_string(), title: "Start here".to_string(),
            description: String::new(), apps: vec!["beta".to_string(), "missing".to_string()],
        }];
        u.open_store();
        u.handle(centre(u.layout.store_collection_card(0)));
        assert_eq!(u.store_tab, StoreTab::Apps);
        assert_eq!(u.store_apps(), vec![1]);
        u.handle(centre(u.layout.store_toolbar_button()));
        assert_eq!(u.store_apps(), vec![0, 1]);
        u.handle(centre(u.layout.store_toolbar_button()));
        assert!(u.store_category_picker);
        u.handle(centre(u.layout.store_category_cell(1)));
        assert_eq!(u.store_category.as_deref(), Some("productivity"));
        assert_eq!(u.store_apps(), vec![0]);
    }

    #[test]
    fn a_required_app_review_pages_the_list_and_can_be_cancelled() {
        use crate::store::{test_entry, Status};

        let mut u = ui(0);
        u.set_store(vec![test_entry("target", "Target", Status::NotInstalled)]);
        u.open_store();
        u.handle(centre(u.layout.store_feature()));
        u.handle(centre(u.layout.store_action(0)));
        u.store_requirements = Some(StoreRequirementReview {
            apps: (0..4).map(|index| test_entry(&format!("required-{index}"), "Required app", Status::NotInstalled)).collect(),
            page: 0,
        });
        let [_, next] = u.layout.store_requirements_nav();
        assert_eq!(u.handle(centre(next)), Action::Redraw { full: false });
        let review = u.store_requirements.as_ref().unwrap();
        assert_eq!(review.page, 1);
        assert_eq!(review.visible_apps()[0].id, "required-3");
        let [cancel, _] = u.layout.store_sheet_buttons();
        assert_eq!(u.handle(centre(cancel)), Action::Redraw { full: true });
        assert!(u.store_requirements.is_none());
        assert!(u.store_installing.is_none());
    }

    #[test]
    fn today_opens_the_featured_app_and_its_rows_and_see_all_goes_to_apps() {
        use crate::store::{test_entry, Status, StoreTab};
        let mut u = ui(0);
        u.set_store(vec![
            test_entry("a", "Alpha", Status::NotInstalled),
            test_entry("b", "Beta", Status::NotInstalled),
            test_entry("c", "Gamma", Status::NotInstalled),
        ]);
        u.open_store();
        assert_eq!(u.handle(centre(u.layout.store_feature())), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::StoreApp);
        assert_eq!(u.store_open, Some(0));
        u.handle(Input::Tap(0.0, 0.0)); // back to the store: the header is above the content
        u.screen = Screen::Store;
        let row = u.layout.store_today_row(1);
        u.handle(centre(row));
        assert_eq!(u.store_open, Some(2), "the second row under the featured card is the third entry");
        u.screen = Screen::Store;
        u.store_tab = StoreTab::Today;
        u.handle(centre(u.layout.store_see_all()));
        assert_eq!(u.store_tab, StoreTab::Apps);
    }

    #[test]
    fn the_apps_list_scrolls_opens_the_visible_entry_and_preserves_position_on_back() {
        use crate::store::{test_entry, Status, StoreTab};
        let mut u = ui(0);
        let entries: Vec<_> = (0..30).map(|i| test_entry(&format!("app{i}"), &format!("App {i:02}"), Status::NotInstalled)).collect();
        u.set_store(entries);
        u.open_store();
        u.handle(centre(u.layout.store_tab(1)));
        assert_eq!(u.store_tab, StoreTab::Apps);
        let viewport = u.store_list_viewport();
        assert_eq!(viewport.bottom(), u.layout.store_tabs().y - 12);
        let swipe = |dy, start_y| Input::Swipe { dx: 0.0, dy, start_y };
        assert_eq!(u.handle(swipe(-380.0, 120.0)), Action::None);
        assert_eq!(u.handle(swipe(-380.0, u.layout.store_tabs().y as f32 + 40.0)), Action::None);
        assert_eq!(u.handle(swipe(-380.0, 900.0)), Action::Redraw { full: false });
        assert_eq!(u.store_list_scroll, 380);
        let (row, index) = u.store_visible_entries().into_iter()
            .find(|(row, _)| row.y >= viewport.y).unwrap();
        assert!(index > 0);
        assert_eq!(u.handle(centre(row)), Action::Redraw { full: false });
        assert_eq!(u.store_open, Some(index));
        u.handle(centre(u.layout.store_back()));
        assert_eq!(u.screen, Screen::Store);
        assert_eq!(u.store_list_scroll, 380);
        u.handle(swipe(-20_000.0, 900.0));
        assert_eq!(u.store_list_scroll, u.store_list_scroll_max());
        assert_eq!(u.store_visible_entries().last().unwrap().1, 29);

        u.notice = Some("Catalog updated".into());
        let tap = Input::Tap(80.0, (u.store_list_viewport().bottom() + 20) as f32);
        assert_eq!(u.handle(tap), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Store);
        assert!(u.notice.is_none());

        u.handle(swipe(20_000.0, 900.0));
        assert_eq!(u.store_list_scroll, 0);
        u.handle(centre(u.layout.store_tab(3)));
        assert_eq!(u.store_tab, StoreTab::Search);
        u.handle(centre(u.layout.store_search_field()));
        assert!(u.store_search_keyboard);
        assert!(u.store_list_viewport().bottom() < u.layout.keyboard_area().y);
        assert_eq!(u.handle(swipe(-400.0, (u.layout.keyboard_area().y + 60) as f32)), Action::None);
        u.handle(swipe(-400.0, 600.0));
        assert_eq!(u.store_list_scroll, 400);
        u.handle(Input::PageBack);
        assert!(!u.store_search_keyboard);
        assert_eq!(u.store_list_scroll, 400);
    }

    #[test]
    fn today_scrolls_to_all_collections_and_opens_the_one_tapped() {
        use crate::store::{test_entry, Status};
        use kl_store::catalog::Collection;
        let mut u = ui(0);
        u.set_store(vec![test_entry("a", "Alpha", Status::NotInstalled)]);
        u.store_collections = (0..11).map(|i| Collection {
            id: format!("collection-{i}"), title: format!("Collection {i}"),
            description: String::new(), apps: vec!["a".into()],
        }).collect();
        u.open_store();
        assert!(u.store_today_collection_card(10).y > u.store_list_viewport().bottom());
        u.handle(Input::Swipe { dx: 0.0, dy: -2400.0, start_y: 900.0 });
        let last = u.store_today_collection_card(10);
        assert_eq!(last.bottom(), u.store_list_viewport().bottom());
        u.handle(centre(last));
        assert_eq!(u.store_tab, StoreTab::Apps);
        assert_eq!(u.store_collection.as_deref(), Some("collection-10"));
        assert_eq!(u.store_list_scroll, 0);
    }

    #[test]
    fn the_screen_timeout_tile_and_the_battery_row_both_cycle_the_setting() {
        let mut u = ui(0);
        u.screen = Screen::QuickSettings;
        let (tile_kind, tile) = u.layout.quick_buttons()[4];
        assert_eq!(tile_kind, QuickTile::ScreenTimeout);
        assert_eq!(u.handle(centre(u.layout.quick_button_circle(tile))), Action::Device(DeviceCmd::CycleScreenTimeout));
        u.screen = Screen::Page(Page::Sleep);
        let row = u.layout.page_sections(Page::Sleep).into_iter().flat_map(|s| s.rows).find(|r| r.row == crate::layout::Row::ScreenTimeout).unwrap();
        assert_eq!(u.handle(centre(row.rect)), Action::Device(DeviceCmd::CycleScreenTimeout));
    }

    #[test]
    fn holding_both_page_buttons_asks_before_restarting_the_launcher() {
        let mut u = ui(0);
        assert_eq!(u.handle(Input::RestartHold), Action::Redraw { full: true });
        assert_eq!(u.screen, Screen::ConfirmRestart);
    }

    #[test]
    fn the_product_page_scrolls_and_its_action_installs_or_shows_the_link() {
        use crate::store::{test_entry, Kind, Status};
        let mut u = ui(0);
        let mut link = test_entry("k", "Kay", Status::NotInstalled);
        link.kind = Kind::Link;
        u.set_store(vec![test_entry("a", "Alpha", Status::NotInstalled), test_entry("i", "Ivy", Status::Installed("1.0.0".into())), link]);
        u.open_store();
        u.handle(centre(u.layout.store_feature()));
        assert!(u.store_scroll_max() > 0, "the page is longer than the screen");
        u.handle(Input::Swipe { dx: 0.0, dy: -400.0, start_y: 1200.0 });
        assert_eq!(u.store_scroll, 400.min(u.store_scroll_max()));
        u.handle(Input::Swipe { dx: 0.0, dy: 5000.0, start_y: 600.0 });
        assert_eq!(u.store_scroll, 0, "scrolling back up stops at the top");
        // Get opens a sheet. Cancel is reversible, and Install starts the operation.
        let action = u.layout.store_action(0);
        let [cancel, install] = u.layout.store_sheet_buttons();
        assert_eq!(u.handle(centre(action)), Action::Redraw { full: false });
        assert_eq!(u.store_sheet, Some(0));
        assert_eq!(u.handle(centre(cancel)), Action::Redraw { full: true });
        assert_eq!(u.store_sheet, None);
        u.handle(centre(action));
        assert_eq!(u.handle(centre(install)), Action::Device(DeviceCmd::StoreInstall(0)));
        u.store_sheet = None;
        u.store_installing = None;
        // Back returns to the store.
        u.handle(centre(u.layout.store_back()));
        assert_eq!(u.screen, Screen::Store);
        // Installed does nothing.
        u.store_open = Some(1);
        u.screen = Screen::StoreApp;
        assert_eq!(u.handle(centre(action)), Action::None);
        // A link says where the app is.
        u.store_open = Some(2);
        u.handle(centre(action));
        assert!(u.notice.as_deref().unwrap_or("").contains("own website"));
    }

    #[test]
    fn builtins_open_screens_and_confirm_leads_to_kindle_mode() {
        let mut u = ui(2); // settings at slot 2, kindle at slot 3
        assert_eq!(u.handle(centre(u.layout.cell(2))), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Settings);
        let done = u.layout.done_button();
        u.handle(centre(done));
        assert_eq!(u.screen, Screen::Home);
        u.handle(centre(u.layout.cell(3)));
        assert_eq!(u.screen, Screen::ConfirmKindle);
        let [(_, cancel), (_, open)] = u.layout.confirm_buttons();
        u.handle(centre(cancel));
        assert_eq!(u.screen, Screen::Home);
        u.handle(centre(u.layout.cell(3)));
        assert_eq!(u.handle(centre(open)), Action::KindleMode);
    }

    #[test]
    fn device_name_row_opens_the_name_page_and_saves_a_name() {
        use crate::layout::Row;
        let mut u = ui(1);
        u.screen = Screen::Page(Page::Device);
        let rows: Vec<_> = u.layout.page_sections(Page::Device).into_iter().flat_map(|s| s.rows).collect();
        let row = rows.iter().find(|x| x.row == Row::DeviceName).expect("the Device name row is on the Device page");
        let button = row.buttons.first().map(|b| b.1).expect("the Device name row has a tap target");
        u.handle(centre(button));
        assert_eq!(u.screen, Screen::DeviceName, "tapping the row opens the name page");
        // Type "Bo" and Save on the keyboard: the name goes to the driver.
        let key_tap = |u: &Ui, key: primitive_keyboard::Key| {
            let keys = u.layout.keyboard_keys_with(u.device_name_entry.layer, "Save");
            let (_, r) = keys.iter().find(|(k, _)| k.key == key).expect("key on the keyboard");
            Input::Tap((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32)
        };
        let tap = key_tap(&u, primitive_keyboard::Key::Char('b'));
        u.handle(tap);
        let tap = key_tap(&u, primitive_keyboard::Key::Char('o'));
        u.handle(tap);
        let save = key_tap(&u, primitive_keyboard::Key::Go);
        assert_eq!(u.handle(save), Action::SetDeviceName("bo".into()), "Save sends the typed name");
    }

    #[test]
    fn connection_page_gives_ssh_and_dns_commands_and_a_password_screen() {
        use primitive_keyboard::Key;
        use crate::layout::Row;
        let mut u = ui(1);
        u.screen = Screen::Page(Page::Connection);
        let rows: Vec<_> = u.layout.page_sections(Page::Connection).into_iter().flat_map(|s| s.rows).collect();
        let button = |rw: Row| rows.iter().find(|x| x.row == rw).unwrap().buttons.first().map(|b| b.1);
        assert_eq!(u.handle(centre(button(Row::Ssh).unwrap())), Action::Device(DeviceCmd::ToggleSsh));
        assert_eq!(u.handle(centre(button(Row::PublicDns).unwrap())), Action::Device(DeviceCmd::ToggleDns));
        assert!(button(Row::SshAddress).is_none(), "the address is only shown");
        // Set with three letters asks for at least six; six letters go to the driver.
        u.handle(centre(button(Row::SshPassword).unwrap()));
        assert_eq!(u.screen, Screen::SshPassword);
        let key = |u: &Ui, k: Key| u.layout.keyboard_keys(u.ssh_password.layer).into_iter().find(|(d, _)| d.key == k).unwrap().1;
        for c in ['a', 'b', 'c'] {
            u.handle(centre(key(&u, Key::Char(c))));
        }
        u.handle(centre(key(&u, Key::Go)));
        assert!(u.notice.as_deref().unwrap().contains("6 to 64"));
        assert_eq!(u.screen, Screen::SshPassword);
        for c in ['d', 'e', 'f'] {
            u.handle(centre(key(&u, Key::Char(c))));
        }
        assert_eq!(u.handle(centre(key(&u, Key::Go))), Action::SetSshPassword("abcdef".into()));
        assert_eq!(u.screen, Screen::Page(Page::Connection), "the password is set from the SSH & DNS page");
    }

    #[test]
    fn the_device_page_restarts_the_launcher_from_its_own_row() {
        use crate::layout::Row;
        let mut u = ui(1);
        u.screen = Screen::Page(Page::Device);
        let rows: Vec<_> = u.layout.page_sections(Page::Device).into_iter().flat_map(|s| s.rows).collect();
        let restart = rows.iter().find(|r| r.row == Row::Restart).expect("the restart row is on the Device page");
        assert_eq!(u.handle(centre(restart.buttons[0].1)), Action::Device(DeviceCmd::RestartLauncher));
    }

    #[test]
    fn the_usb_state_comes_from_the_cable_and_the_drive() {
        assert_eq!(Usb::from_device(false, true), Usb::Unplugged);
        assert_eq!(Usb::from_device(false, false), Usb::Unplugged, "no cable, no drive");
        assert_eq!(Usb::from_device(true, true), Usb::Charging);
        assert_eq!(Usb::from_device(true, false), Usb::Drive, "the drive is on the computer: /mnt/us is away");
    }

    #[test]
    fn the_drive_screen_takes_no_taps_until_it_ends() {
        let mut u = ui(1);
        u.screen = Screen::Settings;
        let done = centre(u.layout.done_button());
        u.usb = Usb::Drive;
        assert_eq!(u.handle(done), Action::None, "no taps while the drive is shown");
        u.usb = Usb::Charging;
        assert_eq!(u.handle(done), Action::Redraw { full: false }, "taps work again");
        assert_eq!(u.screen, Screen::Home);
    }

    #[test]
    fn a_key_tap_is_marked_pressed_until_its_action_runs() {
        use primitive_keyboard::Key;
        let mut u = ui(1);
        u.screen = Screen::NetworkName;
        let (_, r) = u.layout.keyboard_keys_with(u.network_name.layer, "Next").into_iter().find(|(d, _)| d.key == Key::Char('a')).unwrap();
        let tap = Input::Tap(r.x as f32 + 5.0, r.y as f32 + 5.0);
        assert!(u.press_feedback(tap), "a key under the finger is marked");
        assert_eq!(u.pressed_key, Some(r));
        u.handle(tap);
        assert_eq!(u.pressed_key, None, "the action clears the mark");
        assert_eq!(u.network_name.string(), "a");
        // A tap outside the keyboard marks nothing.
        u.screen = Screen::Settings;
        assert!(!u.press_feedback(tap));
    }

    #[test]
    fn the_power_tile_opens_a_menu_with_restart_and_power_off() {
        let mut u = ui(1);
        u.screen = Screen::QuickSettings;
        let (_, tile) = u.layout.quick_buttons()[7];
        assert_eq!(u.handle(centre(u.layout.quick_button_circle(tile))), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::PowerMenu, "the quick panel closes and the menu opens");
        let [(_, cancel), (_, kindle), (_, restart), (_, off)] = u.layout.power_buttons();
        assert_eq!(u.handle(centre(kindle)), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::ConfirmKindle, "the Kindle UI is confirmed first");
        let [(_, _), (_, open)] = u.layout.confirm_buttons();
        assert_eq!(u.handle(centre(open)), Action::KindleMode);
        u.screen = Screen::PowerMenu;
        assert_eq!(u.handle(centre(restart)), Action::Device(DeviceCmd::Restart));
        assert_eq!(u.handle(centre(off)), Action::Device(DeviceCmd::PowerOff));
        assert_eq!(u.handle(centre(cancel)), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Home, "Cancel goes home");
    }

    #[test]
    fn ssh_passwords_have_safe_characters_and_length() {
        assert!(valid_ssh_password("abc123"));
        assert!(valid_ssh_password("Kindle-2026_x.@#%+=!"));
        assert!(!valid_ssh_password("short"));
        assert!(!valid_ssh_password("has space1"));
        assert!(!valid_ssh_password("semi;colon1"));
        assert!(!valid_ssh_password(&"a".repeat(65)));
    }

    #[test]
    fn status_strip_opens_settings_and_buttons_give_device_commands() {
        let mut u = ui(1);
        u.handle(Input::Swipe { dx: 0.0, dy: 400.0, start_y: 40.0 });
        assert_eq!(u.screen, Screen::QuickSettings, "the status bar opens quick settings");
        let all = u.layout.quick_button_circle(u.layout.quick_buttons()[6].1);
        u.handle(centre(all));
        assert_eq!(u.screen, Screen::Settings);
        // The Settings list opens the Display page, whose rows give the device commands.
        let (display, r) = *u.layout.settings_home_rows(0).iter().find(|(p, _)| *p == Page::Display).unwrap();
        assert_eq!(display, Page::Display);
        u.handle(centre(r));
        assert_eq!(u.screen, Screen::Page(Page::Display));
        let rows: Vec<_> = u.layout.page_sections(Page::Display).into_iter().flat_map(|s| s.rows).collect();
        let row = |rw: crate::layout::Row| rows.iter().find(|x| x.row == rw).unwrap().buttons[0].1;
        assert_eq!(u.handle(centre(rows[0].buttons[1].1)), Action::Device(DeviceCmd::Light(1)));
        assert_eq!(u.handle(centre(row(crate::layout::Row::Rotation))), Action::Device(DeviceCmd::ToggleRotation));
        assert_eq!(u.handle(centre(row(crate::layout::Row::DarkMode))), Action::Device(DeviceCmd::ToggleDarkMode));
        // Wi-Fi and Animation are on their own pages.
        u.handle(Input::PageBack);
        assert_eq!(u.screen, Screen::Settings);
        u.handle(Input::PageBack);
        assert_eq!(u.screen, Screen::Home);
        u.screen = Screen::Page(Page::Wifi);
        let wifi = u.layout.page_sections(Page::Wifi)[0].rows[0].buttons[0].1;
        assert_eq!(u.handle(centre(wifi)), Action::Device(DeviceCmd::ToggleWifi));
        u.screen = Screen::Page(Page::Display);
        let anim = u.layout.page_sections(Page::Display).into_iter().flat_map(|s| s.rows).find(|r| r.row == crate::layout::Row::Animation).unwrap().buttons[0].1;
        assert_eq!(u.handle(centre(anim)), Action::Device(DeviceCmd::ToggleAnimation));
    }

    #[test]
    fn display_and_quick_settings_select_the_same_levels_and_check_support() {
        let mut u = ui(1);
        let levels = u.layout.page_sections(Page::Display).remove(0).rows;
        let light = crate::layout::setting_level_bar(levels[0].rect);
        let warmth = crate::layout::setting_level_bar(levels[1].rect);
        u.screen = Screen::Page(Page::Display);
        assert_eq!(u.handle(centre(light)), Action::Device(DeviceCmd::SetLight(12)));
        assert_eq!(u.handle(centre(warmth)), Action::Device(DeviceCmd::SetWarmth(12)));
        assert_eq!(u.handle(centre(levels[1].buttons[0].1)), Action::Device(DeviceCmd::Warmth(-1)));
        u.display_support.warmth = false;
        assert_eq!(u.handle(centre(warmth)), Action::None);
        assert_eq!(u.handle(centre(levels[1].buttons[1].1)), Action::None);

        u.screen = Screen::QuickSettings;
        assert_eq!(u.handle(centre(u.layout.quick_bar(0))), Action::Device(DeviceCmd::SetLight(12)));
        assert_eq!(u.handle(centre(u.layout.quick_bar(1))), Action::None);
        assert_eq!(u.handle(centre(u.layout.quick_minus(1))), Action::None);
        u.display_support.auto_rotation = false;
        let auto_rotate = u.layout.quick_buttons().into_iter().find(|(tile, _)| *tile == QuickTile::AutoRotate).unwrap().1;
        assert_eq!(u.handle(centre(auto_rotate)), Action::None);
    }

    #[test]
    fn accessibility_rows_use_the_saved_appearance_commands() {
        let mut u = ui(1);
        u.screen = Screen::Page(Page::Accessibility);
        let sections = u.layout.page_sections(Page::Accessibility);
        assert_eq!(u.handle(centre(sections[0].rows[0].rect)), Action::Device(DeviceCmd::ToggleLargeText));
        assert_eq!(u.handle(centre(sections[0].rows[1].rect)), Action::Device(DeviceCmd::ToggleHighContrast));
        assert_eq!(u.handle(centre(sections[0].rows[2].rect)), Action::None);
        assert_eq!(u.handle(centre(sections[1].rows[0].rect)), Action::Device(DeviceCmd::ToggleAnimation));
    }

    #[test]
    fn sleep_ignores_input_and_wake_restores_the_screen() {
        let mut u = ui(1);
        u.screen = Screen::Settings;
        assert_eq!(u.handle(Input::Sleep), Action::Redraw { full: true });
        assert_eq!(u.handle(Input::PageForward), Action::None);
        assert_eq!(u.handle(Input::Sleep), Action::Redraw { full: true });
        assert_eq!(u.handle(Input::Wake), Action::Redraw { full: true });
        assert_eq!(u.screen, Screen::Settings);
        assert_eq!(u.handle(Input::Wake), Action::None, "a second wake event does nothing");
    }

    #[test]
    fn a_low_battery_warning_is_reported_once_and_persists_across_input() {
        let mut u = ui(1);
        assert!(!u.set_battery(Some(90), false));
        assert_eq!(u.battery_message(), None);
        assert!(u.set_battery(Some(9), false));
        assert_eq!(u.battery_message().as_deref(), Some("Battery low 9%. Connect the charger soon."));
        assert!(!u.set_battery(Some(8), false)); // still low: no change to redraw for
        u.handle(Input::Tap(5.0, 1400.0));
        assert!(u.battery_message().is_some()); // not a notice: the next input does not clear it
        assert!(u.set_battery(Some(9), true)); // charger connected
        assert_eq!(u.battery_message(), None);
    }

    #[test]
    fn a_long_press_hides_an_app_and_show_all_brings_it_back() {
        let mut u = ui(3);
        let cell = u.layout.cell(0);
        let (x, y) = (cell.x as f32 + 5.0, cell.y as f32 + 5.0);
        let before = u.apps().len();
        let name = u.apps()[0].name.clone();
        let id = u.apps()[0].id.clone();
        assert_eq!(u.handle(Input::LongTap(x, y)), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::AppMenu);
        let hide = menu_row(&u, Button::MenuHide);
        let action = u.handle(Input::Tap(hide.x as f32 + 10.0, hide.y as f32 + 10.0));
        assert_eq!(action, Action::SetHidden(vec![(id, true)]));
        assert_eq!(u.screen, Screen::Home);
        assert_eq!(u.apps().len(), before - 1);
        assert_eq!(u.hidden_count(), 1);
        assert!(u.notice.as_deref().unwrap().contains(&name));
        u.screen = Screen::Settings;
        let Action::SetHidden(changes) = u.show_all() else { panic!() };
        assert_eq!(changes.len(), 1);
        assert!(!changes[0].1);
        assert_eq!(u.apps().len(), before);
        assert_eq!(u.hidden_count(), 0);
        assert_eq!(u.show_all(), Action::None);
    }

    fn tap(u: &mut Ui, r: crate::layout::Rect) -> Action {
        u.handle(Input::Tap(r.x as f32 + 10.0, r.y as f32 + 10.0))
    }

    fn open_menu(u: &mut Ui, slot: usize) {
        let c = u.layout.cell(slot);
        u.handle(Input::LongTap(c.x as f32 + 5.0, c.y as f32 + 5.0));
        assert_eq!(u.screen, Screen::AppMenu);
    }

    /// Tap the app menu's `button`.
    fn press(u: &mut Ui, button: Button) -> Action {
        let row = menu_row(u, button);
        tap(u, row)
    }

    /// The app menu's row for `button`, beside the edited app's icon.
    fn menu_row(u: &Ui, button: Button) -> Rect {
        let actions = u.menu_actions();
        let i = actions.iter().position(|&b| b == button).expect("action in the menu");
        u.layout.menu_row(u.menu_anchor(), actions.len(), i)
    }

    #[test]
    fn moving_an_app_saves_the_order_of_all_movable_apps() {
        let mut u = ui(3);
        let first = u.apps()[0].id.clone();
        let second = u.apps()[1].id.clone();
        open_menu(&mut u, 0);
        let later = menu_row(&u, Button::MenuLater);
        let Action::SetOrder(order) = tap(&mut u, later) else { panic!("no order saved") };
        assert_eq!(order[0], (second.clone(), 1));
        assert_eq!(order[1], (first.clone(), 2));
        assert_eq!(u.menu, 1);
        assert!(order.iter().all(|(id, _)| id != "settings" && id != "kindle"));
        // The menu follows the icon to its new place.
        assert_eq!(u.menu_anchor(), u.layout.icon(1));
        // A tap outside the box closes it.
        assert_eq!(u.handle(Input::Tap(5.0, 5.0)), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Home);
        assert_eq!(u.apps()[1].id, first);
    }

    #[test]
    fn an_app_cannot_move_before_the_first_place_or_past_the_built_ins() {
        let mut u = ui(1);
        open_menu(&mut u, 0);
        let (earlier, later) = (menu_row(&u, Button::MenuEarlier), menu_row(&u, Button::MenuLater));
        assert_eq!(tap(&mut u, earlier), Action::Redraw { full: false });
        assert_eq!(u.notice.as_deref(), Some("Already first"));
        assert_eq!(tap(&mut u, later), Action::Redraw { full: false });
        assert_eq!(u.notice.as_deref(), Some("Already last"));
    }

    #[test]
    fn change_icon_cycles_through_the_files_then_back_to_the_own_icon() {
        let mut u = ui(1);
        u.set_icon_choices(vec!["/i/a.png".into(), "/i/b.png".into()]);
        let id = u.apps()[0].id.clone();
        open_menu(&mut u, 0);
        let icon = menu_row(&u, Button::MenuIcon);
        assert_eq!(tap(&mut u, icon), Action::SetIcon { id: id.clone(), icon: Some("/i/a.png".into()) });
        assert_eq!(tap(&mut u, icon), Action::SetIcon { id: id.clone(), icon: Some("/i/b.png".into()) });
        assert_eq!(tap(&mut u, icon), Action::SetIcon { id: id.clone(), icon: None });
        assert_eq!(u.apps()[0].icon, IconSource::None);
    }

    #[test]
    fn change_icon_skips_the_dark_forms_of_the_icon_files() {
        let mut u = ui(1);
        u.set_icon_choices(vec!["/i/a-dark.png".into(), "/i/a.png".into(), "/i/b-dark.png".into()]);
        let id = u.apps()[0].id.clone();
        open_menu(&mut u, 0);
        let icon = menu_row(&u, Button::MenuIcon);
        assert_eq!(tap(&mut u, icon), Action::SetIcon { id: id.clone(), icon: Some("/i/a.png".into()) });
        assert_eq!(tap(&mut u, icon), Action::SetIcon { id: id.clone(), icon: Some("/i/b-dark.png".into()) }, "no light form: a choice");
        assert_eq!(tap(&mut u, icon), Action::SetIcon { id, icon: None });
    }

    #[test]
    fn change_icon_without_icon_files_explains_where_to_put_them() {
        let mut u = ui(1);
        open_menu(&mut u, 0);
        let icon = menu_row(&u, Button::MenuIcon);
        assert_eq!(tap(&mut u, icon), Action::Redraw { full: false });
        assert!(u.notice.as_deref().unwrap().contains("/mnt/us/launcher/icons"));
    }

    fn menu_items() -> Vec<MenuItem> {
        let leaf = |name: &str| MenuItem { name: name.into(), command: Some(format!("sh '/e/{name}.sh'")), dir: "/e".into(), priority: 0, background: name == "Sync", children: vec![] };
        vec![
            leaf("Alpha"),
            MenuItem { name: "Tools".into(), command: None, dir: "/e".into(), priority: 0, background: false, children: vec![leaf("Reset"), leaf("Sync")] },
        ]
    }

    #[test]
    fn extensions_add_one_tile_with_the_built_ins() {
        let mut u = ui(2);
        u.set_extensions(menu_items());
        u.set_extensions(menu_items());
        let ids: Vec<_> = u.apps().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids.iter().filter(|i| **i == "extensions").count(), 1);
        let at = ids.iter().position(|i| *i == "extensions").unwrap();
        assert!(at >= ids.len() - 3, "the Extensions tile sits with the built-ins at the end: {ids:?}");
        u.set_extensions(vec![]);
        assert!(!u.apps().iter().any(|a| a.id == "extensions"));
    }

    #[test]
    fn the_extensions_list_opens_submenus_runs_items_and_goes_back() {
        let mut u = ui(1);
        u.set_extensions(menu_items());
        assert_eq!(u.open_extensions(), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Extensions);
        assert_eq!(u.ext_title(), "Extensions");
        // Open "Tools" (second row).
        let tools = u.layout.list_row(1);
        assert_eq!(tap(&mut u, tools), Action::Redraw { full: false });
        assert_eq!(u.ext_title(), "Tools");
        assert_eq!(u.ext_depth(), 1);
        // "Reset" leaves the launcher; "Sync" (exitmenu: false) runs in the background.
        let reset = u.layout.list_row(0);
        let Action::Launch(app) = tap(&mut u, reset) else { panic!("no launch") };
        assert_eq!(app.launch, Launch::Shell { command: "sh '/e/Reset.sh'".into() });
        let sync = u.layout.list_row(1);
        let Action::RunBackground(app) = tap(&mut u, sync) else { panic!("not in the background") };
        assert_eq!(app.name, "Sync");
        assert_eq!(u.notice.as_deref(), Some("Running Sync…"));
        assert_eq!(u.screen, Screen::Extensions);
        // Back goes up a level, then to the grid.
        let back = u.layout.done_button();
        assert_eq!(tap(&mut u, back), Action::Redraw { full: false });
        assert_eq!(u.ext_depth(), 0);
        tap(&mut u, back);
        assert_eq!(u.screen, Screen::Home);
    }

    #[test]
    fn the_kindle_ui_and_settings_cannot_be_hidden() {
        let mut u = Ui::new(
            vec![app("kindle", Launch::KindleMode), app("a.sh", Launch::Script { path: "/d/a.sh".into(), hooks: false })],
            Layout::new(1264, 1680, 3, 3),
        );
        let kindle = u.layout.cell(0);
        assert_eq!(u.handle(Input::LongTap(kindle.x as f32 + 5.0, kindle.y as f32 + 5.0)), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Home);
        assert_eq!(u.notice.as_deref(), Some("kindle stays in place"));
        assert_eq!(u.apps().len(), 2);
        assert_eq!(u.hidden_count(), 0);
    }

    #[test]
    fn moving_across_a_page_turns_the_page_with_the_app() {
        let mut u = ui(12); // 3 × 3: app8 is the last on page 1
        open_menu(&mut u, 8);
        assert!(matches!(press(&mut u, Button::MenuLater), Action::SetOrder(_)));
        assert_eq!(u.page, 1);
        assert_eq!(u.menu_anchor(), u.layout.icon(0));
    }

    #[test]
    fn the_wifi_page_scans_lists_the_networks_and_scrolls() {
        use crate::wifi::{Network, WifiInfo};
        let net = |name: &str, level: u32| Network { name: name.into(), level, max: 5, secured: true, known: false };
        let mut u = ui(1);
        u.screen = Screen::Page(Page::Wifi);
        let scan = u.layout.wifi_area().scan;
        assert_eq!(u.handle(centre(scan)), Action::ScanWifi);
        assert!(u.wifi_scanning);
        let current = Network { known: true, ..net("Current", 4) };
        let mut networks: Vec<Network> = (0..30).map(|i| net(&format!("Net {i:02}"), 3)).collect();
        networks.push(current.clone());
        u.set_wifi(WifiInfo { current: Some(current), networks });
        assert!(!u.wifi_scanning);
        assert_eq!(u.wifi_networks().len(), 30, "the current network is not repeated in the list");
        let visible = u.layout.wifi_area().visible;
        assert!((6..30).contains(&visible), "a long list needs scrolling: {visible} rows fit");
        // A swipe moves about as many rows as it covers; the list stops at its end.
        u.handle(Input::Swipe { dx: 0.0, dy: -400.0, start_y: 900.0 });
        assert_eq!(u.wifi_scroll, 3, "400 px covers three rows");
        for _ in 0..40 {
            u.handle(Input::Swipe { dx: 0.0, dy: -400.0, start_y: 900.0 });
        }
        assert_eq!(u.wifi_scroll, 30 - visible, "scrolls to the last network and stops");
        u.handle(Input::Swipe { dx: 0.0, dy: 400.0, start_y: 900.0 });
        assert_eq!(u.wifi_scroll, 30 - visible - 3);
        // Scrolled back to the top, the first network is row 0 of the list.
        for _ in 0..40 {
            u.handle(Input::Swipe { dx: 0.0, dy: 400.0, start_y: 900.0 });
        }
        assert_eq!(u.wifi_scroll, 0);
        let row = u.layout.network_row(0);
        u.handle(centre(row));
        assert_eq!(u.screen, Screen::NetworkDetail, "a network opens its screen");
        assert_eq!(u.wifi_target.as_ref().map(|n| n.name.as_str()), Some("Net 00"));
    }

    #[test]
    fn the_network_screen_follows_a_forget() {
        use crate::wifi::{Network, WifiInfo};
        let saved = Network { name: "Home".into(), level: 4, max: 5, secured: true, known: true };
        let mut u = ui(1);
        u.set_wifi(WifiInfo { current: None, networks: vec![saved.clone()] });
        u.screen = Screen::Page(Page::Wifi);
        u.handle(centre(u.layout.network_row(0)));
        // The forget finishes: the scan now says the network is not saved.
        let forgotten = Network { known: false, ..saved };
        u.set_wifi(WifiInfo { current: None, networks: vec![forgotten] });
        assert_eq!(u.screen, Screen::NetworkDetail);
        // Connect now asks for the password, as it does for any new secured network.
        u.handle(centre(u.layout.net_connect()));
        assert_eq!(u.screen, Screen::Password, "a forgotten secured network asks for its password again");
    }

    #[test]
    fn a_hidden_network_is_joined_by_name_with_or_without_a_password() {
        use primitive_keyboard::Key;
        let mut u = ui(1);
        u.screen = Screen::Page(Page::Wifi);
        u.handle(centre(u.layout.wifi_other()));
        assert_eq!(u.screen, Screen::NetworkName);
        let key_at = |u: &Ui, k: Key, layer: primitive_keyboard::Layer| -> crate::layout::Rect {
            u.layout.keyboard_keys(layer).into_iter().find(|(d, _)| d.key == k).unwrap().1
        };
        // Next with no name asks for one.
        let go = key_at(&u, Key::Go, u.network_name.layer);
        u.handle(centre(go));
        assert!(u.notice.as_deref().unwrap().contains("name"));
        for c in ['l', 'a', 'b'] {
            let r = key_at(&u, Key::Char(c), u.network_name.layer);
            u.handle(centre(r));
        }
        u.handle(centre(go));
        assert_eq!(u.screen, Screen::Password);
        assert_eq!(u.wifi_target.as_ref().unwrap().name, "lab");
        // An empty password joins it as an open network.
        match u.handle(centre(go)) {
            Action::JoinWifi { name, secured, password, saved } => {
                assert_eq!((name.as_str(), secured, saved), ("lab", false, false));
                assert_eq!(password, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_connected_network_opens_its_screen_to_be_forgotten() {
        use crate::wifi::{Network, WifiInfo};
        let current = Network { name: "Home".into(), level: 4, max: 5, secured: true, known: true };
        let mut u = ui(1);
        u.set_wifi(WifiInfo { current: Some(current), networks: vec![] });
        u.screen = Screen::Page(Page::Wifi);
        u.handle(centre(u.layout.wifi_area().current));
        assert_eq!(u.screen, Screen::NetworkDetail);
        assert_eq!(u.handle(centre(u.layout.net_forget())), Action::ForgetWifi("Home".into()));
    }

    #[test]
    fn a_new_secured_network_asks_for_a_password_then_joins() {
        use crate::wifi::{Network, WifiInfo};
        use primitive_keyboard::{Key, Layer};
        let net = Network { name: "Cafe".into(), level: 4, max: 5, secured: true, known: false };
        let mut u = ui(1);
        u.set_wifi(WifiInfo { current: None, networks: vec![net.clone()] });
        u.screen = Screen::Page(Page::Wifi);
        u.handle(centre(u.layout.network_row(0)));
        assert_eq!(u.screen, Screen::NetworkDetail);
        u.handle(centre(u.layout.net_connect()));
        assert_eq!(u.screen, Screen::Password, "a new secured network asks for its password");
        let key_at = |u: &Ui, k: Key| -> crate::layout::Rect {
            u.layout.keyboard_keys(u.password.layer).into_iter().find(|(d, _)| d.key == k).unwrap().1
        };
        let go = key_at(&u, Key::Go);
        for c in ['a', 'b', 'c'] {
            let r = key_at(&u, Key::Char(c));
            u.handle(centre(r));
        }
        assert_eq!(u.password.string(), "abc");
        // A short password is refused with a notice.
        assert_eq!(u.handle(centre(go)), Action::Redraw { full: false });
        assert!(u.notice.as_deref().unwrap().contains("at least"));
        // Shift makes the next letter a capital, then turns off.
        let shift = key_at(&u, Key::Shift);
        u.handle(centre(shift));
        assert_eq!(u.password.layer, Layer::Upper);
        let d = key_at(&u, Key::Char('D'));
        u.handle(centre(d));
        assert_eq!(u.password.string(), "abcD");
        assert_eq!(u.password.layer, Layer::Lower, "one capital, then back to lower");
        for c in ['e', 'f', 'g', 'h'] {
            let r = key_at(&u, Key::Char(c));
            u.handle(centre(r));
        }
        match u.handle(centre(go)) {
            Action::JoinWifi { name, secured, password, saved } => {
                assert_eq!((name.as_str(), secured, saved), ("Cafe", true, false));
                assert_eq!(password.as_deref(), Some("abcDefgh"));
            }
            other => panic!("{other:?}"),
        }
        assert!(u.password.text.is_empty(), "the password is not kept after joining");
        assert_eq!(u.screen, Screen::Page(Page::Wifi));
    }

    #[test]
    fn a_saved_network_connects_without_a_password_and_can_be_forgotten() {
        use crate::wifi::{Network, WifiInfo};
        let saved = Network { name: "Home".into(), level: 3, max: 5, secured: true, known: true };
        let mut u = ui(1);
        u.set_wifi(WifiInfo { current: None, networks: vec![saved] });
        u.screen = Screen::Page(Page::Wifi);
        u.handle(centre(u.layout.network_row(0)));
        assert_eq!(u.handle(centre(u.layout.net_connect())), Action::JoinWifi { name: "Home".into(), secured: true, password: None, saved: true });
        u.screen = Screen::NetworkDetail;
        assert_eq!(u.handle(centre(u.layout.net_forget())), Action::ForgetWifi("Home".into()));
    }

    #[test]
    fn quick_settings_cards_levels_and_closing() {
        let mut u = ui(1);
        u.handle(Input::Swipe { dx: 0.0, dy: 400.0, start_y: 40.0 });
        assert_eq!(u.screen, Screen::QuickSettings);
        let layout = u.layout;
        let buttons = layout.quick_buttons();
        let card = |t: QuickTile| layout.quick_button_circle(buttons.iter().find(|(x, _)| *x == t).unwrap().1);
        assert_eq!(u.handle(centre(card(QuickTile::Airplane))), Action::Device(DeviceCmd::ToggleWifi));
        assert_eq!(u.handle(centre(card(QuickTile::AutoRotate))), Action::Device(DeviceCmd::ToggleAutoRotation));
        assert_eq!(u.handle(centre(card(QuickTile::DarkMode))), Action::Device(DeviceCmd::ToggleDarkMode));
        assert_eq!(u.handle(centre(card(QuickTile::Animation))), Action::Device(DeviceCmd::ToggleAnimation));
        assert_eq!(u.handle(centre(card(QuickTile::Ssh))), Action::Device(DeviceCmd::ToggleSsh));
        assert_eq!(u.handle(centre(card(QuickTile::AllSettings))), Action::Redraw { full: false });
        assert_eq!(u.screen, Screen::Settings);
        u.screen = Screen::QuickSettings;
        assert_eq!(u.handle(centre(u.layout.quick_minus(0))), Action::Device(DeviceCmd::Light(-1)));
        assert_eq!(u.handle(centre(u.layout.quick_plus(1))), Action::Device(DeviceCmd::Warmth(1)));
        let bar = u.layout.quick_bar(0);
        let y = (bar.y + bar.h / 2) as f32;
        assert_eq!(u.handle(Input::Tap(bar.x as f32 + 1.0, y)), Action::Device(DeviceCmd::SetLight(0)));
        assert_eq!(u.handle(Input::Tap(bar.right() as f32 - 1.0, y)), Action::Device(DeviceCmd::SetLight(24)));
        let wifi = u.layout.header_wifi();
        assert_eq!(u.handle(centre(wifi)), Action::Device(DeviceCmd::ToggleWifi));
        // Taps do not close it (the handle and outside the sheet are ignored); a swipe up does.
        assert_eq!(u.handle(centre(u.layout.quick_handle())), Action::None);
        assert_eq!(u.handle(Input::Tap(600.0, 1600.0)), Action::None);
        assert_eq!(u.screen, Screen::QuickSettings);
        u.handle(Input::Swipe { dx: 10.0, dy: -400.0, start_y: 600.0 });
        assert_eq!(u.screen, Screen::Home);
        // Swipe down from the middle of the screen does not open it; from the top edge does.
        u.handle(Input::Swipe { dx: 0.0, dy: 400.0, start_y: 600.0 });
        assert_eq!(u.screen, Screen::Home);
        u.handle(Input::Swipe { dx: 0.0, dy: 400.0, start_y: 40.0 });
        assert_eq!(u.screen, Screen::QuickSettings);
    }

    #[test]
    fn background_jobs_are_counted_for_the_status_bar() {
        let mut u = ui(0);
        let mut job = script("job.sh");
        job.background = true;
        u.apps.insert(0, job);
        assert!(matches!(u.handle(centre(u.layout.cell(0))), Action::RunBackground(_)));
        assert_eq!(u.running, 1);
        u.background_done();
        u.background_done();
        assert_eq!(u.running, 0);
    }

    #[test]
    fn a_critical_battery_goes_to_the_sleep_screen_and_wakes_to_home() {
        let mut u = ui(1);
        assert!(u.set_battery(Some(4), false));
        assert_eq!(u.screen, Screen::Sleep);
        assert_eq!(u.battery_message().as_deref(), Some("Battery critical 4%. Connect the charger now."));
        assert_eq!(u.handle(Input::Wake), Action::Redraw { full: true });
        assert_eq!(u.screen, Screen::Home);
    }

    #[test]
    fn a_critical_battery_does_not_interrupt_settings() {
        let mut u = ui(1);
        u.screen = Screen::Settings;
        u.set_battery(Some(4), false);
        assert_eq!(u.screen, Screen::Settings);
    }

    #[test]
    fn the_autostart_row_toggles_the_boot_switch() {
        let mut u = ui(1);
        u.screen = Screen::Settings;
        u.screen = Screen::Page(Page::Device);
        let row = u.layout.page_sections(Page::Device).into_iter().flat_map(|s| s.rows).find(|r| r.row == crate::layout::Row::Autostart).unwrap();
        let (_, rect) = row.buttons[0];
        assert_eq!(u.handle(Input::Tap(rect.x as f32 + 10.0, rect.y as f32 + 10.0)), Action::Device(DeviceCmd::ToggleAutostart));
    }

    #[test]
    fn a_notice_is_cleared_by_the_next_input() {
        let mut u = ui(1);
        u.notice = Some("apps.toml is not valid".into());
        assert_eq!(u.handle(Input::Tap(5.0, 1400.0)), Action::Redraw { full: false });
        assert_eq!(u.notice, None);
    }
}
