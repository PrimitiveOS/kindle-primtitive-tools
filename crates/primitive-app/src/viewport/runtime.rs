//! The app and its canvas adopt a viewport only after the publisher commits it.
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use kl_core::viewport::ViewportTransform;
use primitive_input::viewport::{ResolvedInput, ViewportEvent, ViewportGeneration};

use super::{
    FramebufferGeometry, LogicalTurn, ReadyViewport, Viewport, ViewportDecision, ViewportPublisher,
    ViewportRequest, NATIVE_SIZE,
};
use crate::{frame, App, Canvas, Display, Rotation};

pub(super) type PresentResult = io::Result<Option<ReadyViewport>>;

const RETRY_DELAY: Duration = Duration::from_millis(350);

pub(crate) struct Session {
    runtime: Runtime,
    publisher: ViewportPublisher,
    retry_at: Option<Instant>,
}

impl Session {
    pub(crate) fn new(publisher: ViewportPublisher) -> Self {
        Self {
            runtime: Runtime::default(),
            publisher,
            retry_at: None,
        }
    }

    pub(crate) fn input_generation(&self) -> Arc<ViewportGeneration> {
        self.runtime.input_generation()
    }

    pub(crate) fn resolve(
        &mut self,
        input: ViewportEvent,
        display: &Display,
    ) -> Option<ResolvedInput> {
        let was_ready = self.runtime.is_ready();
        let resolved = self
            .runtime
            .resolve_verified(input, || display.viewport_framebuffer());
        if was_ready && !self.runtime.is_ready() {
            self.retry_at = None;
        }
        resolved
    }

    pub(crate) fn invalidate(&mut self) {
        self.runtime.invalidate();
        self.retry_at = None;
    }

    pub(crate) fn deadline(&self, usb_drive: bool) -> Option<Instant> {
        if usb_drive && self.runtime.turn().is_none() {
            None
        } else {
            self.retry_at
        }
    }

    pub(crate) fn retry_due(&self) -> bool {
        self.retry_at.is_some_and(|retry| retry <= Instant::now())
    }

    pub(crate) fn render_app(
        &mut self,
        app: &mut impl App,
        canvas: &mut Canvas,
        display: &mut Display,
        fonts: &[String],
        dark: bool,
        inherited: Rotation,
        redraw: Option<bool>,
    ) -> io::Result<()> {
        let target = app
            .viewport_request()
            .unwrap_or(ViewportRequest::FollowLauncher)
            .turn(inherited);
        let active = self.runtime.turn();
        let ready = self.runtime.is_ready();
        let may_attempt = self.retry_at.is_none() || self.retry_due();
        let changing = active != Some(target);
        if may_attempt && changing && (ready || active.is_none()) {
            let outcome = self.runtime.transition(
                app,
                canvas,
                target,
                fonts,
                dark,
                |viewport, pixels, full| {
                    self.publisher
                        .present_ready(display, viewport, pixels, full)
                },
            );
            self.record(&outcome);
            return outcome;
        }
        if !(may_attempt && !ready && active.is_some() || ready && redraw.is_some()) {
            if ready && !changing {
                self.retry_at = None;
            }
            return Ok(());
        }
        let retry = self.retry_at.take();
        let outcome =
            self.present_current(display, frame(app, canvas, dark), redraw.unwrap_or(false));
        // A repaint can go through while a failed layout attempt backs off.
        // After recovery, revisit the requested turn without waiting for input.
        if outcome.is_ok() && changing {
            self.retry_at = if ready { retry } else { Some(Instant::now()) };
        }
        outcome
    }

    pub(crate) fn present_current(
        &mut self,
        display: &mut Display,
        pixels: Vec<u8>,
        full: bool,
    ) -> io::Result<()> {
        if self.runtime.turn().is_none() {
            return Ok(());
        }
        let outcome = self
            .runtime
            .present_current(pixels, full, |viewport, pixels, full| {
                self.publisher
                    .present_ready(display, viewport, pixels, full)
            });
        if outcome.is_err() || self.retry_due() {
            self.record(&outcome);
        }
        outcome
    }

    fn record(&mut self, outcome: &io::Result<()>) {
        self.retry_at = outcome.is_err().then(|| Instant::now() + RETRY_DELAY);
    }
}

#[derive(Default)]
pub(crate) struct Runtime {
    input: Arc<ViewportGeneration>,
    accepted: Option<Viewport>,
    ready: Option<ReadyViewport>,
}

impl Runtime {
    pub(crate) fn input_generation(&self) -> Arc<ViewportGeneration> {
        Arc::clone(&self.input)
    }

