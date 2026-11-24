//! Opt-in native input for a caller that presents more than two screen turns.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use kl_core::viewport::LogicalTurn;

use crate::{gesture, page_button_for, read_buttons, read_touches, Input, NATIVE_H, NATIVE_W};

/// Advance when the visible viewport or its owner changes. Share this instance
/// between the event loop and its input readers.
#[derive(Debug, Default)]
pub struct ViewportGeneration(AtomicU64);

impl ViewportGeneration {
    pub fn current(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    pub fn advance(&self) -> u64 {
        self.0.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }
}

/// Native touch coordinates are scaled to the upright panel's pixel range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeTouch {
    pub start: (f32, f32),
    pub end: (f32, f32),
    pub phase: TouchPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Finished(Duration),
    HoldStart,
    HoldPan,
    HoldRelease { held_ms: u32 },
}

impl NativeTouch {
    pub(super) fn reduce(self, project: impl Fn((f32, f32)) -> (f32, f32)) -> (Input, (f32, f32)) {
        let start = project(self.start);
        let end = project(self.end);
        let input = match self.phase {
            TouchPhase::Finished(held) => gesture(start, end, held),
            TouchPhase::HoldStart => Input::HoldStart(start.0, start.1),
            TouchPhase::HoldPan => Input::HoldPan(end.0, end.1),
            TouchPhase::HoldRelease { held_ms } => Input::HoldRelease {
                start_x: start.0,
                start_y: start.1,
                x: end.0,
                y: end.1,
                held_ms,
            },
        };
        (input, start)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeInput {
    Touch(NativeTouch),
    PageKey(u16),
}

/// A contact keeps its press generation through every hold sample and release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportEvent {
    pub generation: u64,
    pub native: NativeInput,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedInput {
    pub input: Input,
    pub touch_origin: Option<(f32, f32)>,
}

impl ViewportEvent {
    /// Resolve on the event loop using the visible viewport at dispatch time.
    /// A stale event is discarded before it can affect any hit region.
    pub fn resolve(
        self,
        generation: &ViewportGeneration,
        turn: LogicalTurn,
    ) -> Option<ResolvedInput> {
        if self.generation != generation.current() {
            return None;
        }
        match self.native {
            NativeInput::Touch(touch) => {
                let (input, start) = touch.reduce(|point| screen_point(turn, point));
                Some(ResolvedInput {
                    input,
                    touch_origin: Some(start),
                })
            }
            NativeInput::PageKey(code) => {
                let swapped = matches!(turn, LogicalTurn::Clockwise | LogicalTurn::UpsideDown);
                let input = page_button_for(code, swapped)?;
                Some(ResolvedInput {
                    input,
                    touch_origin: None,
                })
            }
        }
    }
}

fn screen_point(turn: LogicalTurn, (x, y): (f32, f32)) -> (f32, f32) {
    let (right, bottom) = (NATIVE_W as f32 - 1.0, NATIVE_H as f32 - 1.0);
    match turn {
        LogicalTurn::Upright => (x, y),
        LogicalTurn::Clockwise => (bottom - y, x),
        LogicalTurn::UpsideDown => (right - x, bottom - y),
        LogicalTurn::Counterclockwise => (y, right - x),
    }
}

pub fn viewport_touch_loop(
    path: String,
    generation: &ViewportGeneration,
    mut report: impl FnMut(ViewportEvent),
) {
    read_touches(
        path,
        || generation.current(),
        || 0,
        |touch, generation, _| {
            report(ViewportEvent {
                generation,
                native: NativeInput::Touch(touch),
            })
        },
    );
}

pub fn viewport_button_loop(
    path: String,
    generation: &ViewportGeneration,
    mut report: impl FnMut(ViewportEvent),
) {
    read_buttons(path, |code| {
        report(ViewportEvent {
            generation: generation.current(),
            native: NativeInput::PageKey(code),
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Contact, KEY_PAGEDOWN, KEY_PAGEUP};
    use kl_core::{viewport::ViewportTransform, Layout, Screen, Ui};
    use std::time::Instant;

    #[test]
    fn native_swipes_open_the_visible_top_edge_in_every_turn() {
        let generation = ViewportGeneration::default();
        let right = NATIVE_W as f32 - 1.0;
        let bottom = NATIVE_H as f32 - 1.0;
        for (turn, start, end, projected_x) in [
            (LogicalTurn::Upright, (200.0, 12.0), (200.0, 350.0), 200.0),
            (
                LogicalTurn::Clockwise,
                (12.0, 160.0),
                (350.0, 160.0),
                bottom - 160.0,
            ),
            (
                LogicalTurn::UpsideDown,
                (200.0, bottom - 12.0),
                (200.0, bottom - 350.0),
                right - 200.0,
            ),
            (
                LogicalTurn::Counterclockwise,
                (right - 12.0, 160.0),
                (right - 350.0, 160.0),
                160.0,
            ),
        ] {
            let view = ViewportTransform::new((NATIVE_W, NATIVE_H), turn, 0).unwrap();
            let (w, h) = view.logical_size();
            let mut ui = Ui::new(Vec::new(), Layout::new(w, h, 4, 5));
            let resolve = |start, end| {
                ViewportEvent {
                    generation: generation.current(),
                    native: NativeInput::Touch(NativeTouch {
                        start,
                        end,
                        phase: TouchPhase::Finished(Duration::from_millis(180)),
                    }),
                }
                .resolve(&generation, turn)
                .unwrap()
            };

            let native_bottom = (right - start.0, bottom - start.1);
            let native_inside = (right - end.0, bottom - end.1);
            ui.handle(resolve(native_bottom, native_inside).input);
            assert_eq!(ui.screen, Screen::Home, "{turn:?}");

            let gesture = resolve(start, end);
            assert_eq!(gesture.touch_origin, Some((projected_x, 12.0)));
            assert_eq!(
                gesture.input,
                Input::Swipe {
                    dx: 0.0,
                    dy: 338.0,
                    start_y: 12.0
                }
            );
            ui.handle(gesture.input);
            assert_eq!(ui.screen, Screen::QuickSettings, "{turn:?}");
        }
    }

    #[test]
    fn resizes_drop_a_contact_started_before_its_first_sample_and_each_hold_phase() {
        let turn = LogicalTurn::Clockwise;
        let generation = ViewportGeneration::default();
        let pressed = Instant::now();
        let mut old = Contact::new(pressed, generation.current());
        assert_eq!(generation.advance(), 1);
        let mut events = Vec::new();
        let mut report = |touch, generation, _| {
            events.push(ViewportEvent {
                generation,
                native: NativeInput::Touch(touch),
            })
        };
        old.sample((18.0, 245.0), pressed, || 0, &mut report);
        old.idle(pressed + Duration::from_millis(800), &mut report);
        assert_eq!(generation.advance(), 2);
        old.sample(
            (315.0, 207.0),
            pressed + Duration::from_millis(1_100),
            || unreachable!("only the first sample latches a turn"),
            &mut report,
        );
        old.finish(pressed + Duration::from_millis(1_200), &mut report);
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|event| event.generation == 0));
        assert!(events
            .iter()
            .all(|event| event.resolve(&generation, turn).is_none()));
        assert!(matches!(
            events[0].native,
            NativeInput::Touch(NativeTouch {
                phase: TouchPhase::HoldStart,
                ..
            })
        ));
        assert_eq!(
            events[1].native,
            NativeInput::Touch(NativeTouch {
                start: (18.0, 245.0),
                end: (315.0, 207.0),
                phase: TouchPhase::HoldPan,
            })
        );
        assert_eq!(
            events[2].native,
            NativeInput::Touch(NativeTouch {
                start: (18.0, 245.0),
                end: (315.0, 207.0),
                phase: TouchPhase::HoldRelease { held_ms: 1_200 },
            })
        );

        let fresh_at = pressed + Duration::from_millis(1_300);
        let mut fresh = Contact::new(fresh_at, generation.current());
        let mut resolved = Vec::new();
        let mut report = |touch, generation_at_press, _| {
            let event = ViewportEvent {
                generation: generation_at_press,
                native: NativeInput::Touch(touch),
            };
            resolved.push(event.resolve(&generation, turn).unwrap());
        };
        fresh.sample((18.0, 245.0), fresh_at, || 0, &mut report);
        fresh.idle(fresh_at + Duration::from_millis(800), &mut report);
        fresh.sample(
            (315.0, 207.0),
            fresh_at + Duration::from_millis(1_100),
            || 0,
            &mut report,
        );
        fresh.finish(fresh_at + Duration::from_millis(1_200), &mut report);
        let first_x = NATIVE_H as f32 - 1.0 - 245.0;
        let last_x = NATIVE_H as f32 - 1.0 - 207.0;
        assert_eq!(
            resolved.iter().map(|event| event.input).collect::<Vec<_>>(),
            [
                Input::HoldStart(first_x, 18.0),
                Input::HoldPan(last_x, 315.0),
                Input::HoldRelease {
                    start_x: first_x,
                    start_y: 18.0,
                    x: last_x,
                    y: 315.0,
                    held_ms: 1_200
                },
            ]
        );
        assert!(resolved
            .iter()
            .all(|event| event.touch_origin == Some((first_x, 18.0))));
    }

    #[test]
    fn both_page_codes_follow_logical_turn_and_ignore_the_framebuffer_turn() {
        let generation = ViewportGeneration::default();
        let resolve = |generation_at_press, code, turn| {
            ViewportEvent {
                generation: generation_at_press,
                native: NativeInput::PageKey(code),
            }
            .resolve(&generation, turn)
            .map(|event| event.input)
        };
        for (turn, back, forward) in [
            (LogicalTurn::Upright, KEY_PAGEUP, KEY_PAGEDOWN),
            (LogicalTurn::Clockwise, KEY_PAGEDOWN, KEY_PAGEUP),
            (LogicalTurn::UpsideDown, KEY_PAGEDOWN, KEY_PAGEUP),
            (LogicalTurn::Counterclockwise, KEY_PAGEUP, KEY_PAGEDOWN),
        ] {
            for firmware in [0, 2] {
                // A key is resolved for the visible logical turn. Firmware
                // affects only this viewport's raster projection.
                let view = ViewportTransform::new((NATIVE_W, NATIVE_H), turn, firmware).unwrap();
                assert!(view.logical_size().0 > 0);
                let current = generation.current();
                assert_eq!(resolve(current, back, turn), Some(Input::PageBack));
                assert_eq!(resolve(current, forward, turn), Some(Input::PageForward));
                assert_eq!(resolve(current, 1, turn), None);
                generation.advance();
                assert_eq!(resolve(current, back, turn), None);
                assert_eq!(resolve(current, forward, turn), None);
            }
        }
    }
}
