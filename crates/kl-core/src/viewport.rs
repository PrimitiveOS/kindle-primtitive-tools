//! Logical drawing orientation over a portrait framebuffer.

use crate::{Rect, Rotation};

/// An app's view of the panel. The system's portrait preference remains a `Rotation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalTurn {
    Upright,
    Clockwise,
    UpsideDown,
    Counterclockwise,
}

impl From<Rotation> for LogicalTurn {
    fn from(rotation: Rotation) -> Self {
        match rotation {
            Rotation::Upright => Self::Upright,
            Rotation::UpsideDown => Self::UpsideDown,
        }
    }
}

impl LogicalTurn {
    fn quarter_turns(self) -> u32 {
        match self {
            Self::Upright => 0,
            Self::Clockwise => 1,
            Self::UpsideDown => 2,
            Self::Counterclockwise => 3,
        }
    }
}

/// Pixel and rectangle projection between a view and packed portrait memory.
/// The caller handles any bytes of padding in the native framebuffer rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportTransform {
    panel: (u32, u32),
    memory_turn: u32,
}

impl ViewportTransform {
    /// The supported firmware turns for the portrait buffer are 0 and 2.
    pub fn new(panel: (u32, u32), turn: LogicalTurn, framebuffer_turn: u32) -> Option<Self> {
        let (width, height) = panel;
        if !matches!(framebuffer_turn, 0 | 2)
            || width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
        {
            return None;
        }
        (width as usize).checked_mul(height as usize)?;
        let memory_turn = (framebuffer_turn + 4 - turn.quarter_turns()) % 4;
        Some(Self { panel, memory_turn })
    }

    pub fn logical_size(self) -> (u32, u32) {
        let (width, height) = self.panel;
        match self.memory_turn {
            1 | 3 => (height, width),
            _ => self.panel,
        }
    }

    /// Convert a tightly packed, eight-bit logical frame to packed panel pixels.
    pub fn project(self, logical: Vec<u8>) -> Option<Vec<u8>> {
        turn_pixels(logical, self.logical_size(), self.memory_turn)
    }

    /// Recover a logical frame from packed panel pixels, for example for an overlay backdrop.
    pub fn unproject(self, panel: Vec<u8>) -> Option<Vec<u8>> {
        turn_pixels(panel, self.panel, (4 - self.memory_turn) % 4)
    }

    /// Clip a logical rectangle to the visible frame, then project it to panel coordinates.
    /// The returned bounds are half-open and do not include e-ink update alignment.
    pub fn panel_rect(self, rect: Rect) -> Option<Rect> {
        let (width, height) = self.logical_size();
        let (x, y) = (i64::from(rect.x), i64::from(rect.y));
        let right = (x + i64::from(rect.w)).clamp(0, i64::from(width)) as i32;
        let bottom = (y + i64::from(rect.h)).clamp(0, i64::from(height)) as i32;
        let left = x.clamp(0, i64::from(width)) as i32;
        let top = y.clamp(0, i64::from(height)) as i32;
        let (w, h) = (right - left, bottom - top);
        if w <= 0 || h <= 0 {
            return None;
        }
        Some(match self.memory_turn {
            1 => Rect::new(height as i32 - bottom, left, h, w),
            2 => Rect::new(width as i32 - right, height as i32 - bottom, w, h),
            3 => Rect::new(top, width as i32 - right, h, w),
            _ => Rect::new(left, top, w, h),
        })
    }
}

