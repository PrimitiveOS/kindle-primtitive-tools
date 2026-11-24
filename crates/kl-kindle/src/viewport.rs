//! Records for a foreground app that explicitly accepts the viewport handoff.

use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    str::FromStr,
};

use kl_core::viewport::{LogicalTurn, ViewportTransform};
pub use primitive_eink::FramebufferGeometry;

const MAX_RECORD: usize = 192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportOwner {
    pub pid: u32,
    pub started_at: u64,
    scope: String,
}

impl ViewportOwner {
    /// The current supervisor's run record, shared with the v1 portrait path.
    pub fn parse(text: &str) -> Option<Self> {
        let mut fields = fields(text)?;
        if fields.next()? != "v1" {
            return None;
        }
        let owner = Self::parse_fields(&mut fields)?;
        if fields.next().is_some() {
            return None;
        }
        Some(owner)
    }

    fn parse_fields(fields: &mut std::str::SplitAsciiWhitespace<'_>) -> Option<Self> {
        let pid = decimal(fields.next()?)?;
        let started_at = decimal(fields.next()?)?;
        let scope = fields.next()?;
        if !(2..=i32::MAX as u32).contains(&pid) || !valid_scope(scope) {
            return None;
        }
        Some(Self {
            pid,
            started_at,
            scope: scope.to_owned(),
        })
    }

    pub fn read(path: &Path) -> Option<Self> {
        Self::parse(&read_bounded(path, MAX_RECORD)?)
    }

    pub fn viewport_path(&self, run: &Path) -> PathBuf {
        run.join(&self.scope).join("viewport")
    }
}

/// The launcher offers one sibling of the legacy turn record for this run.
pub fn valid_viewport_offer(run: &Path, path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(scope) = parent.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    parent.parent() == Some(run)
        && valid_scope(scope)
        && path.file_name().is_some_and(|name| name == "viewport")
}

