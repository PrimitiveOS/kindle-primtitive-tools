//! Bluetooth for Settings: the stock stack (`com.lab126.btfd`, through lipc). This module holds the pure parts: the
//! device model, the parsers for the stack's three lists, the merge and the sort. The lipc calls live in
//! `primitive-device`.
//!
//! Device addresses are private: `BdAddr` prints as `<addr>` in Debug and Display, and only `as_str` gives the
//! value, which goes to lipc and nowhere else. Logs, screenshots and tests never show a real address.

use std::fmt;

/// A Bluetooth device address ("AA:BB:CC:DD:EE:FF"). Its text is private: see the module docs.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BdAddr(String);

impl BdAddr {
    /// Keeps `text` when it is six hex pairs separated by colons (the stack's form), upper-cased.
    pub fn parse(text: &str) -> Option<BdAddr> {
        let text = text.trim();
        let parts: Vec<&str> = text.split(':').collect();
        let ok = parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
        ok.then(|| BdAddr(text.to_ascii_uppercase()))
    }

    /// The address text, for lipc and nothing else.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BdAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<addr>")
    }
}

impl fmt::Display for BdAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<addr>")
    }
}

/// One device the stack knows: nearby (from a scan), paired (remembered), or connected now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub address: BdAddr,
    pub paired: bool,
    pub connected: bool,
    pub discovered: bool,
    pub rssi: Option<i32>,
    /// The class of device (24 bits, from the scan). `None` when the stack did not report one.
    pub class: Option<u32>,
}

impl Device {
    /// The name to show: the device's own name, or "Unknown device".
    pub fn label(&self) -> &str {
        if self.name.is_empty() { "Unknown device" } else { &self.name }
    }

    /// Audio (major class 0x04: headphones, speakers, headsets). Only these are offered as nearby devices in v1.
    pub fn is_audio(&self) -> bool {
        self.class.is_some_and(|c| (c >> 8) & 0x1f == 0x04)
    }
}

/// The module's state for the page: whether the radio is on, and the devices in the order the page shows them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BtInfo {
    pub on: bool,
    pub devices: Vec<Device>,
}

impl BtInfo {
    /// The name of the connected device, if one is connected.
    pub fn connected_name(&self) -> Option<String> {
        self.devices.iter().find(|d| d.connected).map(|d| d.label().to_string())
    }

    /// The device with this address, if it is in the list.
    pub fn find(&self, address: &BdAddr) -> Option<&Device> {
        self.devices.iter().find(|d| &d.address == address)
    }
}

/// The message for a Bluetooth action that did not work. Plain words, no addresses.
pub const NOT_AVAILABLE: &str = "Bluetooth is not available on this Kindle";
pub const TOO_LONG: &str = "Bluetooth did not answer. Turn it off and on, and try again.";

/// Splits a `lipc-hash-prop` report into its blocks: each `Hash Index` line starts a block, and the `key = value`
/// lines under it are the block's fields. Quotes around values are removed. An empty report gives no blocks.
fn blocks(text: &str) -> Vec<Vec<(String, String)>> {
    let mut out: Vec<Vec<(String, String)>> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("Hash Index") {
            out.push(Vec::new());
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if out.is_empty() {
                out.push(Vec::new());
            }
            let value = value.trim().trim_end_matches(',').trim().trim_matches('"').to_string();
            if let Some(block) = out.last_mut() {
                block.push((key.trim().to_string(), value));
            }
        }
    }
    out
}

fn field<'a>(block: &'a [(String, String)], key: &str) -> Option<&'a str> {
    block.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// The stack writes numbers and flags in parentheses: `(1)`, `(999)`. Returns what is inside them (or the value
/// itself, when there are none).
fn inside(value: &str) -> &str {
    let v = value.trim();
    v.strip_prefix('(').and_then(|r| r.strip_suffix(')')).map(str::trim).unwrap_or(v)
}

fn truthy(value: Option<&str>) -> bool {
    matches!(value.map(inside), Some("yes" | "true" | "1" | "Yes" | "True"))
}

/// The major device classes by name, as the stack writes them in `class_of_device` ("Audio/Video"). The value is
/// the class's major number in bits 8 to 12 of the class of device.
const MAJOR_CLASSES: &[(&str, u32)] = &[
    ("Computer", 0x01),
    ("Phone", 0x02),
    ("LAN/Network", 0x03),
    ("Audio/Video", 0x04),
    ("Peripheral", 0x05),
    ("Imaging", 0x06),
    ("Wearable", 0x07),
    ("Toy", 0x08),
    ("Health", 0x09),
];

