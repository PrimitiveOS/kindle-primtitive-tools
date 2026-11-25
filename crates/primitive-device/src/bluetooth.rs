//! Bluetooth through the stock stack (`com.lab126.btfd`, over lipc): the radio on and off, a scan, the device lists,
//! pairing, connecting and forgetting. Every call here blocks (each lipc call has a time limit, and the stack's
//! answers are waited for with a deadline), so the driver runs these on its worker thread.
//!
//! Protocol (verified on the Kindle, Oasis 3, firmware 5.18.2): on is `BTenable 1:1`; off is a restart of the
//! stack's job (`initctl restart btfd`), because `BTenable 0:1` is not reliable (it does nothing while a scan
//! runs). A scan is `triggerBTscan 1` (an integer). The lists are read with `lipc-hash-prop -n`.
//!
//! Device addresses are passed to lipc only. Errors are plain words: no address is ever put in a message.

use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::{mpsc, Mutex},
    thread,
    time::{Duration, Instant},
};

use kl_core::bluetooth::{self, BdAddr, BtInfo};

const BTFD: &str = "com.lab126.btfd";

/// Run a command with a time limit, optionally with one line on its standard input. Returns its output when it
/// exits successfully; `None` when it fails, cannot start, or runs past the limit (then it is killed).
fn run(args: &[&str], limit: Duration, input: Option<&str>) -> Option<String> {
    let mut child = Command::new(args[0])
        .args(&args[1..])
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        use std::io::Write;
        let _ = stdin.write_all(format!("{text}\n").as_bytes());
    }
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }
                return status.success().then_some(out);
            }
            Ok(None) if start.elapsed() < limit => thread::sleep(Duration::from_millis(40)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(_) => return None,
        }
    }
}

const SHORT: Duration = Duration::from_secs(10);

/// Whether the stack answers (it is installed and running). False on a machine without the Kindle's services.
pub fn available() -> bool {
    state_text().is_some()
}

fn state_text() -> Option<String> {
    run(&["lipc-get-prop", BTFD, "BTstate"], SHORT, None)
}

/// Whether the radio is on.
pub fn is_on() -> bool {
    state_text().is_some_and(|s| bluetooth::is_on(&s))
}

/// Turn the radio on (`BTenable 1:1`) and wait for it (about 2 s). Returns whether it came on.
pub fn set_on(on: bool) -> bool {
    if on {
        let _ = run(&["lipc-set-prop", BTFD, "BTenable", "1:1"], SHORT, None);
        return wait_for(Duration::from_secs(15), is_on);
    }
    // Off: restart the stack's job (the reliable form), then wait until it reports off or has restarted.
    let _ = run(&["initctl", "restart", "btfd"], Duration::from_secs(20), None);
    wait_for(Duration::from_secs(10), || !is_on())
}

fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if done() {
            return true;
        }
        thread::sleep(Duration::from_millis(500));
    }
    done()
}

/// Start a scan. The stack reports devices as it finds them, and stops by itself; the lists are read afterwards.
/// Returns whether the request was accepted.
pub fn start_scan() -> bool {
    run(&["lipc-set-prop", "-i", BTFD, "triggerBTscan", "1"], SHORT, None).is_some()
}

/// One of the stack's lists as text: `ListDiscovered`, `ListPaired` or `ListConnected`.
fn list(name: &str) -> String {
    run(&["lipc-hash-prop", "-n", BTFD, name], SHORT, Some("")).unwrap_or_default()
}

/// The connected device's name as the last list read saw it. The status bar reads this (through `connected_name`)
/// without reading the lists again.
static CONNECTED: Mutex<Option<String>> = Mutex::new(None);

/// The name of the connected device, as the last list read saw it.
pub fn connected_name() -> Option<String> {
    CONNECTED.lock().ok().and_then(|g| g.clone())
}

fn remember(info: &BtInfo) {
    if let Ok(mut g) = CONNECTED.lock() {
        *g = info.connected_name();
    }
}

