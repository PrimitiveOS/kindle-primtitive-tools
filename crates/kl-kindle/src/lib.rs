//! The Kindle device layer, shared by Primitive Launcher and Store: the e-ink panel, the touch and button
//! input, the device's status, and the events an app's loop reads.

pub mod device;
pub mod input;
pub mod package_hooks;

pub use primitive_device::{log, profile};
pub use primitive_eink::{self as eink, display};

use kl_core::{Input, Rotation};

pub enum Ev {
    Input(Input),
    Tick,
    /// The USB state changed (the cable, or the drive shown on a computer).
    Usb(kl_core::Usb),
    /// Draw the drive-mode screen again: the stock system draws its own USB screen when the drive goes to the computer.
    UsbRedraw,
    /// The device was turned (auto-rotation, after the debouncer).
    Turn(Rotation),
    /// A background app ended: its name, last output line and whether it succeeded.
    Background { name: String, line: String, ok: bool },
    /// A Wi-Fi scan finished.
    Wifi(kl_core::wifi::WifiInfo),
    /// The sound level changed (`None`: no headphones are connected, so the sound row is hidden).
    Sound(Option<u8>),
    /// The media session quick settings shows changed (`None`: no app plays or is paused).
    Media(Option<kl_core::media::MediaSession>),
    /// A page-turn key was pressed (`down`) or released: the launcher times a hold of both (restart).
    PageKey { code: u16, down: bool },
    /// The two-button hold may be complete: check it.
    ChordTick,
    /// A Wi-Fi join or forget finished: whether it worked, and what to tell the user.
    WifiResult { ok: bool, message: String },
    /// Bluetooth's state and devices: after a scan, a change, or a pairing step.
    Bluetooth(kl_core::bluetooth::BtInfo),
    /// A Bluetooth result or error for the notice line (it never carries an address).
    BluetoothNotice(String),
    /// The stack asks to confirm a pairing: the text it sent (the code, when the device shows one).
    BluetoothConfirm(String),
    /// A background SSH job finished (set the password, check the server): what to tell the user.
    Notice(String),
    /// The catalog refresh finished. A failed refresh leaves the currently displayed catalog in place.
    StoreRefreshed { entries: Option<Vec<kl_core::store::StoreEntry>>, message: String, artwork_changed: bool },
    /// An app from the store was installed (or not): the store's entries, the library's apps again, and the message.
    StoreInstalled { entries: Vec<kl_core::store::StoreEntry>, message: String },
    /// The store's install sheet changed stage or crossed another progress step.
    StoreProgress { message: String, percent: Option<u8> },
    /// The launcher finished installing the standalone store.
    StoreBootstrapped(Result<(), String>),
    /// The app the supervisor ran for the launcher has ended (or never started): the launcher takes the screen back.
    AppEnded,
    /// Settings finished its leased package change and can refresh the app list.
    ManagedAppsChanged {
        lease: kl_core::storage_lease::StorageLease,
        storage_held: bool,
        message: Option<String>,
    },
}