fn valid_scope(scope: &str) -> bool {
    scope.strip_prefix("app-turn.").is_some_and(|token| {
        token.len() == 6 && token.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub generation: u64,
    pub turn: LogicalTurn,
    pub logical_size: (u32, u32),
}

impl Viewport {
    pub fn matches(self, framebuffer: FramebufferGeometry) -> bool {
        framebuffer.pitch >= framebuffer.native_size.0
            && ViewportTransform::new(framebuffer.native_size, self.turn, framebuffer.turn)
                .is_some_and(|transform| transform.logical_size() == self.logical_size)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyViewport {
    pub owner: ViewportOwner,
    pub viewport: Viewport,
    pub framebuffer: FramebufferGeometry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportRecord {
    Pending {
        owner: ViewportOwner,
        viewport: Viewport,
    },
    Ready(ReadyViewport),
}

impl ViewportRecord {
    pub fn parse(text: &str) -> Option<Self> {
        let mut fields = fields(text)?;
        if fields.next()? != "v2" {
            return None;
        }
        let state = fields.next()?;
        let owner = ViewportOwner::parse_fields(&mut fields)?;
        let viewport = Viewport {
            generation: decimal(fields.next()?)?,
            turn: match fields.next()? {
                "U" => LogicalTurn::Upright,
                "CW" => LogicalTurn::Clockwise,
                "D" => LogicalTurn::UpsideDown,
                "CCW" => LogicalTurn::Counterclockwise,
                _ => return None,
            },
            logical_size: size(&mut fields)?,
        };
        let record = match state {
            "pending" => Self::Pending { owner, viewport },
            "ready" => {
                let framebuffer = FramebufferGeometry {
                    native_size: size(&mut fields)?,
                    turn: decimal(fields.next()?)?,
                    yoffset: decimal(fields.next()?)?,
                    pitch: decimal(fields.next()?)?,
                };
                if !viewport.matches(framebuffer) {
                    return None;
                }
                Self::Ready(ReadyViewport {
                    owner,
                    viewport,
                    framebuffer,
                })
            }
            _ => return None,
        };
        if fields.next().is_some() {
            return None;
        }
        Some(record)
    }

    pub fn encode(&self) -> String {
        let (state, owner, viewport, framebuffer) = match self {
            Self::Pending { owner, viewport } => ("pending", owner, viewport, None),
            Self::Ready(ready) => (
                "ready",
                &ready.owner,
                &ready.viewport,
                Some(ready.framebuffer),
            ),
        };
        let turn = match viewport.turn {
            LogicalTurn::Upright => "U",
            LogicalTurn::Clockwise => "CW",
            LogicalTurn::UpsideDown => "D",
            LogicalTurn::Counterclockwise => "CCW",
        };
        let mut text = format!(
            "v2 {state} {} {} {} {} {turn} {} {}",
            owner.pid,
            owner.started_at,
            owner.scope,
            viewport.generation,
            viewport.logical_size.0,
            viewport.logical_size.1,
        );
        if let Some(fb) = framebuffer {
            use std::fmt::Write;
            write!(
                text,
                " {} {} {} {} {}",
                fb.native_size.0, fb.native_size.1, fb.turn, fb.yoffset, fb.pitch
            )
            .expect("writing into a String cannot fail");
        }
        text.push('\n');
        text
    }

    pub fn read(path: &Path) -> Option<Self> {
        Self::parse(&read_bounded(path, MAX_RECORD)?)
    }

    pub fn owner(&self) -> &ViewportOwner {
        match self {
            Self::Pending { owner, .. } => owner,
            Self::Ready(ready) => &ready.owner,
        }
    }

    pub fn viewport(&self) -> Viewport {
        match self {
            Self::Pending { viewport, .. } => *viewport,
            Self::Ready(ready) => ready.viewport,
        }
    }

    /// Validate the record against independent live observations.
    pub fn validate_ready(
        self,
        owner: &ViewportOwner,
        framebuffer: FramebufferGeometry,
        minimum_generation: u64,
    ) -> Option<ReadyViewport> {
        let Self::Ready(ready) = self else {
            return None;
        };
        (&ready.owner == owner
            && ready.framebuffer == framebuffer
            && ready.viewport.generation >= minimum_generation
            && ready.viewport.matches(framebuffer))
        .then_some(ready)
    }
}

fn fields(text: &str) -> Option<std::str::SplitAsciiWhitespace<'_>> {
    let line = text.strip_suffix('\n').unwrap_or(text);
    (text.len() <= MAX_RECORD
        && line
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' '))
    .then(|| line.split_ascii_whitespace())
}

fn decimal<T: FromStr>(value: &str) -> Option<T> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn size(fields: &mut std::str::SplitAsciiWhitespace<'_>) -> Option<(u32, u32)> {
    let width = decimal(fields.next()?)?;
    let height = decimal(fields.next()?)?;
    let valid = 1..=i32::MAX as u32;
    (valid.contains(&width) && valid.contains(&height)).then_some((width, height))
}

fn read_bounded(path: &Path, max_bytes: usize) -> Option<String> {
    // A malformed runtime path must not block the input loop on a pipe or
    // take it outside the named run by following a file-level symlink.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut buf = String::new();
    file.take(max_bytes as u64 + 1)
        .read_to_string(&mut buf)
        .ok()?;
    (buf.len() <= max_bytes).then_some(buf)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveViewportOwner {
    pub owner: ViewportOwner,
    pub stopped: bool,
}

pub fn live_viewport_owner(run: &Path, procfs: &Path) -> Option<LiveViewportOwner> {
    let owner = ViewportOwner::read(&run.join("app-turn-owner"))?;
    let registered: u32 = decimal(read_bounded(&run.join("app.pid"), 32)?.trim())?;
    if registered != owner.pid {
        return None;
    }
    let stat = read_bounded(&procfs.join(owner.pid.to_string()).join("stat"), 4096)?;
    let (name, fields) = stat.rsplit_once(") ")?;
    if !name.starts_with(&format!("{} (", owner.pid)) {
        return None;
    }
    let mut fields = fields.split_ascii_whitespace();
    let state = fields.next()?;
    if !matches!(state, "R" | "S" | "D" | "T" | "t" | "W" | "K" | "P" | "I")
        || decimal::<u64>(fields.nth(18)?)? != owner.started_at
    {
        return None;
    }
    Some(LiveViewportOwner {
        owner,
        stopped: matches!(state, "T" | "t"),
    })
}

/// Read the active run's record. A caller can require a stopped app for frame
/// capture, or use the running record to select preliminary hit regions.
pub fn read_ready_viewport(
    run: &Path,
    procfs: &Path,
    framebuffer: FramebufferGeometry,
    minimum_generation: u64,
    require_stopped: bool,
) -> Option<ReadyViewport> {
    let before = live_viewport_owner(run, procfs)?;
    if require_stopped && !before.stopped {
        return None;
    }
    let record = ViewportRecord::read(&before.owner.viewport_path(run))?;
    let after = live_viewport_owner(run, procfs)?;
    if before.owner != after.owner || (require_stopped && !after.stopped) {
        return None;
    }
    record.validate_ready(&after.owner, framebuffer, minimum_generation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    const OWNER: &str = "v1 43 8701 app-turn.ABC123\n";
    const PENDING: &str = "v2 pending 43 8701 app-turn.ABC123 12 CW 1680 1264\n";
    const READY: &str = "v2 ready 43 8701 app-turn.ABC123 12 CW 1680 1264 1264 1680 2 17 1280\n";
    const FB: FramebufferGeometry = FramebufferGeometry {
        native_size: (1264, 1680),
        turn: 2,
        yoffset: 17,
        pitch: 1280,
    };

    #[test]
    fn records_are_bounded_and_validate_state_generation_scope_and_geometry() {
        let owner = ViewportOwner::parse(OWNER).unwrap();
        let pending = ViewportRecord::parse(PENDING).unwrap();
        assert_eq!(pending.encode(), PENDING);
        assert!(pending.validate_ready(&owner, FB, 12).is_none());

        let ready = ViewportRecord::parse(READY).unwrap();
        assert_eq!(ready.encode(), READY);
        assert!(ready.clone().validate_ready(&owner, FB, 12).is_some());
        assert!(ready.clone().validate_ready(&owner, FB, 13).is_none());
        for wrong_fb in [
            FramebufferGeometry { turn: 0, ..FB },
            FramebufferGeometry { pitch: 1300, ..FB },
            FramebufferGeometry { yoffset: 18, ..FB },
            FramebufferGeometry {
                native_size: (1264, 1679),
                ..FB
            },
        ] {
            assert!(ready.clone().validate_ready(&owner, wrong_fb, 12).is_none());
        }
        for wrong_owner in [
            "v1 43 8702 app-turn.ABC123\n",
            "v1 43 8701 app-turn.DEF456\n",
            "v1 44 8701 app-turn.ABC123\n",
        ] {
            let wrong_owner = ViewportOwner::parse(wrong_owner).unwrap();
            assert!(ready.clone().validate_ready(&wrong_owner, FB, 12).is_none());
        }
    }

    #[test]
    fn rejects_wrong_version_layout_turn_path_extra_fields_and_invalid_numbers() {
        for malformed in [
            READY.replace("v2", "v1"),
            READY.replace("ready", "drawing"),
            READY.replace("app-turn.ABC123", "../escape"),
            READY.replace("app-turn.ABC123", "app-turn.ABCD*3"),
            READY.replace("app-turn.ABC123", "app-turn.ABCD"),
            READY.replace(" 43 ", " -1 "),
            READY.replace(" 43 ", " 1 "),
            READY.replace(" 12 ", " +12 "),
            READY.replace(" 12 ", " 18446744073709551616 "),
            READY.replace(" 17 ", " -17 "),
            READY.replace(" CW ", " R "),
            READY.replace(" CW ", " D "),
            READY.replace(" 2 ", " 1 "),
            READY.replace(" 2 ", " 3 "),
            READY.replace(" 1280", " 1250"),
            READY.replace(" 1280", ""),
            READY.replace(" 1264", " 0"),
            format!("{} extra", READY.trim_end()),
            format!("{} {}", READY.trim_end(), " ".repeat(MAX_RECORD)),
            format!("{READY}\n"),
            "v2 ready 43 8701 app-turn.ABC123 12\nCW 1680 1264 1264 1680 2 17 1280".to_owned(),
        ] {
            assert!(ViewportRecord::parse(&malformed).is_none(), "{malformed:?}");
        }
        let run = Path::new("/runtime");
        assert!(valid_viewport_offer(
            run,
            &run.join("app-turn.ABC123/viewport")
        ));
        assert!(!valid_viewport_offer(
            run,
            &run.join("app-turn.ABC123/turn")
        ));
        assert!(!valid_viewport_offer(
            run,
            &run.join("outer/app-turn.ABC123/viewport")
        ));
    }

    struct TempRun(PathBuf);

    impl TempRun {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "viewport-observer-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("proc/43")).unwrap();
            fs::create_dir(path.join("app-turn.ABC123")).unwrap();
            Self(path)
        }

        fn stat(&self, state: &str, started: u64) {
            fs::write(
                self.0.join("proc/43/stat"),
                format!(
                    "43 (Reader ) with a space) {state} {}{started} 0\n",
                    "0 ".repeat(18)
                ),
            )
            .unwrap();
        }

        fn read(&self, require_stopped: bool) -> Option<ReadyViewport> {
            read_ready_viewport(&self.0, &self.0.join("proc"), FB, 12, require_stopped)
        }
    }

    impl Drop for TempRun {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn live_capture_defers_until_matching_owner_is_stopped_and_ready() {
        let run = TempRun::new();
        let owner_path = run.0.join("app-turn-owner");
        let record_path = run.0.join("app-turn.ABC123/viewport");
        fs::write(&owner_path, OWNER).unwrap();
        fs::write(run.0.join("app.pid"), "43\n").unwrap();
        run.stat("S", 8701);
        assert!(run.read(false).is_none());

        let alternate = run.0.join("alternate");
        fs::write(&alternate, READY).unwrap();
        std::os::unix::fs::symlink(&alternate, &record_path).unwrap();
        assert!(run.read(false).is_none());
        fs::remove_file(&record_path).unwrap();
        fs::create_dir(&record_path).unwrap();
        assert!(run.read(false).is_none());
        fs::remove_dir(&record_path).unwrap();
        fs::write(&record_path, READY).unwrap();
        assert!(run.read(false).is_some());
        assert!(run.read(true).is_none());
        run.stat("T", 8701);
        assert!(run.read(true).is_some());

        for record in [PENDING, "malformed", "v2 ready"] {
            fs::write(&record_path, record).unwrap();
            assert!(run.read(false).is_none());
            assert!(run.read(true).is_none());
        }
        fs::write(&record_path, READY.replace(" 12 ", " 11 ")).unwrap();
        assert!(run.read(true).is_none());
        fs::write(&record_path, READY).unwrap();

        run.stat("Z", 8701);
        assert!(run.read(true).is_none());
        run.stat("T", 8702);
        assert!(run.read(true).is_none());
        run.stat("T", 8701);
        fs::write(run.0.join("app.pid"), "44\n").unwrap();
        assert!(run.read(true).is_none());
        fs::write(run.0.join("app.pid"), "43\n").unwrap();
        fs::write(owner_path, "v1 43 8701 app-turn.DEF456\n").unwrap();
        assert!(run.read(true).is_none());
    }
}
