//! Publish a projected frame to a Launcher that offers viewport handoff for this run.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use kl_core::viewport::ViewportTransform;
use kl_kindle::{
    display::Display,
    profile,
    viewport::{live_viewport_owner, valid_viewport_offer, ViewportOwner, ViewportRecord},
};

use crate::write_atomic;

pub use kl_core::viewport::LogicalTurn;
pub use kl_kindle::viewport::{FramebufferGeometry, ReadyViewport, Viewport};

pub(super) mod runtime;

/// A negotiated app can follow the inherited portrait turn or fix any
/// logical direction. The portrait hooks are used with an older Launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportRequest {
    FollowLauncher,
    Fixed(LogicalTurn),
}

impl ViewportRequest {
    pub(super) fn turn(self, inherited: crate::Rotation) -> LogicalTurn {
        match self {
            Self::FollowLauncher => inherited.into(),
            Self::Fixed(turn) => turn,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportDecision {
    Commit,
    Abort,
}

const NATIVE_SIZE: (u32, u32) = (profile::ACTIVE.screen_w, profile::ACTIVE.screen_h);
static PUBLISHING: Mutex<()> = Mutex::new(());

/// An explicitly negotiated, single foreground-run publisher. Use this for
/// every framebuffer write while the negotiated viewport owns the screen.
pub struct ViewportPublisher {
    run: PathBuf,
    path: PathBuf,
    latest: Option<Viewport>,
}

impl ViewportPublisher {
    /// `offered` is the supervisor's `KL_APP_VIEWPORT_FILE` for this run.
    pub fn new(run: &Path, offered: Option<PathBuf>) -> io::Result<Option<Self>> {
        let Some(path) = offered else { return Ok(None) };
        if !valid_viewport_offer(run, &path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid viewport offer",
            ));
        }
        Ok(Some(Self {
            run: run.to_owned(),
            path,
            latest: None,
        }))
    }

    /// Stage a pending record before presenting the frame. Return true only
    /// when a verified presentation was also committed as ready.
    ///
    /// An unsuccessful operation can leave the run pending, including when
    /// the frame was accepted but publishing ready failed. The caller can
    /// retry the same viewport or submit a newer generation.
    pub fn present(
        &mut self,
        display: &mut Display,
        viewport: Viewport,
        pixels: Vec<u8>,
        full: bool,
    ) -> io::Result<bool> {
        self.present_ready(display, viewport, pixels, full)
            .map(|ready| ready.is_some())
    }

    /// Return the exact ready record that was written after framebuffer readback.
    pub fn present_ready(
        &mut self,
        display: &mut Display,
        viewport: Viewport,
        pixels: Vec<u8>,
        full: bool,
    ) -> io::Result<Option<ReadyViewport>> {
        self.present_record_with(viewport, Path::new("/proc"), std::process::id(), || {
            display.present_view_verified(pixels, viewport.logical_size, viewport.turn, full)
        })
    }

    #[cfg(test)]
    fn present_with(
        &mut self,
        viewport: Viewport,
        procfs: &Path,
        pid: u32,
        draw: impl FnOnce() -> Option<FramebufferGeometry>,
    ) -> io::Result<bool> {
        self.present_record_with(viewport, procfs, pid, draw)
            .map(|ready| ready.is_some())
    }

    // The public method supplies the display readback. Tests can drive the
    // pending/ready transaction without opening a framebuffer.
    fn present_record_with(
        &mut self,
        viewport: Viewport,
        procfs: &Path,
        pid: u32,
        draw: impl FnOnce() -> Option<FramebufferGeometry>,
    ) -> io::Result<Option<ReadyViewport>> {
        if !ViewportTransform::new(NATIVE_SIZE, viewport.turn, 0)
            .is_some_and(|transform| transform.logical_size() == viewport.logical_size)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid viewport dimensions",
            ));
        }
        // Keep pending and ready ordered even if this process reconstructs its publisher.
        let _lock = PUBLISHING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let owner = self.owner(procfs, pid)?;
        let current = if let Some(record) = ViewportRecord::read(&self.path) {
            if record.owner() != &owner {
                return Err(owner_changed());
            }
            Some(record.viewport())
        } else {
            None
        };
        for previous in [self.latest, current].into_iter().flatten() {
            if previous.generation > viewport.generation
                || (previous.generation == viewport.generation && previous != viewport)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "stale viewport generation",
                ));
            }
        }

        let pending = ViewportRecord::Pending {
            owner: owner.clone(),
            viewport,
        };
        write_atomic(&self.path, pending.encode().as_bytes())?;
        self.latest = Some(viewport);
        if self.owner(procfs, pid)? != owner {
            return Err(owner_changed());
        }
        let Some(framebuffer) = draw() else {
            return Ok(None);
        };
        if framebuffer.native_size != NATIVE_SIZE || !viewport.matches(framebuffer) {
            return Ok(None);
        }
        if self.owner(procfs, pid)? != owner {
            return Err(owner_changed());
        }
        let ready = ReadyViewport {
            owner,
            viewport,
            framebuffer,
        };
        write_atomic(
            &self.path,
            ViewportRecord::Ready(ready.clone()).encode().as_bytes(),
        )?;
        Ok(Some(ready))
    }

    fn owner(&self, procfs: &Path, pid: u32) -> io::Result<ViewportOwner> {
        let live = live_viewport_owner(&self.run, procfs).ok_or_else(owner_changed)?;
        if live.owner.pid != pid || live.stopped || live.owner.viewport_path(&self.run) != self.path
        {
            return Err(owner_changed());
        }
        Ok(live.owner)
    }
}