/// A class of device: a major class name ("Audio/Video"), or a number (decimal, or `0x` hex) of the whole class.
fn class_of(value: &str) -> Option<u32> {
    let v = inside(value);
    if let Some((_, major)) = MAJOR_CLASSES.iter().find(|(name, _)| *name == v) {
        return Some(major << 8);
    }
    match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => v.parse().ok(),
    }
}

/// A signal strength in dBm. The stack writes 999 when there is no reading (the device is not seen now).
fn signal(value: &str) -> Option<i32> {
    inside(value).parse::<i32>().ok().filter(|rssi| (-127..=20).contains(rssi))
}

/// One of the stack's device lists (`ListDiscovered`, `ListPaired` or `ListConnected`): one device per block with
/// `bd_name`, `bd_address`, `class_of_device`, `rssi`, `is_paired`, `is_connected` and `is_discovered`. Blocks
/// without a valid address are left out, and a repeated address keeps its first block.
pub fn parse_devices(text: &str) -> Vec<Device> {
    let mut out: Vec<Device> = Vec::new();
    for block in blocks(text) {
        let Some(address) = field(&block, "bd_address").and_then(BdAddr::parse) else { continue };
        if out.iter().any(|d| d.address == address) {
            continue;
        }
        out.push(Device {
            name: field(&block, "bd_name").unwrap_or("").to_string(),
            address,
            paired: truthy(field(&block, "is_paired")),
            connected: truthy(field(&block, "is_connected")),
            discovered: truthy(field(&block, "is_discovered")),
            rssi: field(&block, "rssi").and_then(signal),
            class: field(&block, "class_of_device").and_then(class_of),
        });
    }
    out
}

