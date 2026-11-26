//! Platform-free logic of Primitive Launcher: the app list (scriptlets, `apps.toml`, built-ins), the
//! settings file, the screen layout and the UI state machine. Nothing here touches the Kindle, so all of it
//! is tested on the Mac.

pub mod apps;
pub mod base64;
pub mod bluetooth;
pub mod battery;
pub mod config;
pub mod kual;
pub mod layout;
pub mod media;
pub mod rotation;
pub mod scriptlet;
pub mod settings;
pub mod storage_lease;
pub mod store;
pub mod store_open;
pub mod tz;
pub mod ui;
pub mod wallpaper;
pub mod wifi;
pub mod models;
pub mod usage;
pub mod page_chord;

pub use apps::{discover, App, IconSource, Launch};
pub use config::Config;
pub use layout::{Layout, Rect};
pub use settings::{Rotation, Settings};
pub use ui::{Action, DeviceCmd, Input, InstalledApp, Screen, Status, Ui, Usb};