fn owner_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "viewport offer requires the live foreground owner",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kl_kindle::viewport::read_ready_viewport;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    const PID: u32 = 43;
    const FB: FramebufferGeometry = FramebufferGeometry {
        native_size: NATIVE_SIZE,
        turn: 2,
        yoffset: 17,
        pitch: NATIVE_SIZE.0 + 16,
    };
    const LANDSCAPE: Viewport = Viewport {
        generation: 4,
        turn: LogicalTurn::Clockwise,
        logical_size: (NATIVE_SIZE.1, NATIVE_SIZE.0),
    };

    struct Run {
        root: PathBuf,
        record: PathBuf,
        procfs: PathBuf,
    }

    impl Run {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "viewport-publisher-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let procfs = root.join("proc");
            fs::create_dir_all(procfs.join(PID.to_string())).unwrap();
            let scope = root.join("app-turn.ABC123");
            fs::create_dir(&scope).unwrap();
            fs::write(root.join("app-turn-owner"), "v1 43 870 app-turn.ABC123\n").unwrap();
            fs::write(root.join("app.pid"), "43\n").unwrap();
            let run = Self {
                root,
                record: scope.join("viewport"),
                procfs,
            };
            run.stat(870);
            run
        }

        fn stat(&self, started: u64) {
            fs::write(
                self.procfs.join("43/stat"),
                format!("43 (Reader) S {}{started} 0\n", "0 ".repeat(18)),
            )
            .unwrap();
        }

        fn publisher(&self) -> ViewportPublisher {
            ViewportPublisher::new(&self.root, Some(self.record.clone()))
                .unwrap()
                .unwrap()
        }

        fn ready(&self) -> Option<ReadyViewport> {
            read_ready_viewport(&self.root, &self.procfs, FB, 0, false)
        }
    }

    impl Drop for Run {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn distinguishes_missing_offer_from_an_invalid_scope() {
        let run = Run::new();
        assert!(ViewportPublisher::new(&run.root, None).unwrap().is_none());
        assert!(matches!(
            ViewportPublisher::new(&run.root, Some(run.root.join("viewport"))),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput,
        ));
    }

    #[test]
    fn draw_is_bracketed_by_pending_and_ready_and_a_failed_draw_stays_pending() {
        let run = Run::new();
        let mut publisher = run.publisher();
        assert!(!publisher
            .present_with(LANDSCAPE, &run.procfs, PID, || {
                assert!(matches!(
                    ViewportRecord::read(&run.record),
                    Some(ViewportRecord::Pending {
                        viewport: LANDSCAPE,
                        ..
                    }),
                ));
                assert!(run.ready().is_none());
                None
            })
            .unwrap());
        assert!(run.ready().is_none());

        assert!(publisher
            .present_with(LANDSCAPE, &run.procfs, PID, || {
                assert!(run.ready().is_none());
                Some(FB)
            })
            .unwrap());
        assert_eq!(run.ready().unwrap().viewport, LANDSCAPE);
    }

    #[test]
    fn interrupted_ready_write_never_exposes_a_stale_frame() {
        let run = Run::new();
        let mut publisher = run.publisher();
        assert!(publisher
            .present_with(LANDSCAPE, &run.procfs, PID, || Some(FB))
            .unwrap());
        let next = Viewport {
            generation: 5,
            turn: LogicalTurn::Counterclockwise,
            ..LANDSCAPE
        };
        let temp = run.record.with_file_name("viewport.tmp");

        // Failure before pending cannot reach the draw, even with an older ready record.
        fs::create_dir(&temp).unwrap();
        assert!(publisher
            .present_with(next, &run.procfs, PID, || panic!("draw without pending"))
            .is_err());
        assert_eq!(run.ready().unwrap().viewport, LANDSCAPE);
        fs::remove_dir(&temp).unwrap();

        // Fail the atomic ready write after the framebuffer would have changed.
        assert!(publisher
            .present_with(next, &run.procfs, PID, || {
                assert!(run.ready().is_none());
                fs::create_dir(&temp).unwrap();
                Some(FB)
            })
            .is_err());
        assert_eq!(ViewportRecord::read(&run.record).unwrap().viewport(), next);
        assert!(run.ready().is_none());
        fs::remove_dir(&temp).unwrap();
        assert!(publisher
            .present_with(next, &run.procfs, PID, || Some(FB))
            .unwrap());
        assert_eq!(run.ready().unwrap().viewport, next);
    }

    #[test]
    fn stale_generation_and_replaced_owner_are_rejected_without_calling_draw() {
        let run = Run::new();
        let newer = Viewport {
            generation: 5,
            ..LANDSCAPE
        };
        assert!(run
            .publisher()
            .present_with(newer, &run.procfs, PID, || Some(FB))
            .unwrap());

        // A reconstructed publisher still checks the on-disk generation.
        let mut publisher = run.publisher();
        for stale in [
            LANDSCAPE,
            Viewport {
                turn: LogicalTurn::Counterclockwise,
                ..newer
            },
        ] {
            assert_eq!(
                publisher
                    .present_with(stale, &run.procfs, PID, || panic!("stale draw"))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(run.ready().unwrap().viewport, newer);
        assert_eq!(
            publisher
                .present_with(newer, &run.procfs, 44, || panic!("foreign writer"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied,
        );

        let next = Viewport {
            generation: 6,
            ..newer
        };
        assert_eq!(
            publisher
                .present_with(next, &run.procfs, PID, || {
                    run.stat(871);
                    Some(FB)
                })
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied,
        );
        assert!(run.ready().is_none());
        assert!(matches!(
            ViewportRecord::read(&run.record),
            Some(ViewportRecord::Pending { .. })
        ));
    }
}