/// The page's state: the radio, and the devices (nearby audio, paired and connected ones). Off gives no devices.
pub fn info() -> BtInfo {
    if !is_on() {
        remember(&BtInfo::default());
        return BtInfo::default();
    }
    let devices = bluetooth::merge(
        bluetooth::parse_devices(&list("ListDiscovered")),
        bluetooth::parse_devices(&list("ListPaired")),
        bluetooth::parse_devices(&list("ListConnected")),
    );
    let info = BtInfo { on: true, devices };
    remember(&info);
    info
}

/// Scan for about twelve seconds, then read the lists. Blocks; the driver calls it on its worker thread.
pub fn scan_and_list() -> BtInfo {
    if start_scan() {
        thread::sleep(Duration::from_secs(12));
    }
    info()
}

/// What the stack asks while a pairing runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairStep {
    /// The stack asks to confirm (the device shows a code); the text is what the stack sent, for the screen.
    Confirm(String),
    /// The pairing ended: true when it worked.
    Done(bool),
}

/// Pair with a device: start the pairing, then report what the stack asks until it finishes (or 60 s pass). The
/// answer to a confirmation comes back through `confirm`. Blocks; the driver calls it on its worker thread.
pub fn pair(addr: &BdAddr, mut report: impl FnMut(PairStep)) -> Result<(), String> {
    let mut waiter = Waiter::start(&["btConfirmationEvent", "Bond_Result"])?;
    if run(&["lipc-set-prop", BTFD, "Bond", addr.as_str()], SHORT, None).is_none() {
        waiter.stop();
        return Err("the stack did not start the pairing".into());
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            waiter.stop();
            return Err("the device did not answer in time. Put it in pairing mode, and try again.".into());
        }
        match waiter.next(left) {
            Some((name, rest)) if name == "btConfirmationEvent" => report(PairStep::Confirm(rest)),
            Some((name, rest)) if name == "Bond_Result" => {
                waiter.stop();
                // The result's first value is 0 when the pairing worked.
                let ok = rest.split_whitespace().next() == Some("0");
                report(PairStep::Done(ok));
                return if ok { Ok(()) } else { Err("the pairing did not work. Try again.".into()) };
            }
            Some(_) => {}
            None => {
                waiter.stop();
                return Err("the device did not answer in time. Put it in pairing mode, and try again.".into());
            }
        }
    }
}

/// The owner's answer to a pairing confirmation: yes or no.
pub fn confirm(yes: bool) -> bool {
    run(&["lipc-set-prop", "-i", BTFD, "PairUserConfirmation", if yes { "1" } else { "0" }], SHORT, None).is_some()
}

/// Connect a paired device. Waits up to 25 s for it to show as connected.
pub fn connect(addr: &BdAddr) -> Result<(), String> {
    if run(&["lipc-set-prop", BTFD, "Connect", addr.as_str()], SHORT, None).is_none() {
        return Err("the stack did not start the connection".into());
    }
    let connected = wait_for(Duration::from_secs(25), || {
        bluetooth::parse_devices(&list("ListConnected")).iter().any(|d| &d.address == addr)
    });
    if connected { Ok(()) } else { Err("it did not connect. Turn it on, and try again.".into()) }
}

/// Disconnect a device. Waits up to 15 s for it to leave the connected list.
pub fn disconnect(addr: &BdAddr) -> Result<(), String> {
    if run(&["lipc-set-prop", BTFD, "Disconnect", addr.as_str()], SHORT, None).is_none() {
        return Err("the stack did not start the disconnection".into());
    }
    let gone = wait_for(Duration::from_secs(15), || {
        !bluetooth::parse_devices(&list("ListConnected")).iter().any(|d| &d.address == addr)
    });
    if gone { Ok(()) } else { Err("it did not disconnect. Try again.".into()) }
}

