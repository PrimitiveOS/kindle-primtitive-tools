//! Opt-in contact movement, before the release gesture, in screen coordinates.
use std::time::Duration;

use crate::{turn_point, Input};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanPhase {
    Move,
    Release,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchPan {
    pub start: (f32, f32),
    pub end: (f32, f32),
    /// Time since the first detected movement in this contact.
    pub elapsed: Duration,
    pub phase: PanPhase,
}

impl TouchPan {
    pub fn swipe(self) -> Input {
        Input::Swipe {
            dx: self.end.0 - self.start.0,
            dy: self.end.1 - self.start.1,
            start_y: self.start.1,
        }
    }

    pub(crate) fn project(self, project: impl Fn((f32, f32)) -> (f32, f32)) -> Self {
        Self {
            start: project(self.start),
            end: project(self.end),
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContactEvent {
    Gesture(Input, f32),
    Pan(TouchPan),
}

pub fn legacy_contact_loop(
    path: String,
    generation: &crate::viewport::ViewportGeneration,
    mut report: impl FnMut(u64, ContactEvent),
) {
    use crate::{read_touch_stream, NativeContact, TURN};
    use std::sync::atomic::Ordering;

    read_touch_stream(
        path,
        || generation.current(),
        || TURN.load(Ordering::Relaxed),
        true,
        |event, generation, turn| {
            let project = |(x, y)| turn_point(turn, x, y);
            let event = match event {
                NativeContact::Gesture(touch) => {
                    let (input, origin) = touch.reduce(project);
                    ContactEvent::Gesture(input, origin.0)
                }
                NativeContact::Pan(pan) => ContactEvent::Pan(pan.project(project)),
            };
            report(generation, event);
        },
    );
}
