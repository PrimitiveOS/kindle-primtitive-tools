//! Media sessions on the device (the contract in docs/contracts.md §1.2): read the apps' `.now` files, and send them
//! commands through their `.cmd` files. Apps write the files; the launcher only reads and appends.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use kl_core::media::{self, MediaCommand, MediaSession};

/// The folder of the session files (RAM: cleared when the Kindle restarts).
pub const DIR: &str = "/var/tmp/media";

/// The sessions that play or are paused, whose player process still runs.
pub fn sessions() -> Vec<MediaSession> {
    let Ok(entries) = fs::read_dir(DIR) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "now"))
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|text| media::parse(&text))
        .filter(|session| session.pid.is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists()))
        .collect()
}

/// The session quick settings shows, if any.
pub fn current() -> Option<MediaSession> {
    media::choose(sessions())
}

/// Send a command to an app's player. Returns whether the line was written.
pub fn send(app: &str, command: MediaCommand) -> bool {
    if !media::valid_app(app) {
        return false;
    }
    let path = format!("{DIR}/{app}.cmd");
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{}", command.line()))
        .is_ok()
}
