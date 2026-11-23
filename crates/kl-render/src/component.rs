//! Shared drawing primitives for apps with their own screens.

pub use super::image::GrayImage;
use super::{gray, Align as RenderAlign, Renderer, Text, BLACK, WHITE};
use kl_core::{Layout, Rect, Status};
use parley::{Affinity, Cursor};
use primitive_keyboard::Editor;
pub use primitive_quick::{icons, Icon};

struct Picture {
    rect: Rect,
    image: GrayImage,
    after_popups: usize,
}

enum TouchRegion {
    Control(Rect),
    Cover(Rect),
}

impl TouchRegion {
    fn bounds(&self) -> Rect {
        match *self {
            Self::Control(rect) | Self::Cover(rect) => rect,
        }
    }
}

/// Text alignment inside its bounds.
#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Draws a reusable component screen with the launcher's font and pixel pipeline.
pub struct Canvas {
    renderer: Renderer,
    bounds: Rect,
    pictures: Vec<Picture>,
    popup_covers: Vec<(Rect, f32)>,
    touch_regions: Vec<TouchRegion>,
    pressed: Option<usize>,
}

impl Canvas {
    pub fn new(width: u32, height: u32, fonts: &[String]) -> Result<Self, String> {
        Ok(Self {
            renderer: Renderer::new(width, height, fonts)?,
            bounds: Rect::new(0, 0, width as i32, height as i32),
            pictures: Vec::new(),
            popup_covers: Vec::new(),
            touch_regions: Vec::new(),
            pressed: None,
        })
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Return the upright USB drive screen without accessing user storage.
    pub fn usb_drive_frame(&mut self) -> Option<Vec<u8>> {
        self.renderer.usb_drive_frame(self.bounds.w as u32, self.bounds.h as u32)
    }

    /// Scale a measurement from the reference width of 1264 pixels.
    pub fn px(&self, pixels: i32) -> i32 {
        ((pixels as f64 * f64::from(self.bounds.w)) / 1264.0).round() as i32
    }

    pub fn begin(&mut self) {
        self.renderer.ctx.reset();
        self.renderer.fill(self.bounds, WHITE);
        self.pictures.clear();
        self.popup_covers.clear();
        self.touch_regions.clear();
    }

    pub fn finish(&mut self, dark: bool) -> Vec<u8> {
        let width = self.bounds.w as usize;
        let mut image = self.renderer.finish_gray();
        if dark {
            super::image::invert_except(&mut image, width, &[], None);
        }
        for picture in &self.pictures {
            blit_picture(
                &mut image,
                self.bounds,
                picture,
                &self.popup_covers[picture.after_popups..],
            );
        }
        image
    }

    pub fn fill(&mut self, rect: Rect, ink: u8) {
        self.renderer.fill(rect, ink);
    }

    pub fn circle(&mut self, rect: Rect, ink: u8) {
        self.renderer
            .rounded(rect, f64::from(rect.h.min(rect.w)) / 2.0, ink);
    }

    pub fn card(&mut self, rect: Rect) {
        self.panel(rect, 22, 2, WHITE);
    }

    pub fn popup(&mut self, rect: Rect) {
        // The modal now receives taps, including taps outside its frame that dismiss it.
        self.touch_regions.push(TouchRegion::Cover(self.bounds));
        let halo = self.px(10);
        let cover = rect.inset(-halo);
        let radius = self.px(38);
        self.popup_covers.push((cover, radius as f32));
        self.renderer.rounded(cover, f64::from(radius), WHITE);
        self.panel(rect, 28, 6, WHITE);
    }

    /// The same clock, connectivity and battery strip as the launcher.
    pub fn status_bar(&mut self, status: &Status, running: usize) {
        let layout = Layout::new(self.bounds.w as u32, self.bounds.h as u32, 4, 5);
        self.renderer.status_strip(&layout, status, running);
    }

    /// An outline with semicircular ends.
    pub fn pill(&mut self, rect: Rect, border: i32, inside: u8) {
        self.renderer.rounded_frame(
            rect,
            f64::from(rect.h.min(rect.w)) / 2.0,
            self.px(border).max(1),
            BLACK,
            inside,
        );
    }

    pub fn panel(&mut self, rect: Rect, radius: i32, border: i32, inside: u8) {
        self.renderer.rounded_frame(
            rect,
            f64::from(self.px(radius)),
            self.px(border).max(1),
            BLACK,
            inside,
        );
    }

    pub fn text(&mut self, rect: Rect, content: &str, style: TextStyle) {
        self.draw_text(rect, content, style, None);
    }

    /// Draw a bold UTF-8 byte range in the same shaped, wrapped paragraph.
    /// Invalid or empty ranges use the unmodified text style.
    pub fn emphasized_text(
        &mut self,
        rect: Rect,
        content: &str,
        style: TextStyle,
        emphasis: std::ops::Range<usize>,
    ) {
        let range = (!emphasis.is_empty() && content.get(emphasis.clone()).is_some())
            .then_some(emphasis);
        self.draw_text(rect, content, style, range);
    }

    fn draw_text(
        &mut self,
        rect: Rect,
        content: &str,
        style: TextStyle,
        emphasis: Option<std::ops::Range<usize>>,
    ) {
        if rect.w <= 0 || rect.h <= 0 || style.lines == 0 {
            return;
        }
        let align = match style.align {
            Align::Left => RenderAlign::Start,
            Align::Center => RenderAlign::Center,
            Align::Right => RenderAlign::End,
        };
        let clip = super::kurbo::Shape::to_path(&super::krect(rect), 0.1);
        self.renderer.ctx.set_transform(super::Affine::IDENTITY);
        self.renderer.ctx.push_clip_layer(&clip);
        self.renderer.text_with_emphasis(
            Text {
                text: content,
                x: rect.x as f32,
                y: rect.y as f32,
                w: rect.w as f32,
                size: self.px(style.size).max(1) as f32,
                bold: style.bold,
                align,
                lines: style.lines,
                gray: style.ink,
            },
            style.overflow,
            emphasis,
        );
        self.renderer.ctx.pop_layer();
    }

    pub fn measure(&mut self, content: &str, size: i32) -> i32 {
        self.measure_with_weight(content, size, false)
    }

    /// Measure text at the same size and weight used by `text`.
    pub fn measure_with_weight(&mut self, content: &str, size: i32, bold: bool) -> i32 {
        self.renderer
            .text_width(content, self.px(size).max(1) as f32, bold)
            .ceil() as i32
    }

    /// Byte ranges for pages using the same line breaks as `text`.
    pub fn text_pages(
        &mut self,
        rect: Rect,
        content: &str,
        style: TextStyle,
    ) -> Vec<std::ops::Range<usize>> {
        if rect.w <= 0 || content.is_empty() || style.lines == 0 {
            return Vec::new();
        }
        let layout = self.renderer.layout_with_wrap(
            content,
            self.px(style.size).max(1) as f32,
            style.bold,
            rect.w as f32,
            style.overflow,
        );
        let mut starts = layout
            .lines()
            .step_by(style.lines)
            .map(|line| line.text_range().start)
            .peekable();
        let mut pages = Vec::new();
        while let Some(start) = starts.next() {
            let end = starts.peek().copied().unwrap_or(content.len());
            if start < end {
                pages.push(start..end);
            }
        }
        pages
    }

    /// Draw an editable single line, keeping the editor's caret in view. Masked fields show one bullet per character.
    pub fn single_line_field(
        &mut self,
        rect: Rect,
        editor: &Editor,
        placeholder: &str,
        masked: bool,
    ) {
        self.renderer
            .single_line_field(rect, editor, placeholder, masked);
    }

    pub fn button(&mut self, rect: Rect, label: &str, primary: bool, pressed: bool) {
        let filled = primary != self.control_pressed(rect, pressed);
        self.panel(rect, 16, 4, if filled { BLACK } else { WHITE });
        let top = (rect.h - self.px(45)).max(0) / 2;
        self.text(
            Rect::new(
                rect.x + self.px(8),
                rect.y + top,
                rect.w - self.px(16),
                rect.h - top,
            ),
            label,
            TextStyle::new(36)
                .bold()
                .center()
                .ink(if filled { WHITE } else { BLACK }),
        );
    }

    /// Register a manually drawn control and return its pressed style for the current frame.
    /// Standard buttons, keyboard keys, and shared widgets register their own regions.
    pub fn control_pressed(&mut self, rect: Rect, pressed: bool) -> bool {
        let index = self.touch_regions.len();
        self.touch_regions.push(TouchRegion::Control(rect));
        pressed || self.pressed == Some(index)
    }

    /// Read the live feedback state after a shared widget has drawn its controls.
    pub fn is_control_pressed(&self, rect: Rect) -> bool {
        self.pressed
            .and_then(|index| self.touch_regions.get(index))
            .is_some_and(|target| matches!(target, TouchRegion::Control(area) if *area == rect))
    }

    /// Select the topmost visible control from the previous draw for a pre-action feedback frame.
    pub fn press_at(&mut self, x: f32, y: f32) -> bool {
        self.pressed = self
            .touch_regions
            .iter()
            .rposition(|region| region.bounds().contains(x, y))
            .filter(|&index| matches!(self.touch_regions[index], TouchRegion::Control(_)));
        self.pressed.is_some()
    }

    pub fn clear_press(&mut self) {
        self.pressed = None;
    }

    pub fn icon(&mut self, rect: Rect, icon: Icon, ink: u8) {
        self.renderer.ctx.set_paint(gray(ink));
        self.renderer.draw_icon(
            icon,
            rect.x.into(),
            rect.y.into(),
            rect.w.min(rect.h).into(),
            ink,
        );
    }

    pub fn picture(&mut self, rect: Rect, picture: GrayImage) {
        self.pictures.push(Picture {
            rect,
            image: picture,
            after_popups: self.popup_covers.len(),
        });
    }

    pub fn keyboard(
        &mut self,
        layer: primitive_keyboard::Layer,
        done: &str,
        pressed: Option<Rect>,
    ) {
        let layout = Layout::new(self.bounds.w as u32, self.bounds.h as u32, 4, 5);
        let band = layout.keyboard_area();
        self.touch_regions.push(TouchRegion::Cover(band));
        self.fill(band, WHITE);
        self.fill(Rect::new(band.x, band.y, band.w, 2), 226);
        for (key, rect) in layout.keyboard_keys_with(layer, done) {
            let active = self.control_pressed(rect, pressed == Some(rect));
            self.renderer.key(&key, rect, active);
        }
    }
}

impl Renderer {
    pub(super) fn single_line_field(
        &mut self,
        field: Rect,
        editor: &Editor,
        placeholder: &str,
        masked: bool,
    ) {
        if field.w <= 0 || field.h <= 0 {
            return;
        }
        // The launcher's 100-pixel field supplies the proportions for smaller screens.
        let scale = |measurement| (field.h as f32 * measurement / 100.0) as i32;
        let border = scale(3.0).max(1);
        self.rounded_frame(field, f64::from(scale(20.0)), border, BLACK, WHITE);
        let padding = scale(28.0).min(field.w / 4);
        let inner = Rect::new(
            field.x + padding,
            field.y + border,
            field.w - 2 * padding,
            field.h - 2 * border,
        );
        if inner.w < border || inner.h <= 0 {
            return;
        }
        let clip = super::kurbo::Shape::to_path(&super::krect(inner), 0.1);
        self.ctx.set_transform(super::Affine::IDENTITY);
        self.ctx.push_clip_layer(&clip);
        let (shown, caret) = single_line_content(editor, masked);
        let mut text = Text {
            text: &shown,
            x: inner.x as f32,
            y: (field.y + scale(30.0)) as f32,
            w: inner.w as f32,
            size: scale(40.0).max(1) as f32,
            bold: false,
            align: RenderAlign::Start,
            lines: 1,
            gray: BLACK,
        };
        if shown.is_empty() {
            text.text = placeholder;
            text.gray = 120;
            self.text(text);
        } else {
            let layout = self.layout_text(&shown, text.size, false, f32::MAX);
            let caret_x = Cursor::from_byte_index(&layout, caret, Affinity::Downstream)
                .geometry(&layout, 0.0)
                .x0;
            let scroll = horizontal_scroll(caret_x, inner.w - border);
            text.x -= scroll as f32;
            self.draw_layout(&layout, &text);
            self.fill(
                Rect::new(
                    inner.x + caret_x as i32 - scroll,
                    field.y + scale(22.0),
                    border,
                    scale(56.0),
                ),
                BLACK,
            );
        }
        self.ctx.pop_layer();
    }
}

fn single_line_content(editor: &Editor, masked: bool) -> (String, usize) {
    let shown = if masked {
        "•".repeat(editor.text.len())
    } else {
        editor
            .text
            .iter()
            .map(|&character| match character {
                '\r' | '\n' | '\u{2028}' | '\u{2029}' => ' ',
                _ => character,
            })
            .collect()
    };
    let caret = shown
        .char_indices()
        .nth(editor.caret)
        .map_or(shown.len(), |(index, _)| index);
    (shown, caret)
}

fn horizontal_scroll(caret_x: f64, visible_width: i32) -> i32 {
    (caret_x as i32 - visible_width).max(0)
}

fn blit_picture(frame: &mut [u8], bounds: Rect, picture: &Picture, covered_by: &[(Rect, f32)]) {
    let Picture { rect, image, .. } = picture;
    if u64::from(image.width) * u64::from(image.height) != image.pixels.len() as u64 {
        return;
    }
    let (Ok(width), Ok(height)) = (i32::try_from(image.width), i32::try_from(image.height)) else {
        return;
    };
    let x_start = rect.x.max(0);
    let x_end = rect.x.saturating_add(rect.w.min(width)).min(bounds.w);
    if x_start >= x_end {
        return;
    }
    let y_end = rect.y.saturating_add(rect.h.min(height)).min(bounds.h);
    let stride = bounds.w as usize;
    for y in rect.y.max(0)..y_end {
        let src_start = (y - rect.y) as usize * width as usize + (x_start - rect.x) as usize;
        let dst_start = y as usize * stride + x_start as usize;
        let len = (x_end - x_start) as usize;
        let source = &image.pixels[src_start..src_start + len];
        let target = &mut frame[dst_start..dst_start + len];
        if covered_by.is_empty() {
            target.copy_from_slice(source);
            continue;
        }
        for (offset, (&pixel, target)) in source.iter().zip(target).enumerate() {
            let x = x_start + offset as i32;
            if !covered_by.iter().any(|(r, radius)| {
                r.contains(x as f32, y as f32)
                    && super::image::rect_distance(x - r.x, y - r.y, r.w, r.h, *radius) < 0.0
            }) {
                *target = pixel;
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct TextStyle {
    size: i32,
    bold: bool,
    align: Align,
    lines: usize,
    ink: u8,
    overflow: parley::OverflowWrap,
}

impl TextStyle {
    pub const fn new(size: i32) -> Self {
        Self {
            size,
            bold: false,
            align: Align::Left,
            lines: 1,
            ink: BLACK,
            overflow: parley::OverflowWrap::Normal,
        }
    }

    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub const fn break_long_words(mut self) -> Self {
        self.overflow = parley::OverflowWrap::Anywhere;
        self
    }

    pub const fn center(mut self) -> Self {
        self.align = Align::Center;
        self
    }

    pub const fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }

    pub const fn lines(mut self, lines: usize) -> Self {
        self.lines = lines;
        self
    }

    pub const fn ink(mut self, ink: u8) -> Self {
        self.ink = ink;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the two Noto Sans static UI font files"]
    fn emphasized_utf8_range_shapes_one_line_and_preserves_unstyled_prefix_pixels() {
        let paths = std::env::var_os("KL_TEST_FONTS").expect("supply regular and bold Noto Sans");
        let fonts: Vec<_> = std::env::split_paths(&paths)
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut canvas = Canvas::new(1264, 180, &fonts).unwrap();
        let rect = Rect::new(15, 18, 1220, 110);
        let text = "Coast and café";
        let at = text.find("café").unwrap();
        let mut draw = |span: Option<std::ops::Range<usize>>| {
            canvas.begin();
            match span {
                Some(range) => canvas.emphasized_text(rect, text, TextStyle::new(44), range),
                None => canvas.text(rect, text, TextStyle::new(44)),
            }
            canvas.finish(false)
        };
        let plain = draw(None);
        let styled = draw(Some(at..text.len()));
        assert_eq!(plain, draw(Some(at..text.len() - 1)));
        assert_eq!(plain, draw(Some(at..at)));
        let changed: Vec<_> = plain.iter().zip(&styled).enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i % 1264))
            .collect();
        assert!(changed.len() > 100);
        assert!(changed.iter().all(|&x| x > rect.x as usize + 120));
    }


    #[test]
    fn single_line_field_masks_characters_and_tracks_unicode_caret() {
        let mut editor = Editor::new(16);
        editor.text = "aé\nz".chars().collect();
        editor.caret = 2;
        assert_eq!(single_line_content(&editor, false), ("aé z".into(), 3));
        assert_eq!(single_line_content(&editor, true), ("••••".into(), 6));
        editor.caret = 42;
        assert_eq!(single_line_content(&editor, false), ("aé z".into(), 5));
    }

    #[test]
    fn single_line_field_scrolls_only_to_keep_the_caret_visible() {
        assert_eq!(horizontal_scroll(40.0, 160), 0);
        assert_eq!(horizontal_scroll(160.8, 160), 0);
        assert_eq!(horizontal_scroll(215.2, 160), 55);
    }

    #[test]
    fn pictures_clip_at_screen_edges_and_leave_overlapping_popup_visible() {
        let bounds = Rect::new(0, 0, 6, 4);
        let picture = Picture {
            rect: Rect::new(-2, -1, 7, 5),
            image: GrayImage {
                width: 7,
                height: 5,
                pixels: (0..35).collect(),
            },
            after_popups: 0,
        };
        let mut pixels = vec![99; 24];
        blit_picture(
            &mut pixels,
            bounds,
            &picture,
            &[(Rect::new(2, 1, 2, 3), 0.0)],
        );
        assert_eq!(&pixels[..6], &[9, 10, 11, 12, 13, 99]);
        assert_eq!(&pixels[6..12], &[16, 17, 99, 99, 20, 99]);
        assert_eq!(&pixels[18..], &[30, 31, 99, 99, 34, 99]);
    }
}