/// Joins the three lists into the page's devices. A device is kept when it is paired or connected, or when it is
/// nearby and audio. Its name comes from whichever list has one first. Order: connected, then paired (by name),
/// then nearby (strongest first, then by name).
pub fn merge(discovered: Vec<Device>, paired: Vec<Device>, connected: Vec<Device>) -> Vec<Device> {
    let mut all: Vec<Device> = Vec::new();
    for mut d in paired.into_iter().chain(connected).chain(discovered) {
        match all.iter_mut().find(|x| x.address == d.address) {
            Some(known) => {
                known.paired |= d.paired;
                known.connected |= d.connected;
                known.discovered |= d.discovered;
                if known.name.is_empty() {
                    known.name = std::mem::take(&mut d.name);
                }
                if known.rssi.is_none() {
                    known.rssi = d.rssi;
                }
                if known.class.is_none() {
                    known.class = d.class;
                }
            }
            None => all.push(d),
        }
    }
    all.retain(|d| d.paired || d.connected || (d.discovered && d.is_audio()));
    all.sort_by(|a, b| {
        let rank = |d: &Device| if d.connected { 0 } else if d.paired { 1 } else { 2 };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| {
                if rank(a) == 2 {
                    b.rssi.unwrap_or(i32::MIN).cmp(&a.rssi.unwrap_or(i32::MIN))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| a.label().to_lowercase().cmp(&b.label().to_lowercase()))
    });
    all
}

/// Whether the radio is on, from the `BTstate` value: 0 off; 1 on; 2 connected (also on).
pub fn is_on(state: &str) -> bool {
    matches!(state.trim(), "1" | "2")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Placeholder addresses only: real ones never go in the repository.
    const A: &str = "AA:BB:CC:DD:EE:01";
    const B: &str = "AA:BB:CC:DD:EE:02";
    const C: &str = "AA:BB:CC:DD:EE:03";

    fn list(blocks: &[(&str, &str, &str, &str)]) -> String {
        // (name, address, extra fields)
        blocks
            .iter()
            .map(|(name, addr, extra, _)| format!("Hash Index\nbd_name = \"{name}\"\nbd_address = \"{addr}\"\n{extra}\n"))
            .collect()
    }

    #[test]
    fn an_address_is_checked_and_never_printed() {
        let a = BdAddr::parse(" aa:bb:cc:dd:ee:01 ").unwrap();
        assert_eq!(a.as_str(), A);
        assert_eq!(format!("{a:?} {a}"), "<addr> <addr>");
        assert!(BdAddr::parse("AA:BB:CC:DD:EE").is_none());
        assert!(BdAddr::parse("AA:BB:CC:DD:EE:GG").is_none());
    }

    #[test]
    fn the_stacks_own_format_is_read_as_captured() {
        // A paired device as the Kindle's stack writes it (captured on the Oasis 3; the address is a placeholder).
        let text = "Hash Index: 0\n\tbd_name = \"AirPods Pro\"\n\tbd_address = \"AA:BB:CC:DD:EE:01\"\n\tclass_of_device = \"Audio/Video\"\n\tvid = (76)\n\tpid = (8206)\n\trssi = (999)\n\tis_paired = (1)\n\tis_connected = (0)\n\tis_discovered = (0)\n";
        let d = &parse_devices(text)[0];
        assert_eq!(d.label(), "AirPods Pro");
        assert!(d.is_audio(), "the class name Audio/Video is the audio major class");
        assert_eq!(d.rssi, None, "999 is no reading");
        assert!(d.paired && !d.connected && !d.discovered);
        let nearby = "Hash Index: 0\n\tbd_name = \"Speaker\"\n\tbd_address = \"AA:BB:CC:DD:EE:02\"\n\tclass_of_device = \"Audio/Video\"\n\trssi = (-61)\n\tis_discovered = (1)\n";
        let n = &parse_devices(nearby)[0];
        assert_eq!(n.rssi, Some(-61));
        assert!(n.discovered && !n.paired);
        assert!(!parse_devices("Hash Index: 0\n\tbd_name = \"Phone\"\n\tbd_address = \"AA:BB:CC:DD:EE:03\"\n\tclass_of_device = \"Phone\"\n")[0].is_audio());
    }

    #[test]
    fn an_empty_list_is_no_devices() {
        assert!(parse_devices("").is_empty());
        assert!(parse_devices("\n").is_empty());
    }

    #[test]
    fn a_list_gives_one_device_per_block_and_skips_bad_ones() {
        let text = list(&[("Headphones", A, "class_of_device = 1076\nrssi = -48\nis_discovered = yes", "")])
            + "Hash Index\nbd_name = \"No address\"\n";
        let found = parse_devices(&text);
        assert_eq!(found.len(), 1);
        let d = &found[0];
        assert_eq!(d.label(), "Headphones");
        assert_eq!(d.rssi, Some(-48));
        assert_eq!(d.class, Some(1076));
        assert!(d.is_audio(), "1076 is 0x434, the Audio/Video major class");
        assert!(d.discovered && !d.paired && !d.connected);
    }

    #[test]
    fn a_class_can_be_hex_and_a_nameless_device_is_unknown() {
        let text = "Hash Index\nbd_address = \"AA:BB:CC:DD:EE:02\"\nclass_of_device = 0x240404\nis_paired = yes\n";
        let d = &parse_devices(text)[0];
        assert_eq!(d.label(), "Unknown device");
        assert!(d.is_audio());
        assert!(d.paired);
    }

    #[test]
    fn merge_keeps_paired_and_connected_and_only_nearby_audio() {
        let discovered = parse_devices(&list(&[
            ("Speaker", A, "class_of_device = 1076\nrssi = -60\nis_discovered = yes", ""),
            ("Watch", B, "class_of_device = 1792\nrssi = -30\nis_discovered = yes", ""),
            ("Phone", C, "class_of_device = 524\nrssi = -20\nis_discovered = yes", ""),
        ]));
        let paired = parse_devices(&list(&[("Speaker", A, "is_paired = yes", "")]));
        let connected = parse_devices(&list(&[("Speaker", A, "is_connected = yes", "")]));
        let merged = merge(discovered, paired, connected);
        assert_eq!(merged.len(), 1, "the watch and the phone are not audio; only the paired, connected speaker stays");
        assert!(merged[0].connected && merged[0].paired);
    }

    #[test]
    fn merge_sorts_connected_then_paired_then_nearby_by_signal() {
        let discovered = parse_devices(&list(&[
            ("Far", B, "class_of_device = 1076\nrssi = -90\nis_discovered = yes", ""),
            ("Near", C, "class_of_device = 1076\nrssi = -35\nis_discovered = yes", ""),
            ("Mine", A, "class_of_device = 1076\nrssi = -70\nis_discovered = yes", ""),
        ]));
        let paired = parse_devices(&list(&[("Mine", A, "is_paired = yes", "")]));
        let connected = parse_devices(&list(&[("Mine", A, "is_connected = yes", "")]));
        let merged = merge(discovered, paired, connected);
        let names: Vec<&str> = merged.iter().map(|d| d.label()).collect();
        assert_eq!(names, ["Mine", "Near", "Far"]);
        let order = BtInfo { on: true, devices: merged };
        assert_eq!(order.connected_name().as_deref(), Some("Mine"));
        assert!(order.find(&BdAddr::parse(C).unwrap()).is_some());
    }

    #[test]
    fn a_name_seen_in_one_list_is_kept_after_the_merge() {
        let discovered = parse_devices(&list(&[("", A, "class_of_device = 1076\nis_discovered = yes", "")]));
        let paired = parse_devices(&list(&[("Named", A, "is_paired = yes", "")]));
        assert_eq!(merge(discovered, paired, vec![])[0].label(), "Named");
    }

    #[test]
    fn the_radio_is_on_for_states_one_and_two() {
        assert!(!is_on("0"));
        assert!(is_on("1\n"));
        assert!(is_on("2"));
        assert!(!is_on(""));
    }
}
