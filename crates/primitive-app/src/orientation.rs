use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{write_atomic, Rotation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Turn {
    pub rotation: Rotation,
    fixed: bool,
}

impl Turn {
    pub fn resolve(requested: Option<Rotation>, inherited: Rotation) -> Self {
        Self {
            rotation: requested.unwrap_or(inherited),
            fixed: requested.is_some(),
        }
    }
}

/// Track the panel outcome independently from the app's requested turn. The
/// supervisor supplies a path unique to this run when it supports the protocol.
pub(super) struct Publisher {
    path: Option<PathBuf>,
    visible: Option<Rotation>,
    written: Option<Turn>,
}

impl Publisher {
    pub fn new(run: &Path, offered: Option<PathBuf>) -> Self {
        let path = offered.filter(|path| {
            let Some(parent) = path.parent() else {
                return false;
            };
            let token = parent
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("app-turn."));
            path.file_name().is_some_and(|name| name == "turn")
                && parent.parent() == Some(run)
                && token.is_some_and(|token| {
                    token.len() == 6 && token.bytes().all(|b| b.is_ascii_alphanumeric())
                })
        });
        Self {
            path,
            visible: None,
            written: None,
        }
    }

    pub fn scoped(&self) -> bool {
        self.path.is_some()
    }

    pub fn needs_frame(&self, turn: Turn) -> bool {
        self.scoped() && self.visible != Some(turn.rotation)
    }

    pub fn presented(&mut self, turn: Turn, success: bool) {
        if success && self.scoped() {
            self.visible = Some(turn.rotation);
            self.update_policy(turn);
        }
    }

    /// A follow/fixed change can use the frame already displayed at this turn.
    pub fn update_policy(&mut self, turn: Turn) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        if self.visible != Some(turn.rotation) || self.written == Some(turn) {
            return;
        }
        let policy = if turn.fixed { "fixed" } else { "follow" };
        let record = format!("v1 {} {policy}\n", turn.rotation.letter());
        if write_atomic(path, record.as_bytes()).is_ok() {
            self.written = Some(turn);
        }
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Run(PathBuf);

    impl Run {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "primitive-turn-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn scoped(&self, token: &str) -> PathBuf {
            let path = self.0.join(format!("app-turn.{token}"));
            fs::create_dir(&path).unwrap();
            path.join("turn")
        }
    }

    impl Drop for Run {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_a_presented_frame_changes_the_reported_turn() {
        let run = Run::new();
        let path = run.scoped("ABC123");
        let mut publisher = Publisher::new(&run.0, Some(path.clone()));
        let up = Turn::resolve(None, Rotation::Upright);
        let down = Turn::resolve(Some(Rotation::UpsideDown), Rotation::Upright);

        publisher.update_policy(up);
        publisher.presented(up, false);
        assert!(!path.exists());
        assert!(publisher.needs_frame(up));
        publisher.presented(up, true);
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1 U follow\n");
        assert!(!publisher.needs_frame(up));

        // A failed calm redraw during wake or a failed USB frame keeps the
        // previous visible turn. Either successful path reports the new frame.
        publisher.update_policy(down);
        publisher.presented(down, false);
        assert!(publisher.needs_frame(down));
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1 U follow\n");
        publisher.presented(down, true);
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1 D fixed\n");
        assert!(!publisher.needs_frame(down));
    }

    #[test]
    fn same_turn_policy_changes_and_failed_record_writes_use_the_visible_frame() {
        let run = Run::new();
        let path = run.scoped("ABC123");
        let mut publisher = Publisher::new(&run.0, Some(path.clone()));
        let follow = Turn::resolve(None, Rotation::UpsideDown);
        let fixed = Turn::resolve(Some(Rotation::UpsideDown), Rotation::Upright);
        publisher.presented(follow, true);
        let temp = path.with_file_name("turn.tmp");
        fs::create_dir(&temp).unwrap();
        publisher.update_policy(fixed);
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1 D follow\n");
        assert!(!publisher.needs_frame(fixed));
        fs::remove_dir(&temp).unwrap();
        publisher.update_policy(fixed);
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1 D fixed\n");
        publisher.update_policy(follow);
        assert_eq!(fs::read_to_string(&path).unwrap(), "v1 D follow\n");
        assert!(!temp.exists());
    }

    #[test]
    fn scoped_runs_do_not_write_the_legacy_file_or_adopt_another_path() {
        let run = Run::new();
        let legacy = run.0.join("orientation");
        fs::write(&legacy, "U").unwrap();
        let turn = Turn::resolve(Some(Rotation::UpsideDown), Rotation::Upright);
        for invalid in [
            None,
            Some(legacy.clone()),
            Some(run.0.join("app-turn.longtoken/turn")),
        ] {
            let mut publisher = Publisher::new(&run.0, invalid);
            assert!(!publisher.scoped());
            publisher.presented(turn, true);
        }
        let old = run.scoped("ABC123");
        let next = run.scoped("DEF456");
        let mut earlier = Publisher::new(&run.0, Some(old.clone()));
        let mut later = Publisher::new(&run.0, Some(next.clone()));
        earlier.presented(turn, true);
        later.presented(turn, true);
        drop(earlier);
        assert!(!old.exists());
        assert_eq!(fs::read_to_string(&next).unwrap(), "v1 D fixed\n");
        assert_eq!(fs::read_to_string(legacy).unwrap(), "U");
    }
}