    pub(crate) fn turn(&self) -> Option<LogicalTurn> {
        self.accepted.map(|viewport| viewport.turn)
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.ready.is_some()
    }

    pub(crate) fn resolve(&self, input: ViewportEvent) -> Option<ResolvedInput> {
        let ready = self.ready.as_ref()?;
        (ready.viewport.generation == self.input.current())
            .then(|| input.resolve(&self.input, ready.viewport.turn))
            .flatten()
    }

    fn resolve_verified(
        &mut self,
        input: ViewportEvent,
        readback: impl FnOnce() -> Option<FramebufferGeometry>,
    ) -> Option<ResolvedInput> {
        let resolved = self.resolve(input)?;
        let expected = self.ready.as_ref().map(|ready| ready.framebuffer);
        if readback() != expected {
            self.invalidate();
            return None;
        }
        Some(resolved)
    }

    pub(crate) fn invalidate(&mut self) {
        self.ready = None;
        self.input.advance();
    }

    fn candidate(&self, turn: LogicalTurn) -> Viewport {
        // Contacts received during exposure carry the intervening epoch. They
        // must never be dispatched in the newly accepted viewport.
        Viewport {
            generation: self.input.current().wrapping_add(2),
            logical_size: ViewportTransform::new(NATIVE_SIZE, turn, 0)
                .expect("the native profile supports viewport projection")
                .logical_size(),
            turn,
        }
    }

    pub(crate) fn transition(
        &mut self,
        app: &mut impl App,
        canvas: &mut Canvas,
        turn: LogicalTurn,
        fonts: &[String],
        dark: bool,
        mut present: impl FnMut(Viewport, Vec<u8>, bool) -> PresentResult,
    ) -> io::Result<()> {
        let candidate = self.candidate(turn);
        let (width, height) = candidate.logical_size;
        let mut staged =
            Canvas::new_viewport(width, height, NATIVE_SIZE.0.min(NATIVE_SIZE.1), fonts)
                .map_err(io::Error::other)?;
        if let Err(error) = app.stage_viewport(candidate) {
            app.finish_viewport(candidate, ViewportDecision::Abort);
            return Err(error);
        }
        staged.begin();
        app.draw_staged_viewport(&mut staged);
        let pixels = staged.finish(dark);

        self.invalidate();
        match checked(candidate, present(candidate, pixels, true)) {
            Ok(ready) => {
                app.finish_viewport(candidate, ViewportDecision::Commit);
                *canvas = staged;
                self.accept(ready, true);
                Ok(())
            }
            Err(error) => {
                app.finish_viewport(candidate, ViewportDecision::Abort);
                if self.accepted.is_some() {
                    let prior = frame(app, canvas, dark);
                    if let Err(restore) = self.present_current(prior, true, &mut present) {
                        return Err(io::Error::other(format!(
                            "{error}. Restoring the previous viewport failed: {restore}"
                        )));
                    }
                }
                Err(error)
            }
        }
    }

    pub(crate) fn present_current(
        &mut self,
        image: Vec<u8>,
        full: bool,
        present: impl FnOnce(Viewport, Vec<u8>, bool) -> PresentResult,
    ) -> io::Result<()> {
        let Some(accepted) = self.accepted else {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "no accepted viewport",
            ));
        };
        let restoring = self.ready.is_none();
        let viewport = if restoring {
            let candidate = self.candidate(accepted.turn);
            self.invalidate();
            candidate
        } else {
            accepted
        };
        match checked(viewport, present(viewport, image, full || restoring)) {
            Ok(ready) => {
                self.accept(ready, restoring);
                Ok(())
            }
            Err(error) => {
                self.invalidate();
                Err(error)
            }
        }
    }

    fn accept(&mut self, ready: ReadyViewport, advance: bool) {
        if advance {
            let accepted = self.input.advance();
            debug_assert_eq!(ready.viewport.generation, accepted);
        }
        self.accepted = Some(ready.viewport);
        self.ready = Some(ready);
    }
}