/// Forget a paired device (its pairing is removed). Waits up to 10 s for it to leave the paired list.
pub fn forget(addr: &BdAddr) -> Result<(), String> {
    if run(&["lipc-set-prop", BTFD, "Unbond", addr.as_str()], SHORT, None).is_none() {
        return Err("the stack did not forget it".into());
    }
    let gone = wait_for(Duration::from_secs(10), || {
        !bluetooth::parse_devices(&list("ListPaired")).iter().any(|d| &d.address == addr)
    });
    if gone { Ok(()) } else { Err("it is still paired. Try again.".into()) }
}

/// Watches the stack's events for a few names: a reader thread feeds lines (name, rest) to the caller, and `stop`
/// ends the listener. The listener is a child process; it is killed on stop and when this value is dropped, so no
/// `lipc-wait-event` is left behind.
struct Waiter {
    child: std::process::Child,
    lines: mpsc::Receiver<(String, String)>,
}

impl Waiter {
    fn start(names: &[&str]) -> Result<Waiter, String> {
        let mut child = Command::new("lipc-wait-event")
            .args(["-m", BTFD, &names.join(",")])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "the stack's events are not available".to_string())?;
        let Some(out) = child.stdout.take() else {
            let _ = child.kill();
            return Err("the stack's events are not available".into());
        };
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let (name, rest) = match line.split_once(' ') {
                    Some((name, rest)) => (name.to_string(), rest.trim().to_string()),
                    None => (line.trim().to_string(), String::new()),
                };
                if tx.send((name, rest)).is_err() {
                    break;
                }
            }
        });
        Ok(Waiter { child, lines })
    }

    /// The next event, or `None` when `wait` passes with none.
    fn next(&mut self, wait: Duration) -> Option<(String, String)> {
        self.lines.recv_timeout(wait).ok()
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A tone as a 16-bit mono WAV file: `millis` long, at `rate` Hz, with a fade in and out so it does not click.
pub fn test_tone_wav(rate: u32, hz: f64, millis: u32) -> Vec<u8> {
    let samples = (rate as u64 * millis as u64 / 1000) as u32;
    let data_len = samples * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    let fade = (rate / 50).max(1) as f64; // 20 ms at each end
    for i in 0..samples {
        let t = i as f64 / rate as f64;
        let edge = (i as f64).min((samples - i) as f64).min(fade) / fade;
        let v = (0.5 * edge * 32_767.0 * (2.0 * std::f64::consts::PI * hz * t).sin()).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Plays a short test tone through the stock audio manager (`mplay`), which sends it to the connected headphones.
/// Blocks until the tone has played; the driver runs it on its own thread.
pub fn play_test_tone() -> Result<(), String> {
    let path = format!("{}/bt-test-tone.wav", crate::launcher_dir());
    std::fs::write(&path, test_tone_wav(16_000, 440.0, 1500)).map_err(|_| "could not make the test sound".to_string())?;
    match run(&["mplay", &path], Duration::from_secs(10), None) {
        Some(_) => Ok(()),
        None => Err("the test sound did not play. Check that the headphones are connected.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_gives_no_output() {
        assert!(run(&["/definitely/not/a/program"], SHORT, None).is_none());
    }

    #[test]
    fn a_command_past_its_limit_is_stopped() {
        let start = Instant::now();
        assert!(run(&["sleep", "5"], Duration::from_millis(200), None).is_none());
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn input_reaches_the_program() {
        assert_eq!(run(&["cat"], SHORT, Some("hello")).as_deref(), Some("hello\n"));
    }

    #[test]
    fn the_test_tone_is_a_valid_wav_of_the_right_length() {
        let wav = test_tone_wav(16_000, 440.0, 1500);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize, 24_000 * 2, "1.5 s of 16-bit samples");
        assert_eq!(wav.len(), 44 + 24_000 * 2);
        // The tone starts from silence (the fade in), so its first sample is near zero.
        assert!(i16::from_le_bytes([wav[44], wav[45]]).abs() < 100);
    }

    #[test]
    fn a_failed_status_gives_no_output() {
        assert!(run(&["false"], SHORT, None).is_none());
    }
}
