//! Launcher and Store adapters for the shared device SDK.

use std::{path::Path, sync::mpsc::Sender};

use kl_core::Rotation;

use crate::Ev;

pub use primitive_device::bluetooth;
pub use primitive_device::{
    accelerometer_turn, available_storage_bytes, check_ssh, forget_network, free_storage,
    join_network, launcher_dir, light_level, local_time, power_event, refresh_clock, refresh_screen_timeout, restart_launcher, set_screen_timeout,
    set_autostart, set_light, set_public_dns, set_ssh, set_ssh_password, set_warmth, set_wifi,
    set_stock_screensaver, status, stock_screensaver_loaded, system_command, try_set_light, try_set_warmth,
    unload_stock_screensaver, usb_state, warmth_level, wifi_info, wifi_status, wlan_ip, LEVEL_MAX, NTPDATE,
};

pub fn orientation_loop(tx: Sender<Ev>, start: Rotation) {
    primitive_device::orientation_loop(start, |turn| tx.send(Ev::Turn(turn)).is_ok());
}

pub fn power_loop(tx: Sender<Ev>) {
    primitive_device::power_loop(|input| {
        let _ = tx.send(Ev::Input(input));
    });
}

/// Loading a trusted key opts the device into signed catalog and package verification.
pub fn store_catalog_key() -> std::io::Result<Option<kl_store::signatures::TrustedKey>> {
    #[cfg(target_os = "linux")]
    {
        use std::io::{Error, ErrorKind};
        let Some(bytes) = crate::package_hooks::read_store_policy_file(crate::package_hooks::SIGNING_KEY_FILE, 128)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
        kl_store::signatures::TrustedKey::from_hex(text.trim())
            .map(Some)
            .map_err(|error| Error::new(ErrorKind::InvalidData, error))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(None)
    }
}

/// Resolve the local Kindle's architecture and firmware version for the Store.
pub fn store_device() -> Option<kl_store::resolve::Device> {
    let text = |path: &str| std::fs::read_to_string(path).ok();
    let firmware = text("/etc/prettyversion.txt")
        .and_then(|t| kl_store::version::firmware_from_text(&t))
        .or_else(|| {
            text("/etc/version.txt").and_then(|t| kl_store::version::firmware_from_text(&t))
        })?;
    let arch = if Path::new("/lib/ld-linux-armhf.so.3").exists() {
        "armhf"
    } else {
        "armel"
    };
    Some(kl_store::resolve::Device {
        arch: arch.to_string(),
        firmware,
    })
}

/// The platform codename of this Kindle (`zelda` on the Oasis 3), from its version file. The serial is never read.
pub fn platform_codename() -> Option<String> {
    kl_core::models::codename_from_version(&version_text()?)
}

/// The Kindle model this device is, when its codename is known. `None` means the owner must choose it.
pub fn detected_model() -> Option<&'static kl_core::models::Model> {
    platform_codename().and_then(|codename| kl_core::models::model_for_codename(&codename))
}

/// The catalog name of the Kindle model the launcher recorded in `device.txt`. `None` when the model is unknown (or the
/// launcher has not written the file yet): the Store then says the model is not known.
pub fn launcher_model(launcher: &Path) -> Option<String> {
    let text = std::fs::read_to_string(launcher.join("device.txt")).ok()?;
    let id = text.lines().find_map(|line| line.strip_prefix("model="))?.trim();
    kl_core::models::model_by_id(id).map(|model| model.catalog_name.to_string())
}

/// The system version text (`System Software Version: …`), as the stock UI reads it. Empty when unreadable.
pub fn version_text() -> Option<String> {
    std::fs::read_to_string("/etc/version.txt").ok()
}

/// The Wi-Fi adapter's MAC address, from the kernel. `None` when there is no Wi-Fi adapter.
pub fn mac_address() -> Option<String> {
    let address = std::fs::read_to_string("/sys/class/net/wlan0/address").ok()?;
    let address = address.trim().to_ascii_uppercase();
    (!address.is_empty()).then_some(address)
}

/// The screen panel's code (the system property `panelCode`, such as `A033_ED070KC3U1`), as the stock UI reads it.
pub fn panel_code() -> Option<String> {
    let output = std::process::Command::new("lipc-get-prop").args(["com.lab126.system", "panelCode"]).output().ok()?;
    let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !code.is_empty()).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::launcher_model;
    use std::path::PathBuf;

    /// A fresh folder for one test, so tests never share a `device.txt`.
    fn folder(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("kl-kindle-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_store_reads_the_catalog_name_of_the_model_the_launcher_recorded() {
        let dir = folder("oasis3");
        std::fs::write(dir.join("device.txt"), "model=oasis3\nmodel_name=Kindle Oasis 3\narch=armhf\n").unwrap();
        assert_eq!(launcher_model(&dir).as_deref(), Some("KindleOasis3"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unknown_model_or_a_missing_record_gives_no_model() {
        let dir = folder("unknown");
        std::fs::write(dir.join("device.txt"), "model=unknown\narch=armhf\n").unwrap();
        assert_eq!(launcher_model(&dir), None, "the Store says the model is not known");
        std::fs::remove_file(dir.join("device.txt")).unwrap();
        assert_eq!(launcher_model(&dir), None, "no device.txt yet: the launcher has not run");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
