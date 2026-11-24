//! The e-ink display for the Kindle apps: one panel, with full updates on request and small updates otherwise.

use std::time::Instant;

use kl_core::Settings;

use primitive_device::{log, profile};

use crate as eink;

const W: u32 = profile::ACTIVE.screen_w;
const H: u32 = profile::ACTIVE.screen_h;

/// Shows frames on the panel: a full GC16 update on request, otherwise only the box around the changed
/// pixels, with a non-flashing waveform (DU for pure black and white, AUTO when icons bring gray). A tap on one
/// icon updates that icon, not the full-width band of its row.
pub struct Display {
    panel: Option<eink::Panel>,
    last: Vec<u8>,
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
        Display { panel, last: Vec::new(), presented_frame: false, paused: false }
    }

    /// Stop or resume drawing. Resuming takes what is on the panel now (an app's screen) as the last frame, so the
    /// next `show` updates only the box that changes, without a flash: quick settings over an app updates the panel's
    /// box, not the whole screen. Off the Kindle, or when the panel cannot be read, the next `show` is a full update.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.last = if paused { Vec::new() } else { self.snapshot().unwrap_or_default() };
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// What is on the panel now, in panel orientation, to put back with `restore`. `None` off the Kindle.
    pub fn snapshot(&self) -> Option<Vec<u8>> {
        self.panel.as_ref()?.snapshot(W, H)
    }

    /// Put a snapshot back (it draws even while paused). Only the box that differs from what is on the panel is
    /// updated, without a flash, when it is a pop-up's size (quick settings closing over an app). A change over most
    /// of the screen (the sleep picture going) gets one full update, as a calm update there leaves a ghost.
    pub fn restore(&mut self, frame: Vec<u8>) {
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
        let fb_turn = self.fb_turn();
        let frame = kl_render::for_panel(upright, settings.rotation, fb_turn);
        let Some(panel) = self.panel.as_mut() else {
            self.last = frame;
            return false;
        };
        let t = Instant::now();
        let result = if full || self.last.len() != frame.len() {
            panel.draw(W, (0, 0, W, H), &frame, eink::WAVE_GC16, true).map(|ok| (ok, "GC16 full", (0, 0, W, H)))
        } else if let Some((x, y, w, h)) = kl_render::changed_rect(&self.last, &frame, W as usize) {
            let region = (x as u32, y as u32, w as u32, h as u32);
            let wave = region_wave(&frame, region);
            let name = if wave == eink::WAVE_DU { "DU" } else { "AUTO" };
            panel.draw(W, region, &frame, wave, false).map(|ok| (ok, name, region))
        } else {
            Ok((true, "unchanged", (0, 0, 0, 0)))
        };
        match result {
            Ok((true, wave, (x, y, w, h))) => {
                self.presented_frame |= w > 0 && h > 0;
                self.last = frame;
                log(&format!("draw {wave} {w}x{h} at {x},{y} in {} ms (fb turn {fb_turn})", t.elapsed().as_millis()));
                self.presented_frame
            }
            Ok((false, ..)) => {
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

    /// Show the whole of `upright` without the flash of a full update (GC16 in partial mode). For a redraw the
    /// launcher asks for after a theme switch: the launcher has just flashed the screen, and a flash here would show
    /// the inverse of the new theme, which is the old one, for a moment.
    pub fn show_calm(&mut self, upright: Vec<u8>, settings: &Settings) {
        self.present_calm(upright, settings);
    }

    /// Present a calm full-screen frame and report whether the panel accepted it.
    pub fn present_calm(&mut self, upright: Vec<u8>, settings: &Settings) -> bool {
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
        self.last.clear();
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
}

