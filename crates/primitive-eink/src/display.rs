//! The e-ink display for the Kindle apps: one panel, with full updates on request and small updates otherwise.

use std::time::Instant;

use kl_core::{
    viewport::{LogicalTurn, ViewportTransform},
    Rect, Settings,
};

use primitive_device::{log, profile};

use crate::{self as eink, FramebufferGeometry, PanelConfig, Region};

const W: u32 = profile::ACTIVE.screen_w;
const H: u32 = profile::ACTIVE.screen_h;
const PANEL_SIZE: (u32, u32) = (W, H);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewportFrame {
    transform: ViewportTransform,
    config: PanelConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameUpdate {
    Full,
    Partial(Region),
    Unchanged,
}

fn frame_update(last: &[u8], frame: &[u8], width: u32, force_full: bool) -> FrameUpdate {
    if force_full || last.len() != frame.len() {
        return FrameUpdate::Full;
    }
    match kl_render::changed_rect(last, frame, width as usize) {
        Some((x, y, w, h)) => FrameUpdate::Partial((x as u32, y as u32, w as u32, h as u32)),
        None => FrameUpdate::Unchanged,
    }
}

/// Shows frames on the panel: a full GC16 update on request, otherwise only the box around the changed
/// pixels, with a non-flashing waveform (DU for pure black and white, AUTO when icons bring gray). A tap on one
/// icon updates that icon, not the full-width band of its row.
pub struct Display {
    panel: Option<eink::Panel>,
    last: Vec<u8>,
    viewport_baseline: Option<ViewportFrame>,
    presented_frame: bool,
    /// While an app owns the screen the launcher draws nothing: `show` and `show_region` do nothing.
    paused: bool,
}

impl Display {
    pub fn open() -> Display {
        let panel = match eink::Panel::open() {
            Ok(p) => Some(p),
            Err(e) => {
                log(&format!("panel: {e}"));
                None
            }
        };
        Display {
            panel,
            last: Vec::new(),
            viewport_baseline: None,
            presented_frame: false,
            paused: false,
        }
    }

    /// Stop or resume drawing. Resuming takes what is on the panel now (an app's screen) as the last frame, so the
    /// next `show` updates only the box that changes, without a flash: quick settings over an app updates the panel's
    /// box, not the whole screen. Off the Kindle, or when the panel cannot be read, the next `show` is a full update.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.viewport_baseline = None;
        self.last = if paused { Vec::new() } else { self.snapshot().unwrap_or_default() };
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// What is on the panel now, in panel orientation, to put back with `restore`. `None` off the Kindle.
    pub fn snapshot(&self) -> Option<Vec<u8>> {
        self.panel.as_ref()?.snapshot(W, H)
    }

    /// The geometry of the current supported native framebuffer.
    pub fn viewport_framebuffer(&self) -> Option<FramebufferGeometry> {
        Some(
            self.panel
                .as_ref()?
                .viewport_config(PANEL_SIZE)
                .ok()??
                .geometry(),
        )
    }

    /// Capture packed native rows only when the observed framebuffer still matches.
    /// The caller must stop the foreground writer before relying on this snapshot.
    pub fn snapshot_viewport(&self, expected: FramebufferGeometry) -> Option<Vec<u8>> {
        if expected.native_size != PANEL_SIZE {
            return None;
        }
        self.panel.as_ref()?.snapshot_viewport(expected)
    }

    /// Restore packed native pixels only to the framebuffer from which they came.
    pub fn restore_viewport(&mut self, expected: FramebufferGeometry, frame: Vec<u8>) -> bool {
        if expected.native_size != PANEL_SIZE || frame.len() != W as usize * H as usize {
            return false;
        }
        let Some(panel) = self.panel.as_ref() else { return false };
        let Ok(Some(config)) = panel.viewport_config(PANEL_SIZE) else { return false };
        if config.geometry() != expected {
            return false;
        }
        let Some(current) = panel.snapshot_viewport(expected) else { return false };
        let update = match frame_update(&current, &frame, W, false) {
            FrameUpdate::Partial(region) if !restores_calmly(region) => FrameUpdate::Full,
            FrameUpdate::Unchanged => {
                let restored = panel.matches_viewport(config).unwrap_or(false);
                if restored {
                    self.clear_baseline();
                }
                return restored;
            }
            update => update,
        };
        let restored = self.present_packed(frame, update, config.rotation, Some(config));
        self.clear_baseline();
        restored
    }

    /// Put a snapshot back (it draws even while paused). Only the box that differs from what is on the panel is
    /// updated, without a flash, when it is a pop-up's size (quick settings closing over an app). A change over most
    /// of the screen (the sleep picture going) gets one full update, as a calm update there leaves a ghost.
    pub fn restore(&mut self, frame: Vec<u8>) {
        self.viewport_baseline = None;
        let Some(panel) = self.panel.as_mut() else { return };
        let changed = match panel.snapshot(W, H) {
            Some(now) if now.len() == frame.len() => kl_render::changed_rect(&now, &frame, W as usize),
            _ => Some((0, 0, W as usize, H as usize)),
        };
        let Some((x, y, w, h)) = changed else {
            self.last = frame;
            log("draw: the saved screen is back (unchanged)");
            return;
        };
        let region = (x as u32, y as u32, w as u32, h as u32);
        let calm = restores_calmly(region);
        let (wave, name) = if calm { (region_wave(&frame, region), "calm") } else { (eink::WAVE_GC16, "GC16 full") };
        let region = if calm { region } else { (0, 0, W, H) };
        match panel.draw(W, region, &frame, wave, !calm) {
            Ok(true) => {
                self.last = frame;
                log(&format!("draw: the saved screen is back ({name} {}x{} at {},{})", region.2, region.3, region.0, region.1));
            }
            Ok(false) => {
                self.last.clear();
                log("restore: the framebuffer is not in a drawable state");
            }
            Err(e) => {
                self.last.clear();
                log(&format!("restore: {e}"));
            }
        }
    }

    pub fn fb_turn(&self) -> u32 {
        self.panel.as_ref().map_or(0, |p| p.turn())
    }

    pub fn has_presented_frame(&self) -> bool {
        self.presented_frame
    }

    pub fn show(&mut self, upright: Vec<u8>, settings: &Settings, full: bool) {
        self.present(upright, settings, full);
    }

    /// Present a frame and report whether the panel accepted it.
    pub fn present(&mut self, upright: Vec<u8>, settings: &Settings, full: bool) -> bool {
        if self.paused {
            return false;
        }
        self.leave_viewport();
        let fb_turn = self.fb_turn();
        let frame = kl_render::for_panel(upright, settings.rotation, fb_turn);
        let update = frame_update(&self.last, &frame, W, full);
        self.present_packed(frame, update, fb_turn, None)
    }

    /// Present an explicitly sized logical frame through the portrait framebuffer.
    /// Returns false when the display is paused, the frame is malformed, or the
    /// current panel mode cannot safely use the portrait mapping.
    pub fn present_view(
        &mut self,
        pixels: Vec<u8>,
        logical_size: (u32, u32),
        turn: LogicalTurn,
        full: bool,
    ) -> bool {
        self.present_view_inner(pixels, logical_size, turn, full, false)
            .is_some()
    }

    /// Present a logical frame and report its measured framebuffer geometry.
    /// The geometry is rechecked after presentation before it may be advertised.
    pub fn present_view_verified(
        &mut self,
        pixels: Vec<u8>,
        logical_size: (u32, u32),
        turn: LogicalTurn,
        full: bool,
    ) -> Option<FramebufferGeometry> {
        self.present_view_inner(pixels, logical_size, turn, full, true)
    }

    fn present_view_inner(
        &mut self,
        pixels: Vec<u8>,
        logical_size: (u32, u32),
        turn: LogicalTurn,
        full: bool,
        verify_after: bool,
    ) -> Option<FramebufferGeometry> {
        let view = self.checked_viewport(logical_size, turn)?;
        let frame = view.transform.project(pixels)?;
        let update = self.viewport_update(view, &frame, full);
        if !self.present_packed(frame, update, view.config.rotation, Some(view.config)) {
            return None;
        }
        if verify_after
            && !self
                .panel
                .as_ref()
                .is_some_and(|panel| panel.matches_viewport(view.config).unwrap_or(false))
        {
            self.clear_baseline();
            return None;
        }
        self.viewport_baseline = Some(view);
        Some(view.config.geometry())
    }

    /// Present a clipped logical rectangle from a full logical frame. The
    /// rectangle is projected before aligning its columns to e-ink update bounds.
    /// As with `show_region`, the next complete presentation draws the full view.
    pub fn present_view_region(
        &mut self,
        pixels: Vec<u8>,
        logical_size: (u32, u32),
        turn: LogicalTurn,
        area: Rect,
    ) -> bool {
        self.present_view_region_inner(pixels, logical_size, turn, area, None)
    }

    /// Draw an overlay region only while its captured framebuffer still matches.
    pub fn present_view_region_matching(
        &mut self,
        pixels: Vec<u8>,
        logical_size: (u32, u32),
        turn: LogicalTurn,
        area: Rect,
        expected: FramebufferGeometry,
    ) -> bool {
        self.present_view_region_inner(pixels, logical_size, turn, area, Some(expected))
    }

    fn present_view_region_inner(
        &mut self,
        pixels: Vec<u8>,
        logical_size: (u32, u32),
        turn: LogicalTurn,
        area: Rect,
        expected: Option<FramebufferGeometry>,
    ) -> bool {
        let Some(view) = self.checked_viewport(logical_size, turn) else {
            return false;
        };
        if expected.is_some_and(|geometry| geometry != view.config.geometry()) {
            return false;
        }
        let Some(rect) = view.transform.panel_rect(area) else {
            return false;
        };
        let Some(frame) = view.transform.project(pixels) else {
            return false;
        };
        let region = aligned_region(rect, W);
        let result = self.present_packed(
            frame,
            FrameUpdate::Partial(region),
            view.config.rotation,
            Some(view.config),
        );
        self.clear_baseline();
        result
    }

    fn checked_viewport(
        &mut self,
        logical_size: (u32, u32),
        turn: LogicalTurn,
    ) -> Option<ViewportFrame> {
        if self.paused {
            return None;
        }
        let config = match self.panel.as_ref()?.viewport_config(PANEL_SIZE) {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.clear_baseline();
                return None;
            }
            Err(error) => {
                log(&format!("draw: {error}"));
                self.clear_baseline();
                return None;
            }
        };
        let transform = ViewportTransform::new(PANEL_SIZE, turn, config.rotation)?;
        (transform.logical_size() == logical_size).then_some(ViewportFrame { transform, config })
    }

    fn viewport_update(&self, view: ViewportFrame, frame: &[u8], full: bool) -> FrameUpdate {
        frame_update(
            &self.last,
            frame,
            W,
            full || self.viewport_baseline != Some(view),
        )
    }

    fn clear_baseline(&mut self) {
        self.last.clear();
        self.viewport_baseline = None;
    }

    fn leave_viewport(&mut self) {
        if self.viewport_baseline.is_some() {
            self.clear_baseline();
        }
    }

    fn present_packed(
        &mut self,
        frame: Vec<u8>,
        update: FrameUpdate,
        fb_turn: u32,
        config: Option<PanelConfig>,
    ) -> bool {
        let Some(panel) = self.panel.as_mut() else {
            self.last = frame;
            return false;
        };
        let t = Instant::now();
        let plan = match update {
            FrameUpdate::Full => Some(((0, 0, W, H), eink::WAVE_GC16, "GC16 full", true)),
            FrameUpdate::Partial(region) => {
                let wave = region_wave(&frame, region);
                let name = if wave == eink::WAVE_DU { "DU" } else { "AUTO" };
                Some((region, wave, name, false))
            }
            FrameUpdate::Unchanged => None,
        };
        let result = match plan {
            Some((region, wave, name, full)) => {
                let wrote = match config {
                    Some(expected) => panel.draw_view(expected, region, &frame, wave, full),
                    None => panel.draw(W, region, &frame, wave, full),
                };
                wrote.map(|ok| (ok, name, region))
            }
            None => match config {
                Some(expected) => panel.matches_viewport(expected),
                None => Ok(true),
            }
            .map(|ok| (ok, "unchanged", (0, 0, 0, 0))),
        };
        match result {
            Ok((true, wave, (x, y, w, h))) => {
                self.presented_frame |= w > 0 && h > 0;
                self.last = frame;
                log(&format!(
                    "draw {wave} {w}x{h} at {x},{y} in {} ms (fb turn {fb_turn})",
                    t.elapsed().as_millis()
                ));
                self.presented_frame
            }
            Ok((false, ..)) => {
                self.clear_baseline();
                log("draw: the framebuffer is not in a drawable state");
                false
            }
            Err(e) => {
                self.clear_baseline();
                log(&format!("draw: {e}"));
                false
            }
        }
    }

    /// Show the whole of `upright` without the flash of a full update (GC16 in partial mode). For a redraw the
    /// launcher asks for after a theme switch: the launcher has just flashed the screen, and a flash here would show
    /// the inverse of the new theme, which is the old one, for a moment.
    pub fn show_calm(&mut self, upright: Vec<u8>, settings: &Settings) {
        self.present_calm(upright, settings);
    }

    /// Present a calm full-screen frame and report whether the panel accepted it.
    pub fn present_calm(&mut self, upright: Vec<u8>, settings: &Settings) -> bool {
        self.leave_viewport();
        let fb_turn = self.fb_turn();
        let frame = kl_render::for_panel(upright, settings.rotation, fb_turn);
        let Some(panel) = self.panel.as_mut() else {
            self.last = frame;
            return false;
        };
        let t = Instant::now();
        match panel.draw(W, (0, 0, W, H), &frame, eink::WAVE_GC16, false) {
            Ok(true) => {
                self.presented_frame = true;
                self.last = frame;
                log(&format!("draw GC16 calm {W}x{H} in {} ms (fb turn {fb_turn})", t.elapsed().as_millis()));
                true
            }
            Ok(false) => {
                self.last.clear();
                log("draw: the framebuffer is not in a drawable state");
                false
            }
            Err(e) => {
                self.last.clear();
                log(&format!("draw: {e}"));
                false
            }
        }
    }

    /// Show only `area` of `upright` (a pop-up), leaving the rest of the panel as it is. The next `show` draws
    /// the whole screen, as what is on the panel is no longer known.
    pub fn show_region(&mut self, upright: Vec<u8>, area: kl_core::Rect, settings: &Settings) {
        if self.paused {
            return;
        }
        let fb_turn = self.fb_turn();
        let turned = (settings.rotation.turn() + fb_turn) % 4 == 2;
        let frame = kl_render::for_panel(upright, settings.rotation, fb_turn);
        self.clear_baseline();
        let Some(panel) = self.panel.as_mut() else { return };
        // Upside down, the area sits at the opposite corner of the panel.
        let (x, y) = if turned { (W as i32 - area.right(), H as i32 - area.bottom()) } else { (area.x, area.y) };
        let x0 = x.clamp(0, W as i32) as u32 / 8 * 8;
        let x1 = ((x + area.w).clamp(0, W as i32) as u32).div_ceil(8) * 8;
        let (y0, y1) = (y.clamp(0, H as i32) as u32, (y + area.h).clamp(0, H as i32) as u32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let region = (x0, y0, x1.min(W) - x0, y1 - y0);
        let wave = region_wave(&frame, region);
        if let Err(e) = panel.draw(W, region, &frame, wave, false) {
            log(&format!("draw: {e}"));
        }
    }
}

fn aligned_region(rect: Rect, panel_width: u32) -> Region {
    let left = rect.x as u32 / 8 * 8;
    let right = (rect.right() as u32).div_ceil(8) * 8;
    (left, rect.y as u32, right.min(panel_width) - left, rect.h as u32)
}

/// A restore of this box needs no flash: it is at most 60% of the screen (a pop-up, not the sleep picture).
fn restores_calmly((_, _, w, h): (u32, u32, u32, u32)) -> bool {
    (w as u64 * h as u64) * 10 <= (W as u64 * H as u64) * 6
}

fn region_wave(frame: &[u8], (x, y, w, h): (u32, u32, u32, u32)) -> u32 {
    let stride = W as usize;
    let (left, right) = (x as usize, (x + w) as usize);
    let black_and_white = (y as usize..(y + h) as usize)
        .all(|row| kl_render::is_black_and_white(&frame[row * stride + left..row * stride + right]));
    if black_and_white { eink::WAVE_DU } else { eink::WAVE_AUTO }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mapping, VarInfo};

    #[test]
    fn a_pop_up_is_put_back_calmly_and_the_whole_screen_with_a_flash() {
        assert!(restores_calmly((200, 40, 832, 900)), "quick settings");
        assert!(!restores_calmly((0, 0, W, H)), "the sleep picture");
        assert!(!restores_calmly((0, 0, W, H * 2 / 3)), "most of the screen");
    }

    #[test]
    fn rectangle_waveform_depends_only_on_the_region_pixels() {
        let mut frame = vec![255; W as usize * 2];
        frame[0] = 128;
        assert_eq!(region_wave(&frame, (8, 0, 8, 2)), eink::WAVE_DU);
        frame[W as usize + 8] = 128;
        assert_eq!(region_wave(&frame, (8, 0, 8, 2)), eink::WAVE_AUTO);
    }

    fn view(turn: LogicalTurn, rotation: u32, yoffset: u32) -> ViewportFrame {
        let mapping = Mapping {
            start: 0,
            len: (W + 16) as usize * (H + 8) as usize,
            pitch: (W + 16) as usize,
        };
        let var = VarInfo {
            xres: W,
            yres: H,
            bits_per_pixel: 8,
            rotate: rotation,
            yoffset,
            ..VarInfo::default()
        };
        ViewportFrame {
            transform: ViewportTransform::new(PANEL_SIZE, turn, rotation).unwrap(),
            config: PanelConfig::new(var, mapping, mapping, PANEL_SIZE).unwrap(),
        }
    }

    #[test]
    fn opting_in_to_a_portrait_turn_projects_the_same_pixels_as_the_legacy_path() {
        let pixels: Vec<u8> = (0..15).collect();
        for rotation in [kl_core::Rotation::Upright, kl_core::Rotation::UpsideDown] {
            for firmware in [0, 2] {
                let viewport = ViewportTransform::new((3, 5), rotation.into(), firmware).unwrap();
                let view = viewport.project(pixels.clone()).unwrap();
                let legacy = kl_render::for_panel(pixels.clone(), rotation, firmware);
                assert_eq!(view, legacy);
            }
        }
    }

    #[test]
    fn resizing_or_changing_the_framebuffer_discards_an_equal_pixel_baseline() {
        let original = view(LogicalTurn::Clockwise, 0, 0);
        let packed = vec![255; W as usize * H as usize];
        let mut display = Display {
            panel: None,
            last: packed.clone(),
            viewport_baseline: Some(original),
            presented_frame: true,
            paused: false,
        };
        assert_eq!(
            display.viewport_update(original, &packed, false),
            FrameUpdate::Unchanged
        );
        assert_eq!(
            display.viewport_update(original, &packed, true),
            FrameUpdate::Full
        );
        for different in [
            view(LogicalTurn::Upright, 0, 0),
            view(LogicalTurn::Counterclockwise, 0, 0),
            view(LogicalTurn::Clockwise, 2, 0),
            view(LogicalTurn::Clockwise, 0, 1),
        ] {
            assert_eq!(
                display.viewport_update(different, &packed, false),
                FrameUpdate::Full
            );
        }

        display.leave_viewport();
        assert!(display.last.is_empty());
        assert_eq!(
            display.viewport_update(original, &packed, false),
            FrameUpdate::Full
        );
        display.set_paused(true);
        assert!(!display.present_view(packed, (H, W), LogicalTurn::Clockwise, false));
        assert!(display.viewport_baseline.is_none());
    }

    #[test]
    fn clipped_landscape_dirty_rect_has_exact_physical_columns_and_padded_runs() {
        const WIDTH: u32 = 21;
        const HEIGHT: u32 = 23;
        const PITCH: usize = 32;
        let var = VarInfo {
            xres: WIDTH,
            yres: HEIGHT,
            bits_per_pixel: 8,
            yoffset: 2,
            ..VarInfo::default()
        };
        let mapping = Mapping {
            start: 0,
            len: PITCH * 28,
            pitch: PITCH,
        };
        let config = PanelConfig::new(var, mapping, mapping, (WIDTH, HEIGHT)).unwrap();
        let transform = ViewportTransform::new((WIDTH, HEIGHT), LogicalTurn::Clockwise, 0).unwrap();
        let before = vec![255; (WIDTH * HEIGHT) as usize];
        let mut logical = before.clone();
        for row in logical.chunks_exact_mut(HEIGHT as usize).skip(17).take(4) {
            row[..3].fill(0);
        }
        let packed = transform.project(logical).unwrap();

        let clip = transform.panel_rect(Rect::new(-2, 17, 5, 9)).unwrap();
        assert_eq!(clip, Rect::new(17, 20, 4, 3));
        let region = aligned_region(clip, WIDTH);
        assert_eq!(region, (16, 20, 5, 3));
        assert_eq!(
            frame_update(&before, &packed, WIDTH, false),
            FrameUpdate::Partial(region)
        );

        let mut mapped = vec![127; mapping.len];
        for (source, destination) in config
            .rows
            .runs(region, packed.len(), mapped.len())
            .unwrap()
        {
            let row = &packed[source];
            mapped[destination..destination + row.len()].copy_from_slice(row);
        }
        let mut expected = vec![127; mapping.len];
        for row in 22..25 {
            expected[row * PITCH + 16] = 255;
            expected[row * PITCH + 17..row * PITCH + 21].fill(0);
        }
        assert_eq!(mapped, expected);
    }
}
