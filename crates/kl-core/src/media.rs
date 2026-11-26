//! The media session contract (docs/contracts.md §1.2): an app that plays audio in the background writes
//! `/var/tmp/media/<app>.now`, and quick settings shows its title and controls. Quick settings writes the commands to
//! `<app>.cmd`. Nothing here runs app code; these are the parts that read and write the two files.

/// A command quick settings sends to the app's player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    /// Play when paused, pause when playing.
    Toggle,
    Pause,
    Next,
    Previous,
}

impl MediaCommand {
    /// The line the app reads from its `.cmd` file.
    pub fn line(self) -> &'static str {
        match self {
            MediaCommand::Toggle => "toggle",
            MediaCommand::Pause => "pause",
            MediaCommand::Next => "next",
            MediaCommand::Previous => "previous",
        }
    }
}

/// An app's session as the `.now` file tells it: whether it plays (else it is paused), the track and the place.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaSession {
    /// The app's id: the file name's first part.
    pub app: String,
    /// The name shown when the track has none.
    pub name: String,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    /// Seconds into the track, and its length (0 when unknown).
    pub position: u64,
    pub length: u64,
    /// The player's process id: the session counts only while the process exists.
    pub pid: Option<u32>,
}

/// Whether an app id can name a session file (lower case letters, digits and `-`).
pub fn valid_app(app: &str) -> bool {
    !app.is_empty() && app.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Read a `.now` file. A stopped session, or one without an app or a state, gives `None`.
pub fn parse(text: &str) -> Option<MediaSession> {
    let mut session = MediaSession::default();
    let mut state = None;
    for line in text.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        let value = value.trim();
        match key {
            "app" => session.app = value.to_string(),
            "name" => session.name = value.to_string(),
            "state" => state = Some(value.to_string()),
            "title" => session.title = value.to_string(),
            "artist" => session.artist = value.to_string(),
            "position" => session.position = value.parse().unwrap_or(0),
            "length" => session.length = value.parse().unwrap_or(0),
            "pid" => session.pid = value.parse().ok(),
            _ => {}
        }
    }
    session.playing = match state.as_deref() {
        Some("playing") => true,
        Some("paused") => false,
        _ => return None,
    };
    if !valid_app(&session.app) {
        return None;
    }
    if session.name.is_empty() {
        session.name = session.app.clone();
    }
    Some(session)
}

/// The session quick settings shows: the first one that plays, else the first one that is paused.
pub fn choose(sessions: Vec<MediaSession>) -> Option<MediaSession> {
    sessions.iter().find(|s| s.playing).or(sessions.first()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYING: &str = "app music\nname Music\nstate playing\ntitle Evening Bells\nartist\nposition 83\nlength 203\npid 20561\n";

    #[test]
    fn a_session_is_read_from_its_file() {
        let session = parse(PLAYING).unwrap();
        assert_eq!(session.app, "music");
        assert_eq!(session.name, "Music");
        assert!(session.playing);
        assert_eq!(session.title, "Evening Bells");
        assert_eq!((session.position, session.length), (83, 203));
        assert_eq!(session.pid, Some(20561));
    }

    #[test]
    fn paused_sessions_count_and_stopped_ones_do_not() {
        let paused = parse(&PLAYING.replace("state playing", "state paused")).unwrap();
        assert!(!paused.playing);
        assert_eq!(parse(&PLAYING.replace("state playing", "state stopped")), None);
        assert_eq!(parse("app music\n"), None, "no state");
        assert_eq!(parse(&PLAYING.replace("app music", "app Music!")), None, "a bad app id");
    }

    #[test]
    fn the_playing_session_is_chosen_before_a_paused_one() {
        let paused = MediaSession { app: "reader".into(), playing: false, ..Default::default() };
        let playing = MediaSession { app: "music".into(), playing: true, ..Default::default() };
        assert_eq!(choose(vec![paused.clone(), playing.clone()]).unwrap().app, "music");
        assert_eq!(choose(vec![paused]).unwrap().app, "reader");
        assert_eq!(choose(vec![]), None);
    }

    #[test]
    fn commands_are_the_lines_of_the_contract() {
        assert_eq!(MediaCommand::Toggle.line(), "toggle");
        assert_eq!(MediaCommand::Previous.line(), "previous");
    }
}