fn checked(candidate: Viewport, result: PresentResult) -> io::Result<ReadyViewport> {
    result?
        .filter(|ready| {
            ready.viewport == candidate
                && ready.framebuffer.native_size == NATIVE_SIZE
                && candidate.matches(ready.framebuffer)
        })
        .ok_or_else(|| io::Error::other("viewport presentation was not verified"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Change, Input};
    use kl_kindle::viewport::ViewportOwner;
    use primitive_input::viewport::{NativeInput, NativeTouch, TouchPhase};
    use std::{cell::Cell, path::PathBuf, time::Duration};

    #[derive(Default)]
    struct Fake {
        accepted: Option<Viewport>,
        staged: Option<Viewport>,
        decisions: Vec<ViewportDecision>,
        fail_stage: bool,
        draws: Cell<usize>,
    }

    impl App for Fake {
        fn draw(&self, _canvas: &mut Canvas) {
            assert!(self.staged.is_none());
            self.draws.set(self.draws.get() + 1);
        }
        fn input(&mut self, _input: Input) -> Change {
            Change::None
        }
        fn stage_viewport(&mut self, candidate: Viewport) -> io::Result<()> {
            self.staged = Some(candidate);
            if self.fail_stage {
                Err(io::Error::other("staging failed"))
            } else {
                Ok(())
            }
        }
        fn draw_staged_viewport(&mut self, canvas: &mut Canvas) {
            assert_eq!(self.staged.unwrap().logical_size, size(canvas));
            let expected = (28.0 * f64::from(NATIVE_SIZE.0.min(NATIVE_SIZE.1)) / 1264.0).round();
            assert_eq!(canvas.px(28), expected as i32);
            canvas.fill(
                kl_core::Rect::new(canvas.bounds().w - 30, 4, 20, 10),
                primitive_ui::BLACK,
            );
        }
        fn finish_viewport(&mut self, candidate: Viewport, decision: ViewportDecision) {
            assert_eq!(self.staged.take(), Some(candidate));
            self.decisions.push(decision);
            if decision == ViewportDecision::Commit {
                self.accepted = Some(candidate);
            }
        }
    }

    fn fonts() -> Vec<String> {
        let directory =
            PathBuf::from(std::env::var_os("KL_TEST_FONTS").expect("supply Noto Sans fonts"));
        crate::font_paths(&directory)
    }

    fn size(canvas: &Canvas) -> (u32, u32) {
        let bounds = canvas.bounds();
        (bounds.w as u32, bounds.h as u32)
    }

    fn accepted(viewport: Viewport) -> ReadyViewport {
        ReadyViewport {
            viewport,
            owner: ViewportOwner::parse("v1 12 345 app-turn.ABC123").unwrap(),
            framebuffer: crate::viewport::FramebufferGeometry {
                native_size: NATIVE_SIZE,
                turn: 0,
                yoffset: 0,
                pitch: NATIVE_SIZE.0,
            },
        }
    }

    fn button(generation: u64) -> ViewportEvent {
        ViewportEvent {
            generation,
            native: NativeInput::PageKey(109),
        }
    }

    #[test]
    #[ignore = "Supply the workspace Noto Sans fonts in KL_TEST_FONTS"]
    fn stages_and_draws_then_adopts_only_the_matching_ready_generation() {
        let fonts = fonts();
        let mut runtime = Runtime::default();
        let generation = runtime.input_generation();
        let mut app = Fake::default();
        let mut canvas = Canvas::new(NATIVE_SIZE.0, NATIVE_SIZE.1, &fonts).unwrap();
        let mut during = 0;
        runtime
            .transition(
                &mut app,
                &mut canvas,
                LogicalTurn::Clockwise,
                &fonts,
                false,
                |view, pixels, full| {
                    assert_eq!(view.logical_size, (NATIVE_SIZE.1, NATIVE_SIZE.0));
                    assert_eq!(pixels.len(), (NATIVE_SIZE.0 * NATIVE_SIZE.1) as usize);
                    assert!(full);
                    let width = view.logical_size.0 as usize;
                    assert_eq!(pixels[4 * width + width - 30], 0);
                    assert_eq!(pixels[4 * width + width - 31], 255);
                    during = generation.current();
                    assert_ne!(during, view.generation);
                    Ok(Some(accepted(view)))
                },
            )
            .unwrap();
        let viewport = app.accepted.unwrap();
        assert_eq!(viewport.generation, generation.current());
        assert_eq!(size(&canvas), viewport.logical_size);
        assert!(runtime.resolve(button(during)).is_none());
        assert_eq!(
            runtime.resolve(button(viewport.generation)).unwrap().input,
            Input::PageBack
        );
        let event = ViewportEvent {
            generation: viewport.generation,
            native: NativeInput::Touch(NativeTouch {
                start: (11., 25.),
                end: (11., 25.),
                phase: TouchPhase::Finished(Duration::ZERO),
            }),
        };
        assert_eq!(
            runtime.resolve(event).unwrap().input,
            Input::Tap(NATIVE_SIZE.1 as f32 - 26., 11.)
        );
    }

    #[test]
    #[ignore = "Supply the workspace Noto Sans fonts in KL_TEST_FONTS"]
    fn rejects_stale_readback_and_rolls_back_a_partial_exposure_before_input() {
        let fonts = fonts();
        let mut runtime = Runtime::default();
        let generation = runtime.input_generation();
        let mut app = Fake::default();
        let mut canvas = Canvas::new(NATIVE_SIZE.0, NATIVE_SIZE.1, &fonts).unwrap();
        let present = |view, _, _| Ok(Some(accepted(view)));
        runtime
            .transition(
                &mut app,
                &mut canvas,
                LogicalTurn::Upright,
                &fonts,
                false,
                present,
            )
            .unwrap();
        let prior = app.accepted.unwrap();
        let mut attempted = Vec::new();
        assert!(runtime
            .transition(
                &mut app,
                &mut canvas,
                LogicalTurn::Clockwise,
                &fonts,
                false,
                |view, _, full| {
                    attempted.push((view, generation.current()));
                    assert!(full);
                    if view.turn == LogicalTurn::Clockwise {
                        // Pixels may already have changed, but this record belongs to the old view.
                        Ok(Some(accepted(prior)))
                    } else {
                        Ok(Some(accepted(view)))
                    }
                }
            )
            .is_err());
        assert_eq!(app.accepted, Some(prior));
        assert_eq!(
            app.decisions,
            [ViewportDecision::Commit, ViewportDecision::Abort]
        );
        assert_eq!(app.draws.get(), 1);
        assert_eq!(size(&canvas), NATIVE_SIZE);
        assert!(attempted[1].0.generation > attempted[0].0.generation);
        for epoch in [
            prior.generation,
            attempted[0].0.generation,
            attempted[0].1,
            attempted[1].1,
        ] {
            assert!(runtime.resolve(button(epoch)).is_none());
        }
        assert_eq!(
            runtime.resolve(button(generation.current())).unwrap().input,
            Input::PageForward
        );

        // A failed draw followed by a failed rollback parks input. A later full
        // redraw at the old dimensions can restore it without adopting the candidate.
        assert!(runtime
            .transition(
                &mut app,
                &mut canvas,
                LogicalTurn::UpsideDown,
                &fonts,
                false,
                |_, _, _| Err(io::Error::other("display update failed"))
            )
            .is_err());
        assert!(!runtime.is_ready());
        assert!(runtime.resolve(button(generation.current())).is_none());
        runtime
            .present_current(frame(&app, &mut canvas, false), false, present)
            .unwrap();
        assert_eq!(runtime.turn(), Some(LogicalTurn::Upright));
        assert!(runtime.is_ready());
        runtime
            .transition(
                &mut app,
                &mut canvas,
                LogicalTurn::UpsideDown,
                &fonts,
                false,
                present,
            )
            .unwrap();
        assert_eq!(app.accepted.unwrap().turn, LogicalTurn::UpsideDown);
    }

    #[test]
    #[ignore = "Supply the workspace Noto Sans fonts in KL_TEST_FONTS"]
    fn staging_failure_aborts_without_touching_the_accepted_frame_or_input() {
        let fonts = fonts();
        let mut runtime = Runtime::default();
        let mut app = Fake::default();
        let mut canvas = Canvas::new(NATIVE_SIZE.0, NATIVE_SIZE.1, &fonts).unwrap();
        runtime
            .transition(
                &mut app,
                &mut canvas,
                LogicalTurn::Upright,
                &fonts,
                false,
                |view, _, _| Ok(Some(accepted(view))),
            )
            .unwrap();
        let original = app.accepted.unwrap();
        app.fail_stage = true;
        assert!(runtime
            .transition(
                &mut app,
                &mut canvas,
                LogicalTurn::Clockwise,
                &fonts,
                false,
                |_, _, _| panic!("staging did not expose pixels")
            )
            .is_err());
        assert_eq!(app.accepted, Some(original));
        assert!(runtime.resolve(button(original.generation)).is_some());
    }

    #[test]
    fn input_is_parked_if_live_screen_geometry_no_longer_matches_readback() {
        let mut runtime = Runtime::default();
        let candidate = runtime.candidate(LogicalTurn::Clockwise);
        runtime.invalidate();
        runtime.accept(accepted(candidate), true);
        let event = button(candidate.generation);
        let geometry = accepted(candidate).framebuffer;
        assert!(runtime.resolve_verified(event, || Some(geometry)).is_some());
        let shifted = FramebufferGeometry {
            yoffset: geometry.yoffset + 1,
            ..geometry
        };
        assert!(runtime.resolve_verified(event, || Some(shifted)).is_none());
        assert!(!runtime.is_ready());
        assert!(runtime
            .resolve_verified(event, || panic!("stale input cannot sample the panel"))
            .is_none());
    }
}