fn turn_pixels(mut pixels: Vec<u8>, (width, height): (u32, u32), turn: u32) -> Option<Vec<u8>> {
    let (width, height) = (width as usize, height as usize);
    if pixels.len() != width * height {
        return None;
    }
    match turn {
        0 => Some(pixels),
        2 => {
            pixels.reverse();
            Some(pixels)
        }
        _ => {
            let mut rotated = vec![0; pixels.len()];
            // Write contiguous panel rows rather than writing a strided byte per logical pixel.
            for (y, row) in rotated.chunks_exact_mut(height).enumerate() {
                if turn == 1 {
                    for (x, pixel) in row.iter_mut().enumerate() {
                        *pixel = pixels[(height - 1 - x) * width + y];
                    }
                } else {
                    for (x, pixel) in row.iter_mut().enumerate() {
                        *pixel = pixels[x * width + width - 1 - y];
                    }
                }
            }
            Some(rotated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_and_recovers_stock_landscape_rasters_under_either_firmware_turn() {
        let logical: Vec<_> = (0..15).collect();
        for (turn, size, expected) in [
            (LogicalTurn::Upright, (3, 5), logical.clone()),
            (
                LogicalTurn::Clockwise,
                (5, 3),
                vec![4, 9, 14, 3, 8, 13, 2, 7, 12, 1, 6, 11, 0, 5, 10],
            ),
            (
                LogicalTurn::UpsideDown,
                (3, 5),
                logical.iter().copied().rev().collect(),
            ),
            (
                LogicalTurn::Counterclockwise,
                (5, 3),
                vec![10, 5, 0, 11, 6, 1, 12, 7, 2, 13, 8, 3, 14, 9, 4],
            ),
        ] {
            let transform = ViewportTransform::new((3, 5), turn, 0).unwrap();
            assert_eq!(transform.logical_size(), size);
            assert_eq!(transform.project(logical.clone()).unwrap(), expected);
            assert_eq!(transform.unproject(expected.clone()).unwrap(), logical);
            let flipped = ViewportTransform::new((3, 5), turn, 2).unwrap();
            let expected_flipped: Vec<_> = expected.into_iter().rev().collect();
            assert_eq!(flipped.project(logical.clone()).unwrap(), expected_flipped);
            assert_eq!(flipped.unproject(expected_flipped).unwrap(), logical);
        }
    }

    #[test]
    fn rectangle_projection_clips_even_extreme_bounds_and_matches_the_raster() {
        let requested = Rect::new(5, -9, i32::MAX, 14);
        for firmware in [0, 2] {
            for turn in [
                LogicalTurn::Upright,
                LogicalTurn::Clockwise,
                LogicalTurn::UpsideDown,
                LogicalTurn::Counterclockwise,
            ] {
                let transform = ViewportTransform::new((11, 15), turn, firmware).unwrap();
                let (w, h) = transform.logical_size();
                let mut pixels = vec![255; (w * h) as usize];
                for row in pixels.chunks_exact_mut(w as usize).take(5) {
                    row[5..].fill(0);
                }
                let projected = transform.project(pixels).unwrap();
                let rect = transform.panel_rect(requested).unwrap();
                for (index, pixel) in projected.into_iter().enumerate() {
                    let (x, y) = ((index % 11) as i32, (index / 11) as i32);
                    let inside =
                        x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom();
                    assert_eq!(pixel == 0, inside, "{turn:?}, firmware {firmware}");
                }
                assert!(transform.panel_rect(Rect::new(-10, 1, 9, 3)).is_none());
                assert!(transform.panel_rect(Rect::new(3, 2, -1, 3)).is_none());
            }
        }
    }

    #[test]
    fn rejects_invalid_sizes_and_lengths() {
        assert!(ViewportTransform::new((0, 5), LogicalTurn::Clockwise, 0).is_none());
        assert!(ViewportTransform::new((u32::MAX, 5), LogicalTurn::Clockwise, 0).is_none());
        assert!(ViewportTransform::new((3, 5), LogicalTurn::Clockwise, 1).is_none());
        assert!(ViewportTransform::new((3, 5), LogicalTurn::Clockwise, 3).is_none());
        let transform = ViewportTransform::new((3, 5), LogicalTurn::Clockwise, 0).unwrap();
        assert!(transform.project(vec![255; 14]).is_none());
        assert!(transform.unproject(vec![255; 16]).is_none());
    }
}
