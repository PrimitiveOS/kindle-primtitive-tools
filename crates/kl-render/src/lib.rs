//! Draws the launcher's screens into an 8-bit gray buffer: shapes with `vello_cpu`, text with `parley`
//! (the same libraries and versions as Primitive Browser), icons as pre-dithered gray tiles.
//!
//! E-ink rules (see the `kindle-eink` skill): chrome and text are pure black on pure white after
//! `solidify`, so page changes do not flash; only icons carry gray, dithered to the panel's 16 steps.

pub mod component;
pub mod image;
mod settings_management;

use std::collections::HashMap;

use kl_core::{
    layout::{Button, QuickTile, Rect, Row, LABEL_SIZE, QUICK_LEVEL_MAX},
    wallpaper::{WallpaperChoice, WallpaperDraft, WallpaperTarget},
    App, IconSource, Launch, Layout, Rotation, Screen, Status, Ui,
};
use parley::{FontContext, FontWeight, LayoutContext, PositionedLayoutItem, StyleProperty};
use vello_cpu::{
    kurbo::{self, Affine, Shape},
    peniko::Color,
    Glyph, Pixmap, RenderContext, Resources,
};

pub use image::GrayImage;

const BLACK: u8 = 0;
const WHITE: u8 = 255;
const SYSTEM_NAME: &str = "PrimitiveOS";
/// The horizontal margin of the Settings cards.
const MARGIN_X: i32 = 40;

/// The switch's rectangle in a row: at the right, centred vertically.
fn switch_rect(r: Rect) -> Rect {
    Rect::new(r.right() - 40 - kl_core::layout::SWITCH_W, r.y + (r.h - kl_core::layout::SWITCH_H) / 2, kl_core::layout::SWITCH_W, kl_core::layout::SWITCH_H)
}

/// The Wi-Fi icon's lit arcs for a status: None for Wi-Fi off (the slash), the signal's arcs when connected,
/// and no arcs while Wi-Fi is on without a network.
fn wifi_lit(s: &Status) -> Option<usize> {
    match (s.wifi_on, s.wifi_state == "connected") {
        (false, _) => None,
        (true, false) => Some(0),
        (true, true) => Some(match s.wifi_signal {
            Some((level, max)) => kl_core::wifi::arcs(level, max),
            None => 3,
        }),
    }
}

/// Pop-ups (the app menu, the Kindle UI dialog): corner radius, and the white margin around them.
const POPUP_RADIUS: f32 = 28.0;
/// The power symbol of the Power tile: a stroke from the top and an arc round the bottom (24-unit grid).
const POWER_ICON: primitive_quick::Icon = primitive_quick::Icon { stroke: &["M12 3 V11", "M6.3 6.3 A8 8 0 1 0 17.7 6.3"], fill: &[] };
const POPUP_HALO: i32 = 10;

// Icon paths copied from Primitive Browser (`crates/kb-app/src/ui.rs`, the `icon` set and `battery_icon`).
const WIFI_ARC_OUTER: &str = "M2.5 9 A14 14 0 0 1 21.5 9";
const WIFI_ARC_MIDDLE: &str = "M5.5 12.5 A9.5 9.5 0 0 1 18.5 12.5";
const WIFI_ARC_INNER: &str = "M8.5 16 A5 5 0 0 1 15.5 16";
const BOLT: &str = "M8 0 L1 12 H6 L4 22 L13 9 H8 Z";
/// Wi-Fi arcs above the signal strength.
const UNLIT: u8 = 187;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Start,
    Center,
    End,
}

/// One text block: wrapped to `w`, at most `lines` lines (the last one shortened with "…").
struct Text<'a> {
    text: &'a str,
    x: f32,
    y: f32,
    w: f32,
    size: f32,
    bold: bool,
    align: Align,
    lines: usize,
    gray: u8,
}

pub struct Renderer {
    width: u16,
    ctx: RenderContext,
    resources: Resources,
    pixmap: Pixmap,
    fonts: FontContext,
    layout_cx: LayoutContext<()>,
    large_text: bool,
    high_contrast: bool,
    icons: HashMap<String, GrayImage>,
    /// The photo behind the Launcher grid, already prepared at the screen size.
    home_wallpaper: Option<GrayImage>,
    /// The picture under consideration in Settings, prepared at the screen size.
    wallpaper_preview: Option<GrayImage>,
    /// The launcher's own sleep screen picture, when one is set (screensavers folder).
    sleep_image: Option<GrayImage>,
    /// Over an app (the system layer), the app's screen (upright): the backdrop of the system's pop-ups instead of
    /// the home screen.
    backdrop: Option<Vec<u8>>,
    /// The drive-mode picture (`assets/usb-drive-mode.png`), fitted to the screen and snapped to black and white.
    usb_picture: Option<GrayImage>,
    /// Store pictures (screenshots) to draw over the store's product page, collected while it is drawn.
    store_blits: Vec<(Rect, String)>,
    /// The controls of the last frame drawn: each one's rectangle and corner radius, in drawing order. A tap on one of
    /// them shows that control reversed before its action runs (`press_frame`).
    targets: Vec<(Rect, f64)>,
    /// The control a tap has touched, drawn reversed (black and white swapped) by `draw`.
    press: Option<Rect>,
}

/// The sizes of a store icon's tiles (a row, the featured card, the product page), and a screenshot's size.
pub const STORE_ROW_ICON: u32 = 132;
pub const STORE_FEATURE_ICON: u32 = 200;
pub const STORE_PRODUCT_ICON: u32 = 340;
pub const STORE_ICON_SIZES: [u32; 3] = [STORE_ROW_ICON, STORE_FEATURE_ICON, STORE_PRODUCT_ICON];
pub const STORE_SHOT_W: u32 = 406;
pub const STORE_SHOT_H: u32 = 540;

pub fn store_icon_key(id: &str, size: u32) -> String {
    format!("store-icon:{id}:{size}")
}

/// The key of an app's dark-form tile among the renderer's icons (the light form's key is the app's id).
pub fn dark_icon_key(id: &str) -> String {
    format!("dark-form:{id}")
}

/// The tile to draw for app `id` in the theme: its dark form in dark mode when it has one, else its light form.
fn icon_key_for_theme(icons: &HashMap<String, GrayImage>, id: &str, dark_mode: bool) -> String {
    let dark = dark_icon_key(id);
    if dark_mode && icons.contains_key(&dark) {
        dark
    } else {
        id.to_string()
    }
}

pub fn store_shot_key(id: &str, index: usize) -> String {
    format!("store-shot:{id}:{index}")
}

/// The store's pictures in the cache: `icons/<id>.png` for each app, and `shots/<id>-<n>.png` (n from 0) for its
/// screenshots. A picture that does not read is skipped; the store then shows the app's letter.
pub fn load_store_pictures(renderer: &mut Renderer, cache: &std::path::Path) {
    renderer.clear_store_artwork();
    if let Ok(read) = std::fs::read_dir(cache.join("icons")) {
        for e in read.flatten() {
            let path = e.path();
            let (Some(id), Ok(png)) = (path.file_stem().and_then(|s| s.to_str()).map(str::to_string), std::fs::read(&path)) else { continue };
            let _ = renderer.set_store_icon(&id, &png);
        }
    }
    if let Ok(read) = std::fs::read_dir(cache.join("shots")) {
        for e in read.flatten() {
            let path = e.path();
            let Some((id, index)) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.rsplit_once('-')) else { continue };
            let (id, Ok(index), Ok(png)) = (id.to_string(), index.parse::<usize>(), std::fs::read(&path)) else { continue };
            let _ = renderer.set_store_shot(&id, index, &png);
        }
    }
}

/// The drive-mode picture: a laptop and a Kindle joined by a cable.
const USB_PICTURE: &[u8] = include_bytes!("../../../assets/usb-drive-mode.png");

fn gray(v: u8) -> Color {
    Color::from_rgb8(v, v, v)
}

fn krect(r: Rect) -> kurbo::Rect {
    kurbo::Rect::new(r.x as f64, r.y as f64, r.right() as f64, r.bottom() as f64)
}

impl Renderer {
    pub fn clear_store_artwork(&mut self) {
        self.icons.retain(|key, _| !key.starts_with("store-icon:") && !key.starts_with("store-shot:"));
    }

    /// A renderer for a `width` × `height` screen with the fonts at `font_paths` (Noto Sans regular and
    /// bold). Fails when no font could be loaded.
    pub fn new(width: u32, height: u32, font_paths: &[String]) -> Result<Renderer, String> {
        let mut fonts = FontContext::new();
        let existing: Vec<&String> = font_paths.iter().filter(|p| std::fs::metadata(p).is_ok()).collect();
        if existing.is_empty() {
            return Err(format!("no fonts found in {font_paths:?}"));
        }
        fonts.collection.load_fonts_from_paths(existing.iter().map(|p| p.as_str()));
        let Some(family) = fonts.collection.family_id("Noto Sans") else {
            return Err("font family Noto Sans not found".into());
        };
        fonts.collection.set_generic_families(parley::GenericFamily::SansSerif, std::iter::once(family));
        let (w, h) = (width as u16, height as u16);
        Ok(Renderer {
            width: w,
            ctx: RenderContext::new(w, h),
            resources: Resources::new(),
            pixmap: Pixmap::new(w, h),
            fonts,
            layout_cx: LayoutContext::new(),
            large_text: false,
            high_contrast: false,
            icons: HashMap::new(),
            home_wallpaper: None,
            wallpaper_preview: None,
            sleep_image: None,
            backdrop: None,
            store_blits: Vec::new(),
            targets: Vec::new(),
            press: None,
            usb_picture: None,
        })
    }

    /// Set the dithered icon tile for an app id (sized to the layout's icon box).
    pub fn set_icon(&mut self, id: &str, tile: GrayImage) {
        self.icons.insert(id.to_string(), tile);
    }

    /// The store's picture of an app, at each size the store draws it: a rounded icon tile (`store-icon:<id>:<px>`).
    pub fn set_store_icon(&mut self, id: &str, png: &[u8]) -> Result<(), String> {
        for size in STORE_ICON_SIZES {
            let tile = image::icon_tile(png, size)?;
            self.set_icon(&store_icon_key(id, size), tile);
        }
        Ok(())
    }

    /// One screenshot of an app, fitted to the product page (`store-shot:<id>:<index>`).
    pub fn set_store_shot(&mut self, id: &str, index: usize, png: &[u8]) -> Result<(), String> {
        let tile = image::picture_tile(png, STORE_SHOT_W, STORE_SHOT_H)?;
        self.set_icon(&store_shot_key(id, index), tile);
        Ok(())
    }

    /// The screen-sized photo behind the home grid (`None` for a plain background).
    pub fn set_home_wallpaper(&mut self, picture: Option<GrayImage>) {
        self.home_wallpaper = picture;
    }

    /// Set the preview's screen-sized picture before applying the draft's opacity.
    pub fn set_wallpaper_preview(&mut self, picture: Option<GrayImage>) {
        self.wallpaper_preview = picture;
    }

    /// The picture for the next sleep screen (`None`: the text sleep screen).
    pub fn set_sleep_image(&mut self, image: Option<GrayImage>) {
        self.sleep_image = image;
    }

    /// The app's screen (upright, one byte a pixel, the screen's size) while the system layer shows its pop-ups over
    /// an app; `None` when no app is under them.
    pub fn set_backdrop(&mut self, picture: Option<Vec<u8>>) {
        self.backdrop = picture;
    }

    /// Set the tile of an app's dark form, drawn instead of its light one in dark mode.
    pub fn set_dark_icon(&mut self, id: &str, tile: GrayImage) {
        self.icons.insert(dark_icon_key(id), tile);
    }

    /// Drop an app's tiles, both forms (its icon was changed; the driver loads the new one).
    pub fn forget_icon(&mut self, id: &str) {
        self.icons.remove(id);
        self.icons.remove(&dark_icon_key(id));
    }

    /// Whether a sleep picture is set (the launcher's own sleep screen has something to show).
    pub fn has_sleep_image(&self) -> bool {
        self.sleep_image.is_some()
    }

    pub fn has_icon(&self, id: &str) -> bool {
        self.icons.contains_key(id)
    }

    // ----- primitives -----

    fn fill(&mut self, r: Rect, v: u8) {
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.set_paint(gray(v));
        self.ctx.fill_rect(&krect(r));
    }

    fn frame(&mut self, r: Rect, width: i32, v: u8) {
        self.fill(Rect::new(r.x, r.y, r.w, width), v);
        self.fill(Rect::new(r.x, r.bottom() - width, r.w, width), v);
        self.fill(Rect::new(r.x, r.y, width, r.h), v);
        self.fill(Rect::new(r.right() - width, r.y, width, r.h), v);
    }

    fn rounded(&mut self, r: Rect, radius: f64, v: u8) {
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.set_paint(gray(v));
        let path = kurbo::RoundedRect::from_rect(krect(r), radius).to_path(0.1);
        self.ctx.fill_path(&path);
    }

    /// Stroke an SVG path drawn on the browser's 24-unit icon grid, scaled to `size` pixels at (`x`, `y`).
    fn stroke_icon(&mut self, d: &str, x: f64, y: f64, size: f64, width: f64, v: u8) {
        let Ok(path) = kurbo::BezPath::from_svg(d) else { return };
        self.ctx.set_transform(Affine::translate((x, y)) * Affine::scale(size / 24.0));
        self.ctx.set_stroke(kurbo::Stroke::new(width).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round));
        self.ctx.set_paint(gray(v));
        self.ctx.stroke_path(&path);
        self.ctx.set_transform(Affine::IDENTITY);
    }

    /// Fill an SVG path at (`x`, `y`) with `scale` (path units to pixels).
    fn fill_svg(&mut self, d: &str, x: f64, y: f64, scale: f64, v: u8) {
        let Ok(path) = kurbo::BezPath::from_svg(d) else { return };
        self.ctx.set_transform(Affine::translate((x, y)) * Affine::scale(scale));
        self.ctx.set_paint(gray(v));
        self.ctx.fill_path(&path);
        self.ctx.set_transform(Affine::IDENTITY);
    }

    /// The Wi-Fi icon from Primitive Browser (`kb-app/src/ui.rs`, "wifi" and "wifi-off"): three arcs and a dot
    /// on a 24-unit grid, stroke 2. `lit` arcs (from the inside, by signal strength) are black and the rest light
    /// gray; `None` draws the slash of Wi-Fi off.
    fn wifi_icon(&mut self, x: f64, y: f64, size: f64, lit: Option<usize>) {
        let shown = lit.unwrap_or(3);
        for (i, arc) in [WIFI_ARC_INNER, WIFI_ARC_MIDDLE, WIFI_ARC_OUTER].iter().enumerate() {
            self.stroke_icon(arc, x, y, size, 2.0, if i < shown { BLACK } else { UNLIT });
        }
        let k = size / 24.0;
        self.ctx.set_paint(gray(BLACK));
        let dot = kurbo::Circle::new((x + 12.0 * k, y + 19.5 * k), 1.6 * k).to_path(0.1);
        self.ctx.fill_path(&dot);
        if lit.is_none() {
            // A white gap under the slash keeps it readable over the arcs.
            self.stroke_icon("M3 3 L21 21", x, y, size, 5.0, WHITE);
            self.stroke_icon("M3 3 L21 21", x, y, size, 2.4, BLACK);
        }
    }

    /// The battery icon from Primitive Browser (`kb-app/src/ui.rs`, `battery_icon`): a 128 × 56 battery filled
    /// to the level in black, the number in its middle, black on the empty part and white on the filled part
    /// (split at the level's edge), and a bolt while charging.
    fn battery_icon(&mut self, x: i32, y: i32, level: u32, charging: bool) {
        let pct = level.min(100);
        let fill = (104.0 * pct as f32 / 100.0).round() as i32;
        self.rounded_frame(Rect::new(x + 2, y + 4, 116, 48), 12.0, 3, BLACK, WHITE);
        self.rounded(Rect::new(x + 122, y + 18, 5, 20), 2.0, BLACK);
        if fill > 0 {
            self.rounded(Rect::new(x + 8, y + 10, fill, 36), 6.0f64.min(fill as f64 / 2.0), BLACK);
        }
        let edge = (x + 8 + fill) as f64;
        let number = pct.to_string();
        let draw_number = |r: &mut Renderer, v: u8| {
            let w = r.text_width(&number, 28.0, true);
            let bolt_w = if charging { 18.0 } else { 0.0 };
            let left = x as f32 + (128.0 - w - bolt_w) / 2.0;
            if charging {
                r.fill_svg(BOLT, (left - 2.0) as f64, (y + 17) as f64, 1.0, v);
            }
            r.text(Text { text: &number, x: left + bolt_w, y: (y + 11) as f32, w: w + 4.0, size: 28.0, bold: true, align: Align::Start, lines: 1, gray: v });
        };
        draw_number(self, BLACK);
        if fill > 0 {
            let clip = kurbo::Rect::new(x as f64, y as f64, edge, (y + 56) as f64).to_path(0.1);
            self.ctx.set_transform(Affine::IDENTITY);
            self.ctx.push_clip_layer(&clip);
            draw_number(self, WHITE);
            self.ctx.pop_layer();
        }
    }

    /// A rounded outline: an outer fill in `line`, an inner fill in `inside`.
    fn rounded_frame(&mut self, r: Rect, radius: f64, width: i32, line: u8, inside: u8) {
        self.rounded(r, radius, line);
        self.rounded(r.inset(width), (radius - width as f64).max(0.0), inside);
    }

    /// Lay out and draw text. Returns the height used.
    fn text(&mut self, t: Text) -> f32 {
        self.text_with_wrap(t, parley::OverflowWrap::Normal)
    }

    fn text_with_wrap(&mut self, t: Text, overflow: parley::OverflowWrap) -> f32 {
        let mut content = t.text.to_string();
        // Shorten until it fits in `lines` lines.
        loop {
            let layout = self.layout_with_wrap(&content, t.size, t.bold, t.w, overflow);
            let count = layout.lines().count();
            if count <= t.lines || content.chars().count() <= 1 {
                return self.draw_layout(&layout, &t);
            }
            let keep: String = content.trim_end_matches('…').chars().collect();
            let mut chars: Vec<char> = keep.chars().collect();
            let drop = (chars.len() / 10).max(1);
            chars.truncate(chars.len().saturating_sub(drop));
            content = format!("{}…", chars.into_iter().collect::<String>().trim_end());
        }
    }

    fn layout_text(&mut self, text: &str, size: f32, bold: bool, w: f32) -> parley::Layout<()> {
        self.layout_with_wrap(text, size, bold, w, parley::OverflowWrap::Normal)
    }

    fn layout_with_wrap(&mut self, text: &str, size: f32, bold: bool, w: f32, overflow: parley::OverflowWrap) -> parley::Layout<()> {
        let size = if self.large_text { size * 1.15 } else { size };
        let mut builder = self.layout_cx.ranged_builder(&mut self.fonts, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(size));
        builder.push_default(StyleProperty::FontWeight(if bold { FontWeight::BOLD } else { FontWeight::NORMAL }));
        builder.push_default(StyleProperty::LineHeight(parley::LineHeight::FontSizeRelative(1.25)));
        builder.push_default(StyleProperty::OverflowWrap(overflow));
        let mut layout: parley::Layout<()> = builder.build(text);
        layout.break_all_lines(Some(w));
        layout
    }

    fn draw_layout(&mut self, layout: &parley::Layout<()>, t: &Text) -> f32 {
        let mut height = 0.0;
        let ink = if self.high_contrast && t.gray < 128 { BLACK } else { t.gray };
        for line in layout.lines().take(t.lines) {
            let m = line.metrics();
            let dx = match t.align {
                Align::Start => 0.0,
                Align::Center => ((t.w - m.advance) / 2.0).max(0.0),
                Align::End => (t.w - m.advance).max(0.0),
            };
            height = m.offset + m.line_height;
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
                let run = glyph_run.run();
                self.ctx.set_transform(Affine::translate(((t.x + dx) as f64, t.y as f64)));
                self.ctx.set_paint(gray(ink));
                let _ = self
                    .ctx
                    .glyph_run(&mut self.resources, run.font())
                    .font_size(run.font_size())
                    .hint(true)
                    // The bundled Noto fonts are static, so there are no variation coordinates.
                    .normalized_coords(&[])
                    .fill_glyphs(glyph_run.positioned_glyphs().map(|g| Glyph { id: g.id, x: g.x, y: g.y }));
            }
        }
        self.ctx.set_transform(Affine::IDENTITY);
        height
    }

    /// The pixel width of one line of text.
    fn text_width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        let layout = self.layout_text(text, size, bold, f32::MAX);
        let width = layout.lines().next().map_or(0.0, |l| l.metrics().advance);
        width
    }

    /// Draw the shared drive-mode artwork using the embedded PNG and a white page.
    fn usb_drive_frame(&mut self, width: u32, height: u32) -> Option<Vec<u8>> {
        if self.usb_picture.is_none() {
            self.usb_picture = image::decode_png(USB_PICTURE).ok().map(|img| {
                let mut fitted = image::fit(
                    &img,
                    width.saturating_sub(120),
                    height.saturating_sub(240),
                );
                image::solidify(&mut fitted.pixels);
                fitted
            });
        }
        let pic = self.usb_picture.as_ref()?;
        let (w, h) = (width as usize, height as usize);
        let (pw, ph) = (pic.width as usize, pic.height as usize);
        let (ox, oy) = (w.checked_sub(pw)? / 2, h.checked_sub(ph)? / 2);
        let mut out = vec![WHITE; w * h];
        for y in 0..ph {
            let dst = (oy + y) * w + ox;
            out[dst..dst + pw].copy_from_slice(&pic.pixels[y * pw..(y + 1) * pw]);
        }
        Some(out)
    }

    /// Rasterise the recorded scene over opaque white and snap it to black and white.
    fn finish_gray(&mut self) -> Vec<u8> {
        self.ctx.flush();
        self.ctx.render(self.pixmap.as_mut(), &mut self.resources);
        let mut out: Vec<u8> = self.pixmap.data_as_u8_slice().iter().step_by(4).copied().collect();
        image::solidify(&mut out);
        out
    }

    // ----- screens -----

    /// Draw the current screen. Returns `width × height` gray pixels, upright. The controls drawn are recorded for
    /// `press_frame`, and a pressed control is reversed.
    pub fn draw(&mut self, ui: &Ui, status: &Status) -> Vec<u8> {
        self.targets.clear();
        let mut out = self.draw_frame(ui, status);
        if let Some(r) = self.press {
            let width = ui.layout.width as usize;
            let radius = self.targets.iter().find(|(t, _)| *t == r).map_or(0.0, |(_, radius)| *radius);
            invert_control(&mut out, width, r, radius);
        }
        out
    }

    /// The frame for a tap at (x, y) on the screen last drawn: the frame with the control under the tap reversed, or
    /// `None` when the tap is not on a control. The app shows it before the tap's action runs, so the tap is seen at
    /// once.
    pub fn press_frame(&mut self, ui: &Ui, status: &Status, x: f32, y: f32) -> Option<Vec<u8>> {
        // The smallest control under the tap: a row inside a card is the control, not the card around it.
        let target = self
            .targets
            .iter()
            .filter(|(r, _)| r.contains(x, y))
            .min_by_key(|(r, _)| r.w as i64 * r.h as i64)
            .map(|(r, _)| *r)?;
        self.press = Some(target);
        let frame = self.draw(ui, status);
        self.press = None;
        Some(frame)
    }

    /// Record a control for the tap feedback (see `draw`).
    fn touch(&mut self, r: Rect, radius: f64) {
        self.targets.push((r, radius));
    }

    fn draw_frame(&mut self, ui: &Ui, status: &Status) -> Vec<u8> {
        let launcher = !matches!(ui.screen, Screen::Store | Screen::StoreApp);
        self.large_text = launcher && status.large_text;
        self.high_contrast = launcher && status.high_contrast;
        // While the drive is shown on a computer, the drive-mode screen is all there is.
        if ui.usb == kl_core::Usb::Drive {
            let (width, height) = (ui.layout.width as u32, ui.layout.height as u32);
            return self
                .usb_drive_frame(width, height)
                .unwrap_or_else(|| vec![WHITE; (width * height) as usize]);
        }
        let l = ui.layout;
        let preview = match ui.screen {
            Screen::WallpaperPreview => ui.wallpaper_preview.as_ref(),
            _ => None,
        };
        if let Some((draft, notice)) = preview.zip(ui.notice.as_deref()) {
            return self.wallpaper_preview_frame(&l, status, draft, None, Some(notice));
        }
        let home_preview = preview.filter(|draft| {
            draft.target == WallpaperTarget::Home
                && draft.can_apply()
                && (draft.choice == WallpaperChoice::Default || self.wallpaper_preview.is_some())
        });
        if let Some(draft) = preview.filter(|_| home_preview.is_none()) {
            let area = l.wallpaper_preview().picture.inset(4);
            let miniature = self.wallpaper_preview.as_ref().filter(|_| draft.can_apply() && draft.has_opacity()).map(|source| {
                let mut picture = image::fit(source, area.w as u32, area.h as u32);
                if draft.opacity == 100 {
                    image::dither16(&mut picture);
                } else {
                    image::blend_wallpaper(&mut picture, draft.opacity);
                }
                picture
            });
            return self.wallpaper_preview_frame(&l, status, draft, miniature.as_ref(), None);
        }
        self.ctx.reset();
        self.fill(l.screen(), WHITE);
        let mut icon_blits: Vec<(Rect, String)> = Vec::new();
        let screen = if home_preview.is_some() { Screen::Home } else { ui.screen };
        match screen {
            Screen::Home => self.home(ui, status, &mut icon_blits),
            Screen::Settings => self.settings_home(ui, status),
            Screen::Page(page) => self.settings_page(ui, status, page),
            Screen::WallpaperPicker(target) => self.settings_wallpaper_picker(ui, target),
            Screen::WallpaperPreview => {},
            Screen::NetworkDetail => self.network_detail(ui),
            Screen::Password => self.password_screen(ui),
            Screen::NetworkName => self.network_name_screen(ui),
            Screen::SshPassword => self.ssh_password_screen(ui),
            Screen::DeviceName => self.device_name_screen(ui),
            // The dialog is a pop-up over the grid, drawn last (below).
            Screen::ConfirmKindle => self.home(ui, status, &mut icon_blits),
            Screen::ConfirmRestart => self.home(ui, status, &mut icon_blits),
            Screen::PowerMenu => self.home(ui, status, &mut icon_blits),
            // A sleep picture replaces the text, except for a battery warning, which matters more.
            Screen::Sleep => {
                if self.sleep_image.is_none() || ui.battery_message().is_some() {
                    self.sleep(&l, ui.battery_message());
                }
            }
            Screen::AppMenu => self.app_menu(ui, status, &mut icon_blits),
            Screen::Extensions => self.extensions(ui),
            Screen::Store => self.store_list(ui),
            Screen::StoreApp => self.store_app(ui),
            // The panel is a pop-up over the home screen, drawn last (below).
            Screen::QuickSettings => self.home(ui, status, &mut icon_blits),
        }
        icon_blits.append(&mut self.store_blits);
        let width = self.width as usize;
        let mut out = self.finish_gray();
        let wallpaper = if let Some(draft) = home_preview {
            self.wallpaper_preview.as_ref().filter(|_| draft.has_opacity()).map(|picture| (picture, draft.opacity))
        } else if matches!(screen, Screen::Home | Screen::ConfirmKindle | Screen::ConfirmRestart | Screen::PowerMenu | Screen::AppMenu | Screen::QuickSettings) {
            self.home_wallpaper.as_ref().map(|picture| (picture, ui.wallpapers.opacity(WallpaperTarget::Home)))
        } else {
            None
        }.filter(|(_, opacity)| *opacity > 0);
        if let Some((picture, opacity)) = wallpaper {
            shade_home_wallpaper(&mut out, &l, picture, status.dark_mode, opacity);
        }
        let store_clip = (ui.screen == Screen::Store).then(|| ui.store_list_viewport());
        // Pictures (icons, the sleep picture) keep their look in dark mode.
        let mut pictures = Vec::new();
        for (rect, id) in icon_blits {
            if let Some(tile) = self.icons.get(&id) {
                if id.starts_with("store-shot:") {
                    let top = rect.y.max(170);
                    let bottom = rect.bottom().min(l.height - 120);
                    for y in top..bottom {
                        let source = (y - rect.y) as usize * tile.width as usize;
                        let destination = y as usize * width + rect.x as usize;
                        let count = tile.width as usize;
                        out[destination..destination + count].copy_from_slice(&tile.pixels[source..source + count]);
                    }
                    pictures.push(((rect.x, top, rect.w, bottom - top), 0.0));
                } else if ui.screen != Screen::StoreApp || rect.y >= 170 {
                    let radius = rect.w as f32 * image::ICON_RADIUS;
                    let clip_radius = if wallpaper.is_some() { radius } else { 0.0 };
                    blit(&mut out, width, rect, tile, clip_radius, store_clip);
                    pictures.push(((rect.x, rect.y, rect.w, rect.h), radius));
                }
            }
        }
        if ui.screen == Screen::Sleep && ui.battery_message().is_none() {
            if let Some(picture) = &self.sleep_image {
                let s = l.screen();
                let r = Rect::new((s.w - picture.width as i32) / 2, (s.h - picture.height as i32) / 2, picture.width as i32, picture.height as i32);
                blit(&mut out, width, r, picture, 0.0, None);
                pictures.push(((r.x, r.y, r.w, r.h), 0.0));
            }
        }
        // Pop-ups are drawn in a second pass and copied over the screen, so icon pictures stay under them.
        let popup = match ui.screen {
            Screen::ConfirmKindle => Some(l.confirm_box()),
            Screen::ConfirmRestart => Some(l.confirm_box()),
            Screen::PowerMenu => Some(l.power_box()),
            Screen::AppMenu => Some(l.menu_box(ui.menu_anchor(), ui.menu_actions().len())),
            Screen::QuickSettings => Some(l.quick_panel()),
            Screen::StoreApp if ui.store_sheet.is_some() => Some(l.store_sheet()),
            Screen::StoreApp if ui.store_remove_armed.is_some() => Some(l.confirm_box()),
            Screen::Page(kl_core::layout::Page::Store) if ui.store_clear_confirm => Some(l.store_settings_box()),
            Screen::Store if ui.store_settings_open => Some(l.store_settings_box()),
            Screen::Store if ui.store_category_picker || ui.store_batch_confirm => Some(l.store_sheet()),
            _ => None,
        };
        // Quick settings is a sheet from the top edge: square, with no margin.
        let sheet = ui.screen == Screen::QuickSettings;
        let (radius, halo) = if sheet { (0.0, 0) } else { (POPUP_RADIUS, POPUP_HALO) };
        if let Some(b) = popup {
            self.ctx.reset();
            self.fill(l.screen(), WHITE);
            match ui.screen {
                Screen::ConfirmKindle => self.confirm(&l),
                Screen::ConfirmRestart => self.confirm_restart(&l),
                Screen::PowerMenu => self.power_menu_box(&l),
                Screen::QuickSettings => self.quick_panel(&l, status),
                Screen::StoreApp if ui.store_sheet.is_some() => self.store_install_sheet(ui),
                Screen::StoreApp => self.store_remove_dialog(ui),
                Screen::Page(kl_core::layout::Page::Store) => self.store_settings_dialog(ui),
                Screen::Store if ui.store_settings_open => self.store_settings_dialog(ui),
                Screen::Store if ui.store_category_picker => self.store_category_dialog(ui),
                Screen::Store => self.store_batch_dialog(ui),
                _ => self.app_menu_box(ui),
            }
            let layer = self.finish_gray();
            image::copy_rounded(&mut out, &layer, width, (b.x - halo, b.y - halo, b.w + 2 * halo, b.h + 2 * halo), radius + halo as f32);
        }
        if status.dark_mode && ui.screen != Screen::Sleep {
            let over = popup.map(|b| (b.x - halo, b.y - halo, b.w + 2 * halo, b.h + 2 * halo));
            let clip = store_clip.map(|r| (r.x, r.y, r.w, r.h));
            image::invert_except_clipped(&mut out, width, &pictures, over, clip);
        }
        // Over an app, the system's pop-ups float over the app's screen: every pixel outside the pop-up is the app's.
        if let (Some(backdrop), Some(b)) = (self.backdrop.as_ref(), popup) {
            if matches!(ui.screen, Screen::QuickSettings | Screen::PowerMenu | Screen::ConfirmKindle | Screen::ConfirmRestart | Screen::AppMenu)
                && backdrop.len() == out.len()
            {
                image::keep_area(&mut out, backdrop, width, (b.x - halo, b.y - halo, b.w + 2 * halo, b.h + 2 * halo));
            }
        }
        // The pressed look: only the touched icon darkens.
        if let (Screen::Home, Some(r)) = (ui.screen, ui.pressed) {
            image::darken_icon(&mut out, width, r.x, r.y, r.w);
        }
        if let Some(draft) = home_preview {
            let source = GrayImage { width: l.width as u32, height: l.height as u32, pixels: out };
            let area = l.wallpaper_preview().picture.inset(4);
            let mut miniature = image::fit(&source, area.w as u32, area.h as u32);
            image::dither16(&mut miniature);
            return self.wallpaper_preview_frame(&l, status, draft, Some(&miniature), None);
        }
        out
    }

    /// The "Opening the Kindle UI" dialog, where the confirm dialog was: a frame of the whole screen in which only
    /// the returned rectangle (the dialog and its margin) is drawn. The caller shows that rectangle over what is on
    /// the screen. `step` fills one more block each time.
    pub fn waiting_dialog(&mut self, l: &Layout, step: usize, dark: bool) -> (Vec<u8>, Rect) {
        const BLOCKS: i32 = 10;
        self.ctx.reset();
        self.fill(l.screen(), WHITE);
        let b = l.confirm_box();
        self.rounded_frame(b, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        let inner = b.inset(48);
        self.text(Text { text: "Opening the Kindle UI", x: inner.x as f32, y: inner.y as f32, w: inner.w as f32, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.text(Text { text: "This takes about a minute. The Kindle library opens by itself.", x: inner.x as f32, y: inner.y as f32 + 100.0, w: inner.w as f32, size: 36.0, bold: false, align: Align::Start, lines: 3, gray: BLACK });
        let (bw, gap, bh) = (72, 18, 40);
        let x0 = b.x + (b.w - (BLOCKS * bw + (BLOCKS - 1) * gap)) / 2;
        let filled = (step % (BLOCKS as usize + 1)) as i32;
        for i in 0..BLOCKS {
            let r = Rect::new(x0 + i * (bw + gap), b.bottom() - 48 - bh - 30, bw, bh);
            self.rounded_frame(r, 8.0, 4, BLACK, if i < filled { BLACK } else { WHITE });
        }
        let mut out = self.finish_gray();
        let halo = b.inset(-POPUP_HALO);
        if dark {
            image::invert_except(&mut out, self.width as usize, &[], None);
        }
        (out, halo)
    }

    /// The status bar, laid out like the quick settings header: the clock and date on the left; on the right the
    /// indicators (SSH, a running background job), then Wi-Fi with its signal (a slash when off), then the battery.
    fn status_strip(&mut self, l: &Layout, s: &Status, running: usize) {
        let r = l.status();
        let clock = if s.clock.is_empty() { "--:--" } else { s.clock.as_str() };
        let c = l.header_clock();
        self.text(Text { text: clock, x: c.x as f32, y: 14.0, w: 400.0, size: 52.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        if !s.date.is_empty() {
            self.text(Text { text: &s.date, x: c.x as f32, y: 76.0, w: 400.0, size: 24.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        }
        if let Some(level) = s.battery {
            let b = l.header_battery();
            self.battery_icon(b.x, b.y, level, s.charging);
        }
        let w = l.header_wifi();
        self.wifi_icon(w.x as f64, w.y as f64, 64.0, wifi_lit(s));
        // Text and badges to the left of Wi-Fi, laid out from right to left; `right` is the left edge of what is drawn.
        let mut right = w.x - 20;
        if s.wifi_on && s.wifi_state != "connected" {
            let note = if s.wifi_state.is_empty() { "Wi-Fi …".to_string() } else { "No network".to_string() };
            let width = self.text_width(&note, 30.0, false);
            let left = right as f32 - width;
            self.text(Text { text: &note, x: left, y: 44.0, w: width + 4.0, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
            right = (left - 20.0) as i32;
        }
        if running > 0 {
            right -= self.badge_width("Running") as i32;
            self.badge(right as f64, "Running");
            right -= 12;
        }
        if s.ssh {
            right -= self.badge_width("SSH") as i32;
            self.badge(right as f64, "SSH");
        }
        self.fill(Rect::new(0, r.bottom() - 2, r.w, 2), BLACK);
    }

    /// The width a badge with `label` takes.
    fn badge_width(&mut self, label: &str) -> f64 {
        self.text_width(label, 26.0, true) as f64 + 24.0
    }

    /// A small outlined label in the status bar at `x`. Returns its width.
    fn badge(&mut self, x: f64, label: &str) -> f64 {
        let w = self.badge_width(label);
        let r = Rect::new(x as i32, 38, w as i32, 44);
        self.rounded_frame(r, 10.0, 3, BLACK, WHITE);
        self.text(Text { text: label, x: (x + 12.0) as f32, y: 42.0, w: w as f32, size: 26.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        w
    }

    /// Quick settings, as Primitive Browser's panel: a white sheet from the top edge with the clock and date, the
    /// Wi-Fi and battery icons, two level rows of 25 steps with round − and + buttons, a grid of cards, and a
    /// Close handle.
    fn quick_panel(&mut self, l: &Layout, s: &Status) {
        let p = l.quick_panel();
        self.fill(p, WHITE);
        self.fill(Rect::new(0, p.bottom() - 3, p.w, 3), BLACK);
        let clock = if s.clock.is_empty() { "--:--" } else { s.clock.as_str() };
        let c = l.header_clock();
        self.text(Text { text: clock, x: c.x as f32, y: 14.0, w: 400.0, size: 52.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.text(Text { text: &s.date, x: c.x as f32, y: 76.0, w: 400.0, size: 24.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        if let Some(level) = s.battery {
            let b = l.header_battery();
            self.battery_icon(b.x, b.y, level, s.charging);
        }
        let w = l.header_wifi();
        self.wifi_icon(w.x as f64, w.y as f64, 64.0, wifi_lit(s));
        // Round icon buttons, above the sliders: black with a white icon when on, outlined when off, with a small
        // caption underneath.
        for (tile, slot) in l.quick_buttons() {
            if tile == QuickTile::AutoRotate && !s.display_support.auto_rotation {
                continue;
            }
            let (label, on) = match tile {
                QuickTile::Airplane => ("Airplane", !s.wifi_on),
                QuickTile::AutoRotate => ("Auto-rotate", s.auto_rotation),
                QuickTile::DarkMode => ("Dark mode", s.dark_mode),
                QuickTile::Animation => ("Animation", s.animation),
                QuickTile::Ssh => ("SSH", s.ssh),
                QuickTile::AllSettings => ("Settings", false),
                QuickTile::Power => ("Power", false),
            };
            let (bg, fg) = if on { (BLACK, WHITE) } else { (WHITE, BLACK) };
            let c = l.quick_button_circle(slot);
            self.rounded_frame(c, c.w as f64 / 2.0, 3, BLACK, bg);
            let size = 52.0;
            let (cx, cy) = ((c.x + c.w / 2) as f64, (c.y + c.h / 2) as f64);
            let (ix, iy) = (cx - size / 2.0, cy - size / 2.0);
            // The shared icons (`primitive-quick`), the same as Primitive Browser's panel.
            let icon = match tile {
                QuickTile::Airplane => primitive_quick::icons::PLANE,
                QuickTile::AutoRotate => primitive_quick::icons::ROTATE,
                QuickTile::DarkMode => primitive_quick::icons::MOON,
                QuickTile::Animation => primitive_quick::icons::ANIMATION,
                QuickTile::Ssh => primitive_quick::icons::SSH,
                QuickTile::AllSettings => primitive_quick::icons::SETTINGS,
                QuickTile::Power => POWER_ICON,
            };
            let _ = (cx, cy, bg);
            self.draw_icon(icon, ix, iy, size, fg);
            self.text(Text { text: label, x: slot.x as f32, y: (c.bottom() + 6) as f32, w: slot.w as f32, size: 24.0, bold: false, align: Align::Center, lines: 1, gray: BLACK });
        }
        for row in 0..2 {
            let (icon, label, value, available) = if row == 0 {
                (primitive_quick::icons::SUN, "Front light", s.light, s.display_support.front_light)
            } else {
                (primitive_quick::icons::WARMTH, "Warmth", s.warmth, s.display_support.warmth)
            };
            let r = l.quick_row(row);
            self.draw_icon(icon, (r.x + 28) as f64, (r.y + 14) as f64, 40.0, BLACK);
            self.text(Text { text: label, x: (r.x + 92) as f32, y: (r.y + 18) as f32, w: 400.0, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
            let level = if available { value.to_string() } else { "Unavailable".into() };
            self.text(Text { text: &level, x: 0.0, y: (r.y + 22) as f32, w: (r.w - 28) as f32, size: 26.0, bold: false, align: Align::End, lines: 1, gray: 85 });
            if available {
                self.level_bar(l.quick_bar(row), value);
                self.round_button(l.quick_minus(row), false);
                self.round_button(l.quick_plus(row), true);
            }
        }
        self.quick_system(l, s);
        let handle = l.quick_handle();
        self.stroke_icon("M6 15 L12 9 L18 15", (handle.x + handle.w / 2 - 16) as f64, (handle.y + 8) as f64, 32.0, 2.4, BLACK);
        self.text(Text { text: "Swipe up to close", x: handle.x as f32, y: (handle.y + 40) as f32, w: handle.w as f32, size: 24.0, bold: false, align: Align::Center, lines: 1, gray: BLACK });
    }

    fn level_bar(&mut self, bar: Rect, level: u32) {
        let steps = QUICK_LEVEL_MAX as i32 + 1;
        let gap = (bar.w / steps / 3).clamp(0, 3);
        for n in 0..steps {
            let start = n * bar.w / steps;
            let end = (n + 1) * bar.w / steps;
            let width = end - start - if n + 1 == steps { 0 } else { gap };
            if width <= 0 {
                continue;
            }
            let segment = Rect::new(bar.x + start, bar.y, width, bar.h);
            let radius = f64::from(width.min(12)) / 2.0;
            if n as u32 <= level.min(QUICK_LEVEL_MAX) {
                self.rounded(segment, radius, BLACK);
            } else {
                let border = if self.high_contrast { 100 } else { 189 };
                self.rounded_frame(segment, radius, 2, border, WHITE);
            }
        }
    }

    /// A shared icon (`primitive-quick`) at (`x`, `y`), `size` pixels, in gray `v`: its stroked paths, then its filled ones.
    fn draw_icon(&mut self, icon: primitive_quick::Icon, x: f64, y: f64, size: f64, v: u8) {
        for d in icon.stroke {
            self.stroke_icon(d, x, y, size, 2.0, v);
        }
        for d in icon.fill {
            self.fill_svg(d, x, y, size / 24.0, v);
        }
    }

    /// The system section of quick settings: a header, then CPU, memory and storage, each with its icon, a bar of how
    /// full it is, and its value.
    fn quick_system(&mut self, l: &Layout, s: &Status) {
        let header = l.quick_system_header();
        self.text(Text { text: "System", x: 28.0 + 8.0, y: (header + 10) as f32, w: 400.0, size: 26.0, bold: true, align: Align::Start, lines: 1, gray: 60 });
        for (i, row) in s.system.rows().iter().enumerate() {
            let r = l.quick_stat_row(i);
            self.draw_icon(row.icon, 28.0, (r.y + 18) as f64, 40.0, BLACK);
            self.text(Text { text: row.label, x: 92.0, y: (r.y + 20) as f32, w: 200.0, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
            self.text(Text { text: &row.value, x: 0.0, y: (r.y + 22) as f32, w: (r.w - 28) as f32, size: 26.0, bold: false, align: Align::End, lines: 1, gray: 85 });
            let bar = Rect::new(280, r.y + 26, r.w - 280 - 340, 26);
            self.rounded_frame(bar, 13.0, 2, BLACK, WHITE);
            if let Some(f) = row.fraction {
                let inner = bar.inset(5);
                let w = (inner.w as f32 * f).round() as i32;
                if w > 0 {
                    self.rounded(Rect::new(inner.x, inner.y, w.max(inner.h), inner.h), inner.h as f64 / 2.0, BLACK);
                }
            }
        }
    }

    /// A round button with a bar (−) or a bar and a bar (+) in its middle, as Primitive Browser's level rows.
    fn round_button(&mut self, r: Rect, plus: bool) {
        self.touch(r, r.w as f64 / 2.0);
        self.rounded_frame(r, r.w as f64 / 2.0, 2, BLACK, WHITE);
        let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
        self.fill(Rect::new(cx - 18, cy - 2, 36, 4), BLACK);
        if plus {
            self.fill(Rect::new(cx - 2, cy - 18, 4, 36), BLACK);
        }
    }


    /// The home screen: status bar, the page's icons with their names, and the page dots.
    fn home(&mut self, ui: &Ui, s: &Status, icon_blits: &mut Vec<(Rect, String)>) {
        let l = ui.layout;
        self.status_strip(&l, s, ui.running);
        let page: Vec<(usize, App)> = ui.page_apps().map(|(i, a)| (i, a.clone())).collect();
        for (slot, app) in page {
            let icon = l.icon(slot);
            self.app_icon(icon, &app, s.dark_mode, icon_blits);
            if ui.store_update_count > 0 && (app.launch == Launch::Store || app.id == kl_core::apps::STORE_ENTRY_FILE) {
                let text = if ui.store_update_count > 99 { "99+".into() } else { ui.store_update_count.to_string() };
                // The badge sits above the icon so a custom picture can never cover it.
                let badge = Rect::new(icon.right() - 54, icon.y - 38, 56, 36);
                self.rounded_frame(badge, 18.0, 2, BLACK, BLACK);
                self.text(Text { text: &text, x: badge.x as f32, y: (badge.y + 1) as f32, w: badge.w as f32,
                    size: 26.0, bold: true, align: Align::Center, lines: 1, gray: WHITE });
            }
            let label = l.label(slot);
            self.text(Text {
                text: &app.name,
                x: label.x as f32,
                y: label.y as f32,
                w: label.w as f32,
                size: LABEL_SIZE,
                bold: false,
                align: Align::Center,
                lines: 1,
                gray: BLACK,
            });
        }
        self.home_footer(ui);
    }

    /// An app's icon in a rounded square: its picture in the form for the theme (blitted after rendering, and
    /// kept out of dark mode's inversion, so the form shows as its PNG), a glyph for the built-ins, or its
    /// initials.
    fn app_icon(&mut self, r: Rect, app: &App, dark_mode: bool, icon_blits: &mut Vec<(Rect, String)>) {
        if !matches!(app.icon, IconSource::None) && self.icons.contains_key(&app.id) {
            icon_blits.push((r, icon_key_for_theme(&self.icons, &app.id, dark_mode)));
        } else if !self.glyph_icon(r, &app.launch) {
            self.monogram(r, app);
        }
    }

    fn icon_radius(r: Rect) -> f64 {
        (r.w as f32 * image::ICON_RADIUS) as f64
    }

    /// The built-ins' icons, in the light form: black glyphs on a white tile with an outline, like the app icons.
    /// They are part of the page, so dark mode inverts them into the dark form (white glyphs on a black tile).
    /// Returns false for other apps.
    fn glyph_icon(&mut self, r: Rect, launch: &Launch) -> bool {
        let glyph: fn(&mut Renderer, f64, f64, f64, u8, u8) = match launch {
            Launch::Settings => Renderer::gear_in,
            Launch::KindleMode => Renderer::book,
            Launch::Extensions => Renderer::four_squares,
            Launch::Store => Renderer::bag,
            _ => return false,
        };
        self.rounded_frame(r, Self::icon_radius(r), 4, BLACK, WHITE);
        glyph(self, (r.x + r.w / 2) as f64, (r.y + r.h / 2) as f64, r.w as f64, BLACK, WHITE);
        true
    }

    /// The gear in colour `fg`, with its hole in `bg`.
    fn gear_in(&mut self, cx: f64, cy: f64, s: f64, fg: u8, bg: u8) {
        self.ctx.set_paint(gray(fg));
        for i in 0..8 {
            let turn = Affine::translate((cx, cy)) * Affine::rotate(i as f64 * std::f64::consts::FRAC_PI_4);
            self.ctx.set_transform(turn);
            let tooth = kurbo::RoundedRect::new(-0.065 * s, -0.36 * s, 0.065 * s, -0.2 * s, 0.02 * s).to_path(0.1);
            self.ctx.fill_path(&tooth);
        }
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.fill_path(&kurbo::Circle::new((cx, cy), 0.27 * s).to_path(0.1));
        self.ctx.set_paint(gray(bg));
        self.ctx.fill_path(&kurbo::Circle::new((cx, cy), 0.11 * s).to_path(0.1));
    }

    /// The Kindle UI: an open book, two pages (`fg`) with lines of text (`bg`).
    fn book(&mut self, cx: f64, cy: f64, s: f64, fg: u8, bg: u8) {
        let (pw, ph, gap) = (0.27 * s, 0.44 * s, 0.035 * s);
        for side in [-1.0, 1.0] {
            let x0 = if side < 0.0 { cx - gap / 2.0 - pw } else { cx + gap / 2.0 };
            self.ctx.set_transform(Affine::IDENTITY);
            self.ctx.set_paint(gray(fg));
            self.ctx.fill_path(&kurbo::RoundedRect::new(x0, cy - ph / 2.0, x0 + pw, cy + ph / 2.0, 0.03 * s).to_path(0.1));
            self.ctx.set_paint(gray(bg));
            for line in 0..4 {
                let y = cy - ph / 2.0 + 0.09 * s + line as f64 * 0.085 * s;
                self.ctx.fill_rect(&kurbo::Rect::new(x0 + 0.05 * s, y, x0 + pw - 0.05 * s, y + 0.025 * s));
            }
        }
    }

    /// A shopping bag: a body with a handle (`fg`), for the Store tile.
    fn bag(&mut self, cx: f64, cy: f64, s: f64, fg: u8, bg: u8) {
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.set_paint(gray(fg));
        self.ctx.fill_path(&kurbo::RoundedRect::new(cx - 0.3 * s, cy - 0.12 * s, cx + 0.3 * s, cy + 0.32 * s, 0.05 * s).to_path(0.1));
        // The handle: a frame, its inside the colour of the tile.
        self.ctx.fill_path(&kurbo::RoundedRect::new(cx - 0.16 * s, cy - 0.3 * s, cx + 0.16 * s, cy - 0.02 * s, 0.1 * s).to_path(0.1));
        self.ctx.set_paint(gray(bg));
        self.ctx.fill_path(&kurbo::RoundedRect::new(cx - 0.1 * s, cy - 0.24 * s, cx + 0.1 * s, cy - 0.06 * s, 0.05 * s).to_path(0.1));
    }

    /// Extensions: four rounded squares.
    fn four_squares(&mut self, cx: f64, cy: f64, s: f64, fg: u8, _bg: u8) {
        let (q, gap) = (0.2 * s, 0.07 * s);
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.set_paint(gray(fg));
        for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let x0 = if dx < 0.0 { cx - gap / 2.0 - q } else { cx + gap / 2.0 };
            let y0 = if dy < 0.0 { cy - gap / 2.0 - q } else { cy + gap / 2.0 };
            self.ctx.fill_path(&kurbo::RoundedRect::new(x0, y0, x0 + q, y0 + q, 0.05 * s).to_path(0.1));
        }
    }

    /// The bottom row: a notice, a battery warning, or the page dots.
    fn home_footer(&mut self, ui: &Ui) {
        let f = ui.layout.home_footer();
        let message = ui.notice.clone().or_else(|| ui.battery_message());
        if let Some(text) = message {
            let bold = ui.notice.is_none();
            self.text(Text { text: &text, x: 40.0, y: f.y as f32 + 18.0, w: f.w as f32 - 80.0, size: 30.0, bold, align: Align::Center, lines: 1, gray: BLACK });
            return;
        }
        let pages = ui.pages();
        if pages < 2 {
            return;
        }
        let (d, step) = (18, 44);
        let x0 = (f.w - (pages as i32 - 1) * step - d) / 2;
        for p in 0..pages {
            let dot = Rect::new(x0 + p as i32 * step, f.y + (f.h - d) / 2, d, d);
            if p == ui.page {
                self.rounded(dot, d as f64 / 2.0, BLACK);
            } else {
                self.rounded_frame(dot, d as f64 / 2.0, 3, BLACK, WHITE);
            }
        }
    }

    /// An app without an icon: its initials in a white rounded square with an outline.
    fn monogram(&mut self, r: Rect, app: &App) {
        let (bg, fg) = (WHITE, BLACK);
        self.rounded_frame(r, Self::icon_radius(r), 4, BLACK, bg);
        let initials: String = app
            .name
            .split_whitespace()
            .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
            .take(2)
            .flat_map(char::to_uppercase)
            .collect();
        let size = (r.w as f32 * 0.36).min(120.0);
        self.text(Text {
            text: if initials.is_empty() { "?" } else { &initials },
            x: r.x as f32,
            y: r.y as f32 + r.h as f32 / 2.0 - size * 0.68,
            w: r.w as f32,
            size,
            bold: true,
            align: Align::Center,
            lines: 1,
            gray: fg,
        });
    }

    fn button(&mut self, r: Rect, label: &str, filled: bool) {
        self.touch(r, 16.0);
        let (bg, fg) = if filled { (BLACK, WHITE) } else { (WHITE, BLACK) };
        self.rounded_frame(r, 16.0, 4, BLACK, bg);
        let size = 36.0;
        self.text(Text { text: label, x: r.x as f32, y: r.y as f32 + (r.h as f32 - size * 1.25) / 2.0, w: r.w as f32, size, bold: true, align: Align::Center, lines: 1, gray: fg });
    }

    /// Store tabs keep their controls in place while the content below them scrolls.
    fn store_list(&mut self, ui: &Ui) {
        use kl_core::store::StoreTab;
        let l = ui.layout;
        let (title, active) = match ui.store_tab {
            StoreTab::Today => ("Today", 0),
            StoreTab::Apps => ("Apps", 1),
            StoreTab::Updates => ("Updates", 2),
            StoreTab::Search => ("Search", 3),
        };
        self.text(Text { text: title, x: 40.0, y: 50.0, w: (l.width - 80) as f32, size: 84.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        let refresh = if ui.store_busy { "Refreshing" } else { "Refresh" };
        self.store_capsule(l.store_refresh(), refresh, false, BLACK);
        self.store_capsule(l.store_done(), "Exit", true, BLACK);
        let settings = l.store_settings_button();
        self.rounded_frame(settings, 50.0, 3, BLACK, WHITE);
        self.draw_icon(primitive_quick::icons::SETTINGS, (settings.x + 25) as f64, (settings.y + 25) as f64, 50.0, BLACK);
        if ui.store_has_toolbar() {
            let toolbar = l.store_toolbar_button();
            let (heading, action) = match ui.store_tab {
                StoreTab::Updates => {
                    let count = ui.store_pending_updates().count();
                    (format!("{count} {} ready", if count == 1 { "update" } else { "updates" }), "Update all")
                }
                StoreTab::Apps => match ui.store_selected_collection() {
                    Some(collection) => (collection.title.clone(), "All apps"),
                    None => (ui.store_category.as_deref().map(capitalise).unwrap_or_else(|| "All apps".to_string()), "Categories"),
                },
                _ => unreachable!("this tab has no toolbar"),
            };
            self.text(Text { text: &heading, x: 40.0, y: (toolbar.y + 26) as f32, w: (toolbar.x - 60) as f32, size: 42.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            self.store_capsule(toolbar, action, ui.store_tab == StoreTab::Updates, BLACK);
        }
        if ui.store_tab == StoreTab::Search {
            self.store_search_field(ui);
        }
        let viewport = ui.store_list_viewport();
        if viewport.h > 0 {
            self.ctx.set_transform(Affine::IDENTITY);
            self.ctx.push_clip_rect(&krect(viewport));
            if ui.store_tab == StoreTab::Today {
                self.store_today(ui, viewport);
            } else {
                self.store_rows(ui, viewport);
            }
            self.ctx.pop_clip();
            self.store_list_scrollbar(viewport, ui.store_list_scroll, ui.store_list_scroll_max());
        }
        if ui.store_search_keyboard {
            self.store_search_keyboard(ui);
        } else {
            self.store_tab_bar(&l, active);
        }
        if let Some(notice) = ui.store_list_notice() {
            self.text(Text {
                text: notice, x: 40.0, y: (viewport.bottom() + 6) as f32,
                w: (l.width - 80) as f32, size: 26.0, bold: true, align: Align::Start, lines: 2, gray: BLACK,
            });
        }
    }

    fn store_search_field(&mut self, ui: &Ui) {
        let field = ui.layout.store_search_field();
        let query = ui.store_query.string();
        let inner = field.inset(26);
        let (shown, ink) = if query.is_empty() { ("Search apps", 85) } else { (query.as_str(), BLACK) };
        self.rounded_frame(field, 22.0, 3, BLACK, WHITE);
        self.text(Text { text: shown, x: inner.x as f32, y: (inner.y + 4) as f32, w: inner.w as f32, size: 36.0, bold: false, align: Align::Start, lines: 1, gray: ink });
        if ui.store_search_keyboard && !query.is_empty() {
            let before: String = ui.store_query.text[..ui.store_query.caret].iter().collect();
            let x = (inner.x + self.text_width(&before, 36.0, false) as i32).min(inner.right());
            self.fill(Rect::new(x, inner.y + 2, 3, inner.h - 4), BLACK);
        }
    }

    fn store_search_keyboard(&mut self, ui: &Ui) {
        let l = ui.layout;
        let area = l.keyboard_area();
        self.fill(Rect::new(area.x, area.y, area.w, primitive_keyboard::style::TOP_LINE as i32), primitive_keyboard::style::TOP_LINE_GRAY);
        for (key, r) in l.keyboard_keys_with(ui.store_query.layer, "Search") {
            self.key(&key, r, ui.pressed_key == Some(r));
        }
    }

    /// The notice (what happened: a refresh, an install, a removal), on the store's screens.
    fn store_notice(&mut self, ui: &Ui, y: i32) {
        let notice = ui.notice.as_deref().or(if ui.store_busy { ui.store_progress.as_deref() } else { None });
        if let Some(notice) = notice {
            self.text(Text { text: notice, x: 40.0, y: y as f32, w: (ui.layout.width - 80) as f32, size: 30.0, bold: true, align: Align::Start, lines: 2, gray: BLACK });
        }
    }

    /// Today: a featured app, then collections or the next apps.
    fn store_today(&mut self, ui: &Ui, viewport: Rect) {
        let Some(index) = ui.store_featured() else {
            self.store_empty(viewport, "No apps yet. The store lists the apps once its catalog has been downloaded.");
            return;
        };
        let feature = ui.store_today_feature();
        if feature.bottom() > viewport.y && feature.y < viewport.bottom() {
            self.store_feature_card(&ui.store[index], feature);
        }
        let see = ui.store_today_see_all();
        let collections = ui.store_today_collections();
        if see.bottom() + 20 > viewport.y && see.y < viewport.bottom() {
            let heading = if collections.is_empty() { "Apps" } else { "Collections" };
            self.text(Text { text: heading, x: 40.0, y: see.y as f32, w: 400.0, size: 52.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            self.text(Text { text: "See all apps", x: see.x as f32, y: (see.y + 12) as f32, w: see.w as f32, size: 34.0, bold: true, align: Align::End, lines: 1, gray: BLACK });
        }
        if collections.is_empty() {
            for (row, idx) in ui.store_visible_entries().into_iter().filter(|&(_, idx)| idx != index) {
                self.store_row(&ui.store[idx], row);
            }
        } else {
            for (i, collection) in collections.iter().enumerate() {
                let card = ui.store_today_collection_card(i);
                if card.bottom() <= viewport.y || card.y >= viewport.bottom() {
                    continue;
                }
                self.card(card);
                let r = card.inset(30);
                self.text(Text { text: &collection.title, x: r.x as f32, y: r.y as f32, w: r.w as f32, size: 36.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
                self.text(Text { text: &collection.description, x: r.x as f32, y: (r.y + 66) as f32, w: r.w as f32, size: 28.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
            }
        }
    }

    fn store_feature_card(&mut self, e: &kl_core::store::StoreEntry, f: Rect) {
        self.touch(f, 48.0);
        self.rounded(f, 48.0, BLACK);
        self.text(Text { text: "Featured", x: (f.x + 56) as f32, y: (f.y + 56) as f32, w: 600.0, size: 26.0, bold: true, align: Align::Start, lines: 1, gray: WHITE });
        self.text(Text { text: &e.name, x: (f.x + 56) as f32, y: (f.y + 100) as f32, w: (f.w - 56 * 2 - 240) as f32, size: 68.0, bold: true, align: Align::Start, lines: 2, gray: WHITE });
        self.text(Text { text: &e.subtitle, x: (f.x + 56) as f32, y: (f.y + 290) as f32, w: (f.w - 56 * 2 - 240) as f32, size: 30.0, bold: false, align: Align::Start, lines: 2, gray: WHITE });
        let size = STORE_FEATURE_ICON as i32;
        let tile = Rect::new(f.right() - 56 - size, f.y + 56, size, size);
        self.store_tile(tile, e, WHITE, BLACK);
        let pill = Rect::new(f.x + 56, f.bottom() - 56 - 84, 200, 84);
        // White on the black card: a white pill with a black label.
        self.rounded(pill, pill.h as f64 / 2.0, WHITE);
        self.text(Text { text: e.action_label(), x: pill.x as f32, y: pill.y as f32 + (pill.h as f32 - 30.0 * 1.25) / 2.0, w: pill.w as f32, size: 30.0, bold: true, align: Align::Center, lines: 1, gray: BLACK });
    }

    fn store_rows(&mut self, ui: &Ui, viewport: Rect) {
        use kl_core::store::StoreTab;
        let entries = ui.store_visible_entries();
        if entries.is_empty() {
            let empty = match ui.store_tab {
                StoreTab::Updates => "Nothing from the store yet. Apps you get here, and their updates, are listed here.",
                _ if ui.store.is_empty() => "Refresh the store to find apps.",
                StoreTab::Search => "No apps match your search.",
                _ => "There are no apps in this group.",
            };
            self.store_empty(viewport, empty);
        } else {
            for (row, index) in entries {
                self.store_row(&ui.store[index], row);
            }
        }
    }

    fn store_list_scrollbar(&mut self, viewport: Rect, scroll: i32, max: i32) {
        if max <= 0 || viewport.h <= 0 {
            return;
        }
        let height = viewport.h as i64;
        let thumb = (height * height / (height + i64::from(max))).max(height.min(40));
        let travel = height - thumb;
        let top = i64::from(scroll.clamp(0, max)) * travel / i64::from(max);
        let bar = Rect::new(viewport.right() - 18, viewport.y + top as i32, 6, thumb as i32);
        self.rounded(bar, 3.0, 85);
    }

    /// A message in the middle of a tab, when there is nothing to list.
    fn store_empty(&mut self, viewport: Rect, text: &str) {
        self.text(Text { text, x: (viewport.x + 80) as f32, y: (viewport.y + (viewport.h - 120).max(0) / 3) as f32, w: (viewport.w - 160).max(0) as f32, size: 36.0, bold: false, align: Align::Center, lines: 3, gray: 85 });
    }

    /// One app's row: its icon, name and summary, and its action on the right.
    fn store_row(&mut self, e: &kl_core::store::StoreEntry, r: Rect) {
        let size = STORE_ROW_ICON as i32;
        let tile = Rect::new(r.x, r.y + (r.h - size) / 2, size, size);
        self.store_tile(tile, e, BLACK, WHITE);
        let text_x = (r.x + 164) as f32;
        let text_w = (r.w - 164 - 240) as f32;
        self.text(Text { text: &e.name, x: text_x, y: (r.y + 30) as f32, w: text_w, size: 40.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.text(Text { text: &e.subtitle, x: text_x, y: (r.y + 92) as f32, w: text_w, size: 28.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        let primary = e.can_install() || e.can_open();
        let pill = Layout::store_row_action(r);
        self.store_capsule(pill, e.action_label(), primary, BLACK);
        self.hairline(r.bottom() - 1, r.x + 164, r.right());
    }

    /// An action pill: filled (primary) or outlined, with its label centred.
    fn store_capsule(&mut self, r: Rect, label: &str, primary: bool, outline_ink: u8) {
        let radius = r.h as f64 / 2.0;
        self.touch(r, radius);
        let ink = if primary { WHITE } else { outline_ink };
        if primary {
            self.rounded(r, radius, BLACK);
        } else {
            self.rounded_frame(r, radius, 3, BLACK, WHITE);
        }
        let size = 30.0;
        self.text(Text { text: label, x: r.x as f32, y: r.y as f32 + (r.h as f32 - size * 1.25) / 2.0, w: r.w as f32, size, bold: true, align: Align::Center, lines: 1, gray: ink });
    }

    /// An app's icon tile: its picture when the store has it at this size, else a black or white tile with the
    /// app's first letter. The frame blit keeps scrolling pictures inside the viewport.
    fn store_tile(&mut self, tile: Rect, e: &kl_core::store::StoreEntry, bg: u8, ink: u8) {
        let key = store_icon_key(&e.id, tile.w as u32);
        if self.icons.contains_key(&key) {
            self.store_blits.push((tile, key));
            return;
        }
        self.rounded(tile, Self::icon_radius(tile), bg);
        self.initial(tile, &e.name, ink);
    }

    /// A tile's first letter, centred, in `ink`.
    fn initial(&mut self, tile: Rect, name: &str, ink: u8) {
        let letter = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let size = tile.h as f32 * 0.46;
        self.text(Text { text: &letter, x: tile.x as f32, y: tile.y as f32 + (tile.h as f32 - size * 1.25) / 2.0, w: tile.w as f32, size, bold: true, align: Align::Center, lines: 1, gray: ink });
    }

    /// The floating tab bar: the active tab is filled.
    fn store_tab_bar(&mut self, l: &kl_core::Layout, active: usize) {
        let t = l.store_tabs();
        self.rounded_frame(t, 65.0, 3, BLACK, WHITE);
        let names = ["Today", "Apps", "Updates", "Search"];
        for (i, name) in names.iter().enumerate() {
            let r = l.store_tab(i);
            self.touch(r, r.h as f64 / 2.0);
            let on = i == active;
            if on {
                self.rounded(r, r.h as f64 / 2.0, BLACK);
            }
            let ink = if on { WHITE } else { BLACK };
            self.tab_glyph(i, (r.x + r.w / 2) as f64, (r.y + 36) as f64, 40.0, ink);
            self.text(Text { text: name, x: r.x as f32, y: (r.y + 72) as f32, w: r.w as f32, size: 24.0, bold: true, align: Align::Center, lines: 1, gray: ink });
        }
    }

    /// The tab glyphs: a star, four squares, a tray with an arrow, and a magnifying glass.
    fn tab_glyph(&mut self, i: usize, cx: f64, cy: f64, s: f64, ink: u8) {
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.set_paint(gray(ink));
        match i {
            0 => {
                let mut p = kurbo::BezPath::new();
                for k in 0..10 {
                    let r = if k % 2 == 0 { 0.5 * s } else { 0.22 * s };
                    let a = -std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::PI / 5.0;
                    let pt = (cx + r * a.cos(), cy + r * a.sin());
                    if k == 0 {
                        p.move_to(pt);
                    } else {
                        p.line_to(pt);
                    }
                }
                p.close_path();
                self.ctx.fill_path(&p);
            }
            1 => {
                let (q, gap) = (0.4 * s, 0.14 * s);
                for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    let x0 = if dx < 0.0 { cx - gap / 2.0 - q } else { cx + gap / 2.0 };
                    let y0 = if dy < 0.0 { cy - gap / 2.0 - q } else { cy + gap / 2.0 };
                    self.ctx.fill_path(&kurbo::RoundedRect::new(x0, y0, x0 + q, y0 + q, 0.1 * s).to_path(0.1));
                }
            }
            2 => {
                self.ctx.fill_path(&kurbo::RoundedRect::new(cx - 0.07 * s, cy - 0.5 * s, cx + 0.07 * s, cy + 0.12 * s, 0.02 * s).to_path(0.1));
                let mut arrow = kurbo::BezPath::new();
                arrow.move_to((cx - 0.3 * s, cy - 0.08 * s));
                arrow.line_to((cx + 0.3 * s, cy - 0.08 * s));
                arrow.line_to((cx, cy + 0.26 * s));
                arrow.close_path();
                self.ctx.fill_path(&arrow);
                self.ctx.fill_path(&kurbo::RoundedRect::new(cx - 0.42 * s, cy + 0.36 * s, cx + 0.42 * s, cy + 0.46 * s, 0.04 * s).to_path(0.1));
            }
            _ => {
                let lens = (cx - 0.10 * s, cy - 0.10 * s);
                self.ctx.fill_path(&kurbo::Circle::new(lens, 0.36 * s).to_path(0.1));
                let mut handle = kurbo::BezPath::new();
                handle.move_to((cx + 0.08 * s, cy + 0.17 * s));
                handle.line_to((cx + 0.17 * s, cy + 0.08 * s));
                handle.line_to((cx + 0.48 * s, cy + 0.39 * s));
                handle.line_to((cx + 0.39 * s, cy + 0.48 * s));
                handle.close_path();
                self.ctx.fill_path(&handle);
                self.ctx.set_paint(gray(if ink == BLACK { WHITE } else { BLACK }));
                self.ctx.fill_path(&kurbo::Circle::new(lens, 0.24 * s).to_path(0.1));
            }
        }
    }

    fn store_install_sheet(&mut self, ui: &Ui) {
        let Some(entry) = ui.store_sheet.and_then(|index| ui.store.get(index)) else { return };
        let l = ui.layout;
        let sheet = l.store_sheet();
        let inner = sheet.inset(48);
        self.rounded_frame(sheet, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        let x = inner.x as f32;
        let y = inner.y as f32;
        let w = inner.w as f32;
        if ui.store_progress.is_none() {
            if let Some(review) = &ui.store_requirements {
                self.store_requirements_dialog(ui, entry, inner, review);
                return;
            }
        }
        self.text(Text { text: &entry.name, x, y, w, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        if let Some(progress) = &ui.store_progress {
            self.store_progress(inner, progress, ui.store_progress_percent);
            return;
        }
        if let Some(size) = entry.size {
            let size = format!("{} download", kl_core::store::format_size(size));
            self.text(Text { text: &size, x, y: y + 80.0, w, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        }
        self.text(Text { text: "What this app changes", x, y: y + 150.0, w, size: 38.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        let changes = entry.changes.join("\n");
        self.text(Text { text: &changes, x, y: y + 216.0, w, size: 30.0, bold: false, align: Align::Start, lines: 7, gray: BLACK });
        if entry.system_changes {
            self.text(Text { text: "This app may change how your Kindle starts or runs.", x, y: y + 500.0, w, size: 30.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        }
        if let Some(notice) = &ui.notice {
            self.text(Text { text: notice, x, y: y + 548.0, w, size: 28.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        }
        let [cancel, install] = l.store_sheet_buttons();
        let label = if matches!(entry.status, kl_core::store::Status::Update { .. }) { "Update" } else { "Install" };
        self.button(cancel, "Cancel", false);
        self.button(install, label, true);
    }

    fn store_requirements_dialog(&mut self, ui: &Ui, entry: &kl_core::store::StoreEntry, inner: Rect, review: &kl_core::ui::StoreRequirementReview) {
        let (x, y, w) = (inner.x as f32, inner.y as f32, inner.w as f32);
        self.text(Text { text: "Install required apps?", x, y, w, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        let explanation = format!("The store will install or update these apps before installing {}.", entry.name);
        self.text(Text { text: &explanation, x, y: y + 82.0, w, size: 30.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
        for (i, app) in review.visible_apps().iter().enumerate() {
            let top = inner.y + 170 + i as i32 * 102;
            let version = app.latest.as_deref().unwrap_or_default();
            let action = app.action_label();
            let action = if action == "Get" { "Install" } else { action };
            let title = format!("{action} {} {version}", app.name);
            self.text(Text { text: &title, x, y: top as f32, w, size: 33.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            let size = app.size.map(kl_core::store::format_size).unwrap_or_default();
            let change = if app.system_changes {
                "May change how your Kindle starts or runs."
            } else {
                app.changes.first().map(String::as_str).unwrap_or_default()
            };
            let details = format!("{size} · {change}");
            self.text(Text { text: &details, x, y: (top + 44) as f32, w, size: 26.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
            self.hairline(top + 91, inner.x, inner.right());
        }
        if review.pages() > 1 {
            let [previous, next] = ui.layout.store_requirements_nav();
            if review.page > 0 {
                self.store_capsule(previous, "Previous", false, BLACK);
            }
            if review.page + 1 < review.pages() {
                self.store_capsule(next, "Next", false, BLACK);
            }
            let page = format!("{} of {}", review.page + 1, review.pages());
            self.text(Text { text: &page, x, y: (previous.y + 23) as f32, w, size: 28.0, bold: false, align: Align::Center, lines: 1, gray: 85 });
        }
        let [cancel, install] = ui.layout.store_sheet_buttons();
        self.button(cancel, "Cancel", false);
        self.button(install, "Install apps", true);
    }

    fn store_progress(&mut self, inner: Rect, message: &str, percent: Option<u8>) {
        self.text(Text { text: message, x: inner.x as f32, y: (inner.y + 136) as f32, w: inner.w as f32, size: 38.0, bold: true, align: Align::Start, lines: 2, gray: BLACK });
        if let Some(percent) = percent {
            let gap = 12;
            let width = (inner.w - 9 * gap) / 10;
            for i in 0..10 {
                let segment = Rect::new(inner.x + i * (gap + width), inner.y + 264, width, 36);
                if i < i32::from(percent / 10) {
                    self.rounded(segment, 8.0, BLACK);
                } else {
                    self.rounded_frame(segment, 8.0, 2, BLACK, WHITE);
                }
            }
        }
        self.text(Text { text: "Keep this screen open until the install finishes.", x: inner.x as f32, y: (inner.y + 362) as f32, w: inner.w as f32, size: 30.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
    }

    fn store_settings_dialog(&mut self, ui: &Ui) {
        let l = ui.layout;
        let panel = l.store_settings_box();
        let inner = panel.inset(48);
        self.rounded_frame(panel, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        let (x, y, w) = (inner.x as f32, inner.y as f32, inner.w as f32);
        let [(_, cancel), (_, done)] = l.confirm_buttons();
        let title = if ui.store_clear_confirm { "Clear saved catalog?" } else { "Store settings" };
        self.text(Text { text: title, x, y, w, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        if ui.store_clear_confirm {
            let explanation = "The store will download the catalog again the next time it refreshes. Installed apps stay on this Kindle.";
            self.text(Text { text: explanation, x, y: y + 120.0, w, size: 34.0, bold: false, align: Align::Start, lines: 4, gray: BLACK });
            self.button(cancel, "Cancel", false);
            self.button(done, "Clear catalog", true);
        } else {
            let row = l.store_daily_check_button();
            self.card(row);
            self.text(Text { text: "Check for updates daily", x: (row.x + 32) as f32, y: (row.y + 34) as f32, w: (row.w - 230) as f32, size: 34.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            self.switch(switch_rect(row), ui.store_daily_check);
            self.button(l.store_clear_button(), "Clear saved catalog", false);
            if let Some(notice) = &ui.notice {
                self.text(Text { text: notice, x, y: y + 344.0, w, size: 28.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            }
            self.button(done, "Done", true);
        }
    }

    fn store_category_dialog(&mut self, ui: &Ui) {
        let l = ui.layout;
        let panel = l.store_sheet();
        let inner = panel.inset(48);
        self.rounded_frame(panel, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        self.text(Text { text: "Categories", x: inner.x as f32, y: inner.y as f32, w: inner.w as f32, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.store_capsule(l.store_category_cell(0).inset(3), "All apps", ui.store_category.is_none(), BLACK);
        for (i, category) in ui.store_categories().into_iter().enumerate() {
            let selected = ui.store_category.as_deref() == Some(category);
            self.store_capsule(l.store_category_cell(i + 1).inset(3), &capitalise(category), selected, BLACK);
        }
        let [_, done] = l.store_sheet_buttons();
        self.button(done, "Done", true);
    }

    fn store_batch_dialog(&mut self, ui: &Ui) {
        let l = ui.layout;
        let panel = l.store_sheet();
        let inner = panel.inset(48);
        self.rounded_frame(panel, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        let (x, y, w) = (inner.x as f32, inner.y as f32, inner.w as f32);
        self.text(Text { text: "Update all apps", x, y, w, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        if let Some(progress) = &ui.store_progress {
            self.store_progress(inner, progress, ui.store_progress_percent);
            return;
        }
        let mut text = ui.store_pending_updates().map(|app| app.name.as_str()).collect::<Vec<_>>().join(", ");
        text.insert_str(0, "Update these apps?\n\n");
        self.text(Text { text: &text, x, y: y + 114.0, w, size: 38.0, bold: false, align: Align::Start, lines: 6, gray: BLACK });
        if ui.store_pending_updates().any(|app| app.system_changes) {
            self.text(Text { text: "Some updates may change how your Kindle starts or runs.", x, y: y + 425.0, w, size: 30.0, bold: true, align: Align::Start, lines: 2, gray: BLACK });
        }
        if let Some(notice) = &ui.notice {
            self.text(Text { text: notice, x, y: y + 540.0, w, size: 28.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        }
        let [cancel, update] = l.store_sheet_buttons();
        self.button(cancel, "Cancel", false);
        self.button(update, "Update all", true);
    }

    fn store_remove_dialog(&mut self, ui: &Ui) {
        let Some(entry) = ui.store_open.and_then(|index| ui.store.get(index)) else { return };
        let l = ui.layout;
        let sheet = l.confirm_box();
        let inner = sheet.inset(48);
        self.rounded_frame(sheet, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        self.text(Text { text: "Remove from this Kindle?", x: inner.x as f32, y: inner.y as f32, w: inner.w as f32, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        let consequence = if entry.system_changes {
            "Removing it may change how your Kindle starts or runs."
        } else {
            "You can install it again from the store."
        };
        let explanation = format!("This removes {} from this Kindle. {consequence}", entry.name);
        self.text(Text { text: &explanation, x: inner.x as f32, y: (inner.y + 100) as f32, w: inner.w as f32, size: 36.0, bold: false, align: Align::Start, lines: 4, gray: BLACK });
        let [(_, cancel), (_, remove)] = l.confirm_buttons();
        self.button(cancel, "Cancel", false);
        self.button(remove, "Remove", true);
    }

    /// A product page. The header (back) stays at the top; the rest scrolls by `ui.store_scroll`.
    fn store_app(&mut self, ui: &Ui) {
        let l = ui.layout;
        let Some(index) = ui.store_open else { return };
        let Some(e) = ui.store.get(index) else { return };
        let scroll = ui.store_scroll;
        let w = (l.width - 80) as f32;

        let hero = l.store_hero(scroll);
        self.store_tile(hero, e, BLACK, WHITE);
        let name = l.store_name(scroll);
        self.text(Text { text: &e.name, x: 40.0, y: name.y as f32, w, size: 56.0, bold: true, align: Align::Center, lines: 2, gray: BLACK });
        let sub = l.store_subtitle(scroll);
        self.text(Text { text: &e.subtitle, x: 40.0, y: sub.y as f32, w, size: 32.0, bold: false, align: Align::Center, lines: 2, gray: 85 });
        let label = if ui.store_installing.as_deref() == Some(e.id.as_str()) {
            if ui.store_removing { "Removing" } else { "Installing" }
        } else {
            e.action_label()
        };
        self.store_capsule(l.store_action(scroll), label, e.can_install() || e.can_open() || e.kind == kl_core::store::Kind::Link, BLACK);
        let dev = l.store_developer(scroll);
        let caption = e.availability.clone().unwrap_or_else(|| {
            if e.developer.is_empty() { e.status_line() } else { format!("Offered by {}", e.developer) }
        });
        self.text(Text { text: &caption, x: 40.0, y: dev.y as f32, w, size: 26.0, bold: false, align: Align::Center, lines: 1, gray: 85 });

        // The information strip, split by hairlines.
        let cells = l.store_info_cells(scroll);
        let version = e.latest.clone().unwrap_or_else(|| "—".to_string());
        let size = e.size.map(kl_core::store::format_size).unwrap_or_else(|| "—".to_string());
        let firmware = if e.firmware_min.is_empty() { "—".to_string() } else { format!("{}+", e.firmware_min) };
        let strip = [
            ("VERSION", version),
            ("SIZE", size),
            ("FIRMWARE", firmware),
            ("DOWNLOADS", download_count_label(e.downloads)),
        ];
        self.hairline(cells[0].y, 40, l.width - 40);
        self.hairline(cells[0].bottom() - 1, 40, l.width - 40);
        for (i, (cell, (label, value))) in cells.iter().zip(strip.iter()).enumerate() {
            if i > 0 {
                self.fill(Rect::new(cell.x, cell.y + 28, 2, cell.h - 56), 200);
            }
            self.text(Text { text: label, x: cell.x as f32, y: (cell.y + 26) as f32, w: cell.w as f32, size: 24.0, bold: true, align: Align::Center, lines: 1, gray: 60 });
            let value_width = (cell.w - 16) as f32;
            let text_width = self.text_width(value, 36.0, true).max(1.0);
            let size = 36.0 * (value_width / text_width).min(1.0);
            self.text(Text { text: value, x: (cell.x + 8) as f32, y: (cell.y + 68) as f32, w: value_width, size, bold: true, align: Align::Center, lines: 1, gray: BLACK });
        }

        // Screenshots, when the store has them for this app (from the cache, or the desktop preview).
        if e.screenshots > 0 {
            let key = store_shot_key(&e.id, ui.store_shot_index);
            let r = l.store_shot(scroll);
            if self.icons.contains_key(&key) && r.y < l.height - 120 && r.bottom() > 170 {
                self.store_blits.push((r, key));
            }
            if ui.store_shot_index > 0 {
                self.store_capsule(l.store_shot_button(scroll, false), "Previous", false, BLACK);
            }
            if ui.store_shot_index + 1 < e.screenshots {
                self.store_capsule(l.store_shot_button(scroll, true), "Next", false, BLACK);
            }
        }

        let about = l.store_about(scroll, e.screenshots);
        self.text(Text { text: "About", x: 40.0, y: (about.y - 74) as f32, w, size: 48.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.text(Text { text: &e.description, x: 40.0, y: about.y as f32, w, size: 30.0, bold: false, align: Align::Start, lines: 7, gray: BLACK });

        for (i, version) in e.versions.iter().enumerate() {
            let r = l.store_version_row(i, scroll, e.screenshots);
            if i <= 1 {
                let heading = if i == 0 { "What's new" } else { "Version history" };
                self.text(Text { text: heading, x: r.x as f32, y: (r.y - 74) as f32, w, size: 48.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            }
            let label = format!("Version {} · {}", version.version, version.date);
            self.text(Text { text: &label, x: r.x as f32, y: r.y as f32, w, size: 32.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            self.text(Text { text: &version.notes, x: r.x as f32, y: (r.y + 56) as f32, w, size: 28.0, bold: false, align: Align::Start, lines: 4, gray: 85 });
        }
        let rows = e.page_rows();
        if !rows.is_empty() {
            let first = l.store_info_row(0, scroll, e.screenshots, e.versions.len());
            self.text(Text { text: "Information", x: 40.0, y: (first.y - 74) as f32, w, size: 48.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        }
        for (i, (label, value)) in rows.iter().enumerate() {
            let r = l.store_info_row(i, scroll, e.screenshots, e.versions.len());
            let remove = label == kl_core::store::StoreEntry::REMOVE_LABEL;
            self.text(Text { text: label, x: r.x as f32, y: (r.y + 34) as f32, w: (r.w * 2 / 3) as f32, size: 30.0, bold: remove, align: Align::Start, lines: 1, gray: BLACK });
            if !value.is_empty() {
                self.text(Text { text: value, x: (r.x + r.w / 3) as f32, y: (r.y + 34) as f32, w: (r.w - r.w / 3) as f32, size: 30.0, bold: false, align: Align::End, lines: 1, gray: 85 });
            }
            self.hairline(r.bottom() - 1, r.x, r.right());
        }

        // The header covers what has scrolled under it, with a rule once the page has moved.
        self.fill(Rect::new(0, 0, l.width, 170), WHITE);
        if scroll > 0 {
            self.fill(Rect::new(0, 169, l.width, 2), 200);
        }
        let back = l.store_back();
        self.fill(Rect::new(0, l.height - 120, l.width, 120), WHITE);
        self.store_notice(ui, l.height - 110);
        self.text(Text { text: "‹  Store", x: back.x as f32, y: (back.y + (back.h - 40 * 5 / 4) / 2) as f32, w: back.w as f32, size: 40.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
    }

    fn extensions(&mut self, ui: &Ui) {
        let l = ui.layout;
        let title = l.title();
        let heading = ui.ext_title();
        let name_w = (l.done_button().x - 80) as f32;
        self.text(Text { text: &heading, x: 40.0, y: 38.0, w: name_w, size: 48.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.button(l.done_button(), if ui.ext_depth() == 0 { "Exit" } else { "Back" }, true);
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);
        let per = l.list_rows_per_page();
        let items: Vec<_> = ui.ext_items().iter().skip(ui.ext_page * per).take(per).cloned().collect();
        if items.is_empty() {
            self.text(Text { text: "Nothing here", x: 0.0, y: (title.bottom() + 80) as f32, w: title.w as f32, size: 38.0, bold: false, align: Align::Center, lines: 1, gray: BLACK });
        }
        for (i, item) in items.iter().enumerate() {
            let r = l.list_row(i);
            let chevron_w = if item.is_submenu() { 60.0 } else { 0.0 };
            self.text(Text { text: &item.name, x: r.x as f32 + 8.0, y: r.y as f32 + 36.0, w: r.w as f32 - 16.0 - chevron_w, size: 40.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
            if item.is_submenu() {
                self.text(Text { text: "›", x: (r.right() - 60) as f32, y: r.y as f32 + 26.0, w: 52.0, size: 56.0, bold: true, align: Align::End, lines: 1, gray: BLACK });
            }
            self.fill(Rect::new(r.x, r.bottom() - 2, r.w, 2), BLACK);
        }
        let f = l.footer();
        self.fill(Rect::new(0, f.y, f.w, 3), BLACK);
        let pages = ui.ext_pages();
        if pages > 1 {
            let label = format!("{} / {}", ui.ext_page + 1, pages);
            self.text(Text { text: &label, x: 0.0, y: f.y as f32 + 22.0, w: f.w as f32, size: 38.0, bold: true, align: Align::Center, lines: 1, gray: BLACK });
            if ui.ext_page > 0 {
                self.text(Text { text: "‹  Previous", x: 40.0, y: f.y as f32 + 26.0, w: 240.0, size: 34.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
            }
            if ui.ext_page + 1 < pages {
                self.text(Text { text: "Next  ›", x: f.w as f32 - 280.0, y: f.y as f32 + 26.0, w: 240.0, size: 34.0, bold: false, align: Align::End, lines: 1, gray: BLACK });
            }
        }
    }

    /// The app menu under its box: the home screen, with a ring around the icon being edited.
    fn app_menu(&mut self, ui: &Ui, s: &Status, icon_blits: &mut Vec<(Rect, String)>) {
        self.home(ui, s, icon_blits);
        let Some(app) = ui.menu_app().cloned() else { return };
        let anchor = ui.menu_anchor();
        let ring = anchor.inset(-10);
        self.rounded_frame(ring, Self::icon_radius(ring), 4, BLACK, WHITE);
        self.app_icon(anchor, &app, s.dark_mode, icon_blits);
    }

    /// The app menu's box beside the icon: the app's name, then one row per action (a pop-up, drawn last).
    fn app_menu_box(&mut self, ui: &Ui) {
        let l = ui.layout;
        let Some(app) = ui.menu_app().cloned() else { return };
        let (anchor, actions) = (ui.menu_anchor(), ui.menu_actions());
        let b = l.menu_box(anchor, actions.len());
        self.rounded_frame(b, POPUP_RADIUS as f64, 4, BLACK, WHITE);
        let name_row = Rect::new(b.x, b.y, b.w, l.menu_row(anchor, actions.len(), 0).y - b.y);
        self.text(Text { text: &app.name, x: (b.x + 36) as f32, y: (name_row.y + 28) as f32, w: (b.w - 72) as f32, size: 36.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        for (i, action) in actions.iter().enumerate() {
            let r = l.menu_row(anchor, actions.len(), i);
            self.fill(Rect::new(b.x + 4, r.y, b.w - 8, 2), BLACK);
            let label = match action {
                Button::MenuEarlier => "Move earlier".to_string(),
                Button::MenuLater => "Move later".to_string(),
                Button::MenuIcon => match &app.icon {
                    IconSource::File(path) => format!("Change icon ({})", path.rsplit('/').next().unwrap_or(path)),
                    _ => "Change icon".to_string(),
                },
                Button::MenuHide => "Hide".to_string(),
                _ => String::new(),
            };
            self.text(Text { text: &label, x: (r.x + 36) as f32, y: (r.y + 28) as f32, w: (r.w - 72) as f32, size: 36.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
        }
    }

    /// The network screen: the network's name, its details as rows, Connect, and Forget for a saved network.
    fn network_detail(&mut self, ui: &Ui) {
        let l = ui.layout;
        let title = l.title();
        let Some(net) = ui.wifi_target.clone() else { return };
        let name_w = (l.done_button().x - 80) as f32;
        self.text(Text { text: &net.name, x: 40.0, y: 36.0, w: name_w, size: 56.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.button(l.done_button(), "Back", true);
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);
        let card = Rect::new(MARGIN_X, l.title().bottom() + 40, self.width as i32 - 2 * MARGIN_X, 3 * 96);
        self.card(card);
        let rows = [
            ("Security", if net.secured { "Secured" } else { "Open" }.to_string()),
            ("Signal", format!("{} of {}", net.level, net.max)),
            ("Saved", if net.known { "Yes" } else { "No" }.to_string()),
        ];
        for (i, (key, value)) in rows.iter().enumerate() {
            let r = Rect::new(card.x, card.y + i as i32 * 96, card.w, 96);
            self.text(Text { text: key, x: (r.x + 32) as f32, y: (r.y + 32) as f32, w: 400.0, size: 32.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
            self.text(Text { text: value, x: (r.x + 32) as f32, y: (r.y + 32) as f32, w: (r.w - 64) as f32, size: 32.0, bold: true, align: Align::End, lines: 1, gray: BLACK });
            if i + 1 < rows.len() {
                self.hairline(r.bottom() - 1, r.x + 32, r.right() - 32);
            }
        }
        let connect = if net.secured && !net.known { "Enter password" } else { "Connect" };
        self.button(l.net_connect(), connect, true);
        if net.known {
            self.button(l.net_forget(), "Forget this network", false);
        }
        if let Some(notice) = ui.notice.clone() {
            self.text(Text { text: &notice, x: MARGIN_X as f32, y: (l.height - 120) as f32, w: (l.width - 2 * MARGIN_X) as f32, size: 30.0, bold: false, align: Align::Start, lines: 2, gray: BLACK });
        }
    }

    /// The password screen: the network's name, the password field with its caret, Show password, the keyboard (its
    /// Go key joins), and the notice under it.
    fn password_screen(&mut self, ui: &Ui) {
        let l = ui.layout;
        let title = l.title();
        self.text(Text { text: "Password", x: 40.0, y: 36.0, w: 700.0, size: 56.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.button(l.done_button(), "Back", true);
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);
        let name = ui.wifi_target.as_ref().map(|n| n.name.clone()).unwrap_or_default();
        self.text(Text { text: &name, x: MARGIN_X as f32, y: (l.title().bottom() + 24) as f32, w: (l.width - 2 * MARGIN_X) as f32, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        self.single_line_field(l.password_field(), &ui.password, "Type the password", !ui.show_password);
        if ui.wifi_hidden {
            self.text(Text { text: "Leave it empty for an open network.", x: MARGIN_X as f32, y: (l.password_notice().y - 60) as f32, w: (l.width - 2 * MARGIN_X) as f32, size: 26.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        }
        let show = l.password_show();
        self.text(Text { text: "Show password", x: show.x as f32 + 8.0, y: (show.y + 20) as f32, w: 600.0, size: 32.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.switch(switch_rect(show), ui.show_password);
        // The keyboard's area at the bottom, with the line above it, as in Primitive Browser.
        let area = l.keyboard_area();
        self.fill(Rect::new(area.x, area.y, area.w, primitive_keyboard::style::TOP_LINE as i32), primitive_keyboard::style::TOP_LINE_GRAY);
        for (key, r) in l.keyboard_keys(ui.password.layer) {
            let pressed = ui.pressed_key == Some(r);
            self.key(&key, r, pressed);
        }
        if let Some(notice) = ui.notice.clone() {
            let n = l.password_notice();
            self.text(Text { text: &notice, x: n.x as f32, y: n.y as f32, w: n.w as f32, size: 28.0, bold: false, align: Align::Center, lines: 2, gray: BLACK });
        }
    }

    /// The name screen for a network that is not in the list: the name field with its caret, a note, and the keyboard
    /// whose Go key says Next.
    fn network_name_screen(&mut self, ui: &Ui) {
        let l = ui.layout;
        let title = l.title();
        self.text(Text { text: "Other network", x: 40.0, y: 36.0, w: 800.0, size: 56.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.button(l.done_button(), "Back", true);
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);
        self.text(Text { text: "Network name", x: MARGIN_X as f32, y: (title.bottom() + 24) as f32, w: 600.0, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        let field = l.password_field();
        self.single_line_field(field, &ui.network_name, "Type the name, as it is written", false);
        let note = "For a network that does not show in the list. Next asks for its password.";
        self.text(Text { text: note, x: MARGIN_X as f32, y: (field.bottom() + 24) as f32, w: (l.width - 2 * MARGIN_X) as f32, size: 26.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
        let area = l.keyboard_area();
        self.fill(Rect::new(area.x, area.y, area.w, primitive_keyboard::style::TOP_LINE as i32), primitive_keyboard::style::TOP_LINE_GRAY);
        for (key, r) in l.keyboard_keys_with(ui.network_name.layer, "Next") {
            let pressed = ui.pressed_key == Some(r);
            self.key(&key, r, pressed);
        }
        if let Some(notice) = ui.notice.clone() {
            let n = l.password_notice();
            self.text(Text { text: &notice, x: n.x as f32, y: n.y as f32, w: n.w as f32, size: 28.0, bold: false, align: Align::Center, lines: 2, gray: BLACK });
        }
    }

    /// The SSH password screen: the password field with its caret, the rule, and the keyboard whose Go key says Set.
    fn device_name_screen(&mut self, ui: &Ui) {
        let l = ui.layout;
        let title = l.title();
        self.text(Text { text: "Device name", x: 40.0, y: 36.0, w: 800.0, size: 56.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.button(l.done_button(), "Cancel", true);
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);
        self.text(Text { text: "The name of this Kindle", x: MARGIN_X as f32, y: (title.bottom() + 24) as f32, w: 600.0, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        let field = l.password_field();
        self.single_line_field(field, &ui.device_name_entry, "Type a name", false);
        let note = "Type a name, then tap Save (bottom right). Cancel leaves without saving. 1 to 32 letters, digits or spaces.";
        self.text(Text { text: note, x: MARGIN_X as f32, y: (field.bottom() + 24) as f32, w: (l.width - 2 * MARGIN_X) as f32, size: 26.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
        let area = l.keyboard_area();
        self.fill(Rect::new(area.x, area.y, area.w, primitive_keyboard::style::TOP_LINE as i32), primitive_keyboard::style::TOP_LINE_GRAY);
        for (key, r) in l.keyboard_keys_with(ui.device_name_entry.layer, "Save") {
            let pressed = ui.pressed_key == Some(r);
            self.key(&key, r, pressed);
        }
        if let Some(notice) = ui.notice.clone() {
            let n = l.password_notice();
            self.text(Text { text: &notice, x: n.x as f32, y: n.y as f32, w: n.w as f32, size: 28.0, bold: false, align: Align::Center, lines: 2, gray: BLACK });
        }
    }

    fn ssh_password_screen(&mut self, ui: &Ui) {
        let l = ui.layout;
        let title = l.title();
        self.text(Text { text: "SSH password", x: 40.0, y: 36.0, w: 800.0, size: 56.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.button(l.done_button(), "Back", true);
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);
        self.text(Text { text: "Root password for SSH", x: MARGIN_X as f32, y: (title.bottom() + 24) as f32, w: 600.0, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        let field = l.password_field();
        self.single_line_field(field, &ui.ssh_password, "Type the password", false);
        let note = "6 to 64 letters, digits or - _ . @ # % + = !, no spaces. Connect with the address on this page.";
        self.text(Text { text: note, x: MARGIN_X as f32, y: (field.bottom() + 24) as f32, w: (l.width - 2 * MARGIN_X) as f32, size: 26.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
        let area = l.keyboard_area();
        self.fill(Rect::new(area.x, area.y, area.w, primitive_keyboard::style::TOP_LINE as i32), primitive_keyboard::style::TOP_LINE_GRAY);
        for (key, r) in l.keyboard_keys_with(ui.ssh_password.layer, "Set") {
            let pressed = ui.pressed_key == Some(r);
            self.key(&key, r, pressed);
        }
        if let Some(notice) = ui.notice.clone() {
            let n = l.password_notice();
            self.text(Text { text: &notice, x: n.x as f32, y: n.y as f32, w: n.w as f32, size: 28.0, bold: false, align: Align::Center, lines: 2, gray: BLACK });
        }
    }

    /// One key of the keyboard, in the shared look (`primitive-keyboard::style`, the same as Primitive Browser's
    /// keys): a light key with a thin grey outline, or an inked key with white text when it is on (Shift, Go).
    /// One key, in the keyboard's shared look: on, or pressed, takes the ink look (see `primitive_keyboard::look`).
    fn key(&mut self, key: &primitive_keyboard::KeyDef, r: Rect, pressed: bool) {
        use primitive_keyboard::style::*;
        let look = primitive_keyboard::look(key, pressed);
        self.rounded_frame(r, RADIUS as f64, BORDER as i32, look.line, look.fill);
        let size = label_size(&key.label) as f32;
        self.text(Text { text: &key.label, x: r.x as f32, y: r.y as f32 + r.h as f32 / 2.0 - size * 0.7, w: r.w as f32, size, bold: false, align: Align::Center, lines: 1, gray: look.ink });
    }

    /// Settings: the list of pages in groups. Each group is a header and one card; each page row shows its title, its
    /// current value under it, and a chevron. Done returns to the grid.
    fn settings_home(&mut self, ui: &Ui, s: &Status) {
        let l = ui.layout;
        self.text(Text { text: "Settings", x: 40.0, y: 36.0, w: 700.0, size: 64.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.button(l.done_button(), "Exit", true);
        let viewport = l.settings_viewport();
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.push_clip_rect(&krect(viewport));
        for (header, card, rows) in l.home_sections(ui.settings_scroll) {
            if card.bottom() <= viewport.y || card.y - 44 >= viewport.bottom() {
                continue;
            }
            if !header.is_empty() {
                self.section_header(card.x, card.y - 44, header);
            }
            self.card(card);
            let count = rows.len();
            for (i, (page, r)) in rows.into_iter().enumerate() {
                if r.bottom() <= viewport.y || r.y >= viewport.bottom() {
                    continue;
                }
                let value = page_value(page, ui, s);
                self.text(Text { text: page.title(), x: (r.x + 32) as f32, y: (r.y + 22) as f32, w: (r.w - 120) as f32, size: 38.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
                self.text(Text { text: &value, x: (r.x + 32) as f32, y: (r.y + 72) as f32, w: (r.w - 120) as f32, size: 28.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
                self.text(Text { text: "›", x: (r.right() - 80) as f32, y: (r.y + 30) as f32, w: 50.0, size: 52.0, bold: true, align: Align::End, lines: 1, gray: BLACK });
                if i + 1 < count {
                    self.hairline(r.bottom() - 1, card.x + 32, card.right() - 32);
                }
            }
        }
        self.ctx.pop_clip();
        self.settings_scrollbar(viewport, ui.settings_scroll, l.settings_scroll_max());
    }

    fn wallpaper_preview_frame(
        &mut self, layout: &Layout, status: &Status, draft: &WallpaperDraft,
        picture: Option<&GrayImage>, notice: Option<&str>,
    ) -> Vec<u8> {
        self.ctx.reset();
        self.fill(layout.screen(), WHITE);
        let area = layout.wallpaper_preview();
        let back = layout.done_button();
        let title = format!("{} preview", draft.target.title());
        self.text(Text { text: &title, x: MARGIN_X as f32, y: 36.0, w: (back.x - 2 * MARGIN_X) as f32, size: 56.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.button(back, "Back", true);
        let title = layout.title();
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);

        let label = area.choice_label;
        self.text(Text { text: draft.choice.label(draft.target), x: label.x as f32, y: label.y as f32, w: label.w as f32, size: 30.0, bold: true, align: Align::Center, lines: 1, gray: BLACK });
        self.frame(area.picture, 4, BLACK);
        if picture.is_none() {
            let hint = draft.error.as_deref().or(notice).unwrap_or(match draft.target {
                WallpaperTarget::Sleep if draft.choice == WallpaperChoice::Default => "The Kindle chooses the sleep picture.",
                WallpaperTarget::Startup if draft.choice == WallpaperChoice::Default => "Launcher uses its standard startup screen.",
                _ => "No picture available to preview.",
            });
            let frame = area.picture.inset(32);
            self.text(Text { text: hint, x: frame.x as f32, y: (frame.y + frame.h / 2 - 48) as f32, w: frame.w as f32, size: 30.0, bold: false, align: Align::Center, lines: 3, gray: 85 });
        }

        if draft.has_opacity() {
            let label = area.opacity_label;
            let text = format!("Opacity {}%", draft.opacity);
            self.text(Text { text: &text, x: label.x as f32, y: label.y as f32, w: label.w as f32, size: 32.0, bold: true, align: Align::Center, lines: 1, gray: BLACK });
            self.round_button(area.opacity_down, false);
            self.round_button(area.opacity_up, true);
            let track = area.opacity_track;
            let cy = track.y + track.h / 2;
            let bar = Rect::new(track.x, cy - 7, track.w, 14);
            self.rounded_frame(bar, 7.0, 2, BLACK, WHITE);
            let cx = track.x + 14 + (track.w - 28) * i32::from(draft.opacity) / 100;
            self.rounded(Rect::new(bar.x, bar.y, cx - bar.x, bar.h), 7.0, BLACK);
            self.rounded(Rect::new(cx - 14, cy - 14, 28, 28), 14.0, BLACK);
        }
        if draft.can_apply() {
            self.button(area.apply, "Apply", true);
        }

        let mut out = self.finish_gray();
        let width = self.width as usize;
        if status.dark_mode {
            image::invert_except(&mut out, width, &[], None);
        }
        if let Some(picture) = picture {
            blit(&mut out, width, area.picture.inset(4), picture, 0.0, None);
        }
        out
    }

    /// One page of Settings: its title and Back, then its sections (header, card of rows, footer). The Wi-Fi page
    /// adds the current network and the scan list; About is key and value text.
    fn settings_page(&mut self, ui: &Ui, s: &Status, page: kl_core::layout::Page) {
        let l = ui.layout;
        let title = l.title();
        self.text(Text { text: page.title(), x: 40.0, y: 36.0, w: 900.0, size: 56.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        if page != kl_core::layout::Page::InstalledApps || !ui.installed_busy {
            self.button(l.done_button(), "Back", true);
        }
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);
        if page == kl_core::layout::Page::About {
            self.about_card(ui, s);
            return;
        }
        if page == kl_core::layout::Page::FileTransfer {
            self.file_transfer_page(ui);
            return;
        }
        if page == kl_core::layout::Page::AppVisibility {
            self.settings_visibility(ui);
        }
        if page == kl_core::layout::Page::InstalledApps {
            self.settings_installed_apps(ui);
        }
        for section in l.page_sections(page) {
            self.section_header(section.card.x, section.header_y, section.header);
            self.card(section.card);
            let count = section.rows.len();
            for (i, row) in section.rows.iter().enumerate() {
                self.setting_row(ui, s, row);
                if i + 1 < count {
                    self.hairline(row.rect.bottom() - 1, row.rect.x + 32, row.rect.right() - 32);
                }
            }
            if !section.footer.is_empty() {
                self.text(Text { text: section.footer, x: section.card.x as f32 + 8.0, y: section.footer_y as f32, w: section.card.w as f32 - 16.0, size: 24.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
            }
        }
        if page == kl_core::layout::Page::Wifi {
            self.wifi_list(ui, s);
        }
        if page == kl_core::layout::Page::Battery {
            self.battery_history(l, s);
        }
        // A result of an action on this page (a Wi-Fi join, an SSH check or password): shown at the bottom on every page.
        if let Some(notice) = ui.notice.clone() {
            let width = (l.width - 2 * MARGIN_X) as f32;
            self.text(Text { text: &notice, x: MARGIN_X as f32, y: (l.height - 84) as f32, w: width, size: 28.0, bold: true, align: Align::Start, lines: 2, gray: BLACK });
        }
        if page == kl_core::layout::Page::InstalledApps {
            self.settings_uninstall_sheet(ui);
        }
    }

    fn file_transfer_page(&mut self, ui: &Ui) {
        let width = (ui.layout.width - 2 * MARGIN_X) as f32;
        let x = MARGIN_X as f32;
        if let Some((address, code)) = &ui.file_transfer {
            self.text(Text { text: "From a phone or computer on the same Wi-Fi, open this address and enter the code.", x, y: 210.0, w: width, size: 36.0, bold: false, align: Align::Start, lines: 3, gray: BLACK });
            let card = Rect::new(MARGIN_X, 396, width as i32, 315);
            self.card(card);
            let inner = card.inset(40);
            self.text(Text { text: address, x: inner.x as f32, y: inner.y as f32, w: inner.w as f32, size: 38.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            self.text(Text { text: "Transfer code", x: inner.x as f32, y: (inner.y + 88) as f32, w: inner.w as f32, size: 30.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
            self.text(Text { text: code, x: inner.x as f32, y: (inner.y + 138) as f32, w: inner.w as f32, size: 54.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            self.text(Text { text: "Keep this page open while you send files.", x, y: 780.0, w: width, size: 30.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
        }
        if let Some(notice) = &ui.notice {
            self.text(Text { text: notice, x, y: 980.0, w: width, size: 32.0, bold: true, align: Align::Start, lines: 3, gray: BLACK });
        }
    }

    /// One setting row: its label and its value under it, and its control on the right: a switch for on/off, round
    /// buttons for a level, a pill for "Show all", a chevron for something that opens.
    fn setting_row(&mut self, ui: &Ui, s: &Status, row: &kl_core::layout::SettingsRow) {
        let r = row.rect;
        self.touch(r, 0.0);
        let value = row_value(row.row, ui, s);
        let level = match row.row {
            Row::Frontlight if s.display_support.front_light => Some(s.light),
            Row::Warmth if s.display_support.warmth => Some(s.warmth),
            _ => None,
        };
        let label_w = if level.is_some() { (r.w / 3 - 52) as f32 } else { r.w as f32 - 360.0 };
        let text_x = (r.x + 32) as f32;
        self.text(Text { text: row.row.label(), x: text_x, y: (r.y + 20) as f32, w: label_w, size: 36.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.text(Text { text: &value, x: text_x, y: (r.y + 66) as f32, w: label_w, size: 27.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
        if let Some(value) = level {
            self.level_bar(kl_core::layout::setting_level_bar(r), value);
        }
        match row.row {
            Row::BatteryReading => {
                if let Some(percent) = s.power.percent {
                    self.level_meter(r, u64::from(percent), 100);
                }
            }
            Row::StorageUsage if ui.usb != kl_core::Usb::Drive => {
                if let Some((used, total)) = s.system.storage {
                    self.level_meter(r, used, total);
                }
            }
            _ => {}
        }
        for &(b, rect) in &row.buttons {
            match b {
                Button::LightDown | Button::WarmDown if level.is_some() => self.round_button(rect, false),
                Button::LightUp | Button::WarmUp if level.is_some() => self.round_button(rect, true),
                Button::RotationAuto if s.display_support.auto_rotation => self.switch(rect, s.auto_rotation),
                Button::ShowHidden => self.button(rect, "Show all", false),
                Button::SyncClock if s.clock_sync_available && s.wifi_on && s.wifi_state == "connected" && !ui.clock_syncing => {
                    self.button(Rect::new(r.right() - 240, r.y + 28, 200, 64), "Sync", false);
                }
                Button::RefreshTimeZone => self.button(Rect::new(r.right() - 240, r.y + 28, 200, 64), "Refresh", false),
                Button::OpenPage(_) | Button::OpenWallpaperPicker(_) | Button::ManageInstalled | Button::GridColumns | Button::GridRows => self.chevron(r),
                Button::LowPower => self.switch(switch_rect(r), s.low_power),
                Button::KindleUi => self.chevron(r),
                Button::Restart => {}
                Button::Rotation => {}
                Button::Wifi => self.switch(switch_rect(r), s.wifi_on),
                Button::DarkMode => self.switch(switch_rect(r), s.dark_mode),
                Button::Animation => self.switch(switch_rect(r), s.animation),
                Button::LargeText => self.switch(switch_rect(r), s.large_text),
                Button::HighContrast => self.switch(switch_rect(r), s.high_contrast),
                Button::Ssh => self.switch(switch_rect(r), s.ssh),
                Button::Autostart => self.switch(switch_rect(r), s.autostart),
                Button::SleepScreen => self.switch(switch_rect(r), s.sleep_launcher),
                Button::StoreDailyCheck => self.switch(switch_rect(r), ui.store_daily_check),
                _ => {}
            }
        }
    }

    /// The Wi-Fi page's network sections: the current network, then the scan list with its Scan button.
    fn wifi_list(&mut self, ui: &Ui, s: &Status) {
        let l = ui.layout;
        let area = l.wifi_area();
        self.section_header(area.current.x, area.current_header, "Current network");
        self.card(area.current);
        match &ui.wifi.current {
            Some(net) => {
                let detail = format!("Connected · {} of {} signal{}", net.level, net.max, if net.secured { " · secured" } else { "" });
                self.text(Text { text: &net.name, x: (area.current.x + 32) as f32, y: (area.current.y + 20) as f32, w: (area.current.w - 220) as f32, size: 36.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
                self.text(Text { text: &detail, x: (area.current.x + 32) as f32, y: (area.current.y + 66) as f32, w: (area.current.w - 220) as f32, size: 27.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
                let lit = kl_core::wifi::arcs(net.level, net.max);
                self.wifi_icon((area.current.right() - 110) as f64, (area.current.y + 28) as f64, 56.0, Some(lit));
                self.text(Text { text: "›", x: (area.current.right() - 52) as f32, y: (area.current.y + 30) as f32, w: 40.0, size: 52.0, bold: true, align: Align::End, lines: 1, gray: BLACK });
            }
            None => {
                let text = if s.wifi_on { "Not connected" } else { "Wi-Fi is off" };
                self.text(Text { text, x: (area.current.x + 32) as f32, y: (area.current.y + 40) as f32, w: area.current.w as f32 - 64.0, size: 36.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            }
        }
        let total = ui.wifi_networks().len();
        let heading = if total == 0 { "Networks".to_string() } else { format!("Networks · {total}") };
        self.section_header(area.current.x, area.networks_header, &heading);
        self.button(area.scan, if ui.wifi_scanning { "Scanning…" } else { "Scan" }, false);
        self.button(l.wifi_other(), "Other…", false);
        self.card(area.list);
        let networks = ui.wifi_networks();
        if networks.is_empty() {
            let text = if !s.wifi_on {
                "Wi-Fi is off. Turn it on to see the networks around."
            } else if ui.wifi_scanning {
                "Looking for networks…"
            } else {
                "No networks found. Tap Scan to look again."
            };
            self.text(Text { text, x: (area.list.x + 32) as f32, y: (area.list.y + 36) as f32, w: area.list.w as f32 - 64.0, size: 30.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
            return;
        }
        for i in 0..area.visible {
            let Some(net) = networks.get(ui.wifi_scroll + i) else { break };
            let r = l.network_row(i);
            let name_w = (r.w - 260) as f32;
            self.text(Text { text: &net.name, x: (r.x + 32) as f32, y: (r.y + 18) as f32, w: name_w, size: 34.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
            if net.known {
                self.text(Text { text: "Saved", x: (r.x + 32) as f32, y: (r.y + 60) as f32, w: name_w, size: 24.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
            }
            if net.secured {
                self.lock_icon((r.right() - 150) as f64, (r.y + 34) as f64);
            }
            self.wifi_icon((r.right() - 100) as f64, (r.y + 26) as f64, 52.0, Some(kl_core::wifi::arcs(net.level, net.max)));
            if i + 1 < area.visible && ui.wifi_scroll + i + 1 < networks.len() {
                self.hairline(r.bottom() - 1, r.x + 32, r.right() - 32);
            }
        }
        // A scroll bar when the list is longer than the window: the thumb is the share of the list in view.
        if networks.len() > area.visible {
            let track = Rect::new(area.list.right() - 16, area.list.y + 16, 6, area.list.h - 32);
            self.fill(track, 200);
            let len = networks.len() as i32;
            let shown = area.visible as i32;
            let thumb_h = (track.h * shown / len).max(48);
            let max_scroll = (len - shown).max(1);
            let thumb_y = track.y + (track.h - thumb_h) * ui.wifi_scroll as i32 / max_scroll;
            self.rounded(Rect::new(track.x, thumb_y, track.w, thumb_h), 3.0, BLACK);
        }
    }

    /// System, Launcher version and device details, as key and value rows.
    fn about_card(&mut self, ui: &Ui, s: &Status) {
        let l = ui.layout;
        let top = l.title().bottom() + 24;
        let or_unknown = |value: &Option<String>| value.clone().unwrap_or_else(|| "Unknown".into());
        let rows = [
            ("System", SYSTEM_NAME.into()),
            ("Launcher version", ui.version.clone()),
            ("Device name", ui.device_name.clone().unwrap_or_else(|| "Not set".into())),
            ("Device profile", ui.device_profile.clone().unwrap_or_else(|| "Unavailable".into())),
            ("Kindle firmware", ui.firmware.clone().unwrap_or_else(|| "Unavailable".into())),
            ("Series number", or_unknown(&ui.series)),
            ("Wi-Fi MAC address", or_unknown(&ui.mac)),
            ("IP address", s.ip.clone()),
            ("Screen panel", or_unknown(&ui.panel)),
            ("USB", settings_management::usb_status(ui.usb).into()),
            ("Apps", ui.apps().len().to_string()),
            ("Memory", if s.memory.is_empty() { "unknown".into() } else { s.memory.clone() }),
            ("Storage", s.storage.clone()),
        ];
        self.section_header(MARGIN_X, top, "Device information");
        let row_height = 80;
        let card = Rect::new(MARGIN_X, top + 56, self.width as i32 - 2 * MARGIN_X, rows.len() as i32 * row_height);
        self.card(card);
        for (i, (key, value)) in rows.iter().enumerate() {
            let r = Rect::new(card.x, card.y + i as i32 * row_height, card.w, row_height);
            self.text(Text { text: key, x: (r.x + 32) as f32, y: (r.y + 32) as f32, w: 400.0, size: 32.0, bold: false, align: Align::Start, lines: 1, gray: BLACK });
            self.text(Text { text: value, x: (r.x + 32) as f32, y: (r.y + 32) as f32, w: (r.w - 64) as f32, size: 32.0, bold: true, align: Align::End, lines: 1, gray: BLACK });
            if i + 1 < rows.len() {
                self.hairline(r.bottom() - 1, r.x + 32, r.right() - 32);
            }
        }
        self.text(Text { text: "From the Kindle UI, open “Launcher” in the library to come back.", x: (card.x + 8) as f32, y: (card.bottom() + 16) as f32, w: (card.w - 16) as f32, size: 24.0, bold: false, align: Align::Start, lines: 2, gray: 85 });
    }

    /// A section header above a card: small, bold, at `x`, `y`.
    fn section_header(&mut self, x: i32, y: i32, text: &str) {
        self.text(Text { text, x: (x + 8) as f32, y: (y + 8) as f32, w: 900.0, size: 26.0, bold: true, align: Align::Start, lines: 1, gray: 60 });
    }

    /// A card: a rounded outline with white inside (the grouped list's container).
    fn card(&mut self, r: Rect) {
        self.rounded_frame(r, 22.0, 2, BLACK, WHITE);
    }

    /// A hairline between rows, from `x0` to `x1` at `y`.
    fn hairline(&mut self, y: i32, x0: i32, x1: i32) {
        self.fill(Rect::new(x0, y, x1 - x0, 2), if self.high_contrast { 105 } else { 200 });
    }

    /// A switch: a pill with a knob, black and knob-right when on; outlined and knob-left when off.
    fn switch(&mut self, r: Rect, on: bool) {
        let radius = r.h as f64 / 2.0;
        if on {
            self.rounded(r, radius, BLACK);
            self.rounded(Rect::new(r.right() - r.h + 6, r.y + 6, r.h - 12, r.h - 12), radius - 6.0, WHITE);
        } else {
            self.rounded_frame(r, radius, 3, BLACK, WHITE);
            self.rounded(Rect::new(r.x + 6, r.y + 6, r.h - 12, r.h - 12), radius - 6.0, BLACK);
        }
    }

    /// A disclosure chevron at the right of a row.
    fn chevron(&mut self, r: Rect) {
        self.text(Text { text: "›", x: (r.right() - 80) as f32, y: (r.y + 30) as f32, w: 50.0, size: 52.0, bold: true, align: Align::End, lines: 1, gray: BLACK });
    }

    /// A padlock (a secured network), at (`x`, `y`): a shackle over a body.
    fn lock_icon(&mut self, x: f64, y: f64) {
        self.rounded_frame(Rect::new(x as i32 + 5, y as i32, 14, 16), 7.0, 3, BLACK, WHITE);
        self.rounded(Rect::new(x as i32, y as i32 + 14, 24, 20), 4.0, BLACK);
    }

    /// The power menu: a box over the grid, with Cancel, Restart and Power off.
    fn power_menu_box(&mut self, l: &Layout) {
        let b = l.power_box();
        self.rounded_frame(b, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        let inner = b.inset(48);
        self.text(Text { text: "Power", x: inner.x as f32, y: inner.y as f32, w: inner.w as f32, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.text(Text { text: "Kindle UI opens the stock Kindle screens. Restart starts the Kindle again. Power off turns it off until you press its power button.", x: inner.x as f32, y: (inner.y + 80) as f32, w: inner.w as f32, size: 30.0, bold: false, align: Align::Start, lines: 3, gray: 85 });
        for (button, r) in l.power_buttons() {
            let (label, primary) = match button {
                Button::PowerCancel => ("Cancel", false),
                Button::PowerKindleUi => ("Kindle UI", true),
                Button::PowerRestart => ("Restart", true),
                _ => ("Power off", true),
            };
            self.button(r, label, primary);
        }
    }

    fn confirm(&mut self, l: &Layout) {
        let b = l.confirm_box();
        self.rounded_frame(b, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        let inner = b.inset(48);
        self.text(Text { text: "Open the Kindle UI?", x: inner.x as f32, y: inner.y as f32, w: inner.w as f32, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.text(Text {
            text: "The launcher closes and the stock Kindle UI starts, which takes about a minute. To come back, open “Launcher” in the Kindle library.",
            x: inner.x as f32,
            y: inner.y as f32 + 100.0,
            w: inner.w as f32,
            size: 36.0,
            bold: false,
            align: Align::Start,
            lines: 5,
            gray: BLACK,
        });
        let [(_, cancel), (_, open)] = l.confirm_buttons();
        self.button(cancel, "Cancel", false);
        self.button(open, "Open Kindle UI", true);
    }

    fn confirm_restart(&mut self, l: &Layout) {
        let b = l.confirm_box();
        self.rounded_frame(b, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        let inner = b.inset(48);
        self.text(Text { text: "Restart the launcher?", x: inner.x as f32, y: inner.y as f32, w: inner.w as f32, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        self.text(Text {
            text: "The app you have open closes, and the launcher starts again. Unsaved work in that app is lost.",
            x: inner.x as f32,
            y: inner.y as f32 + 100.0,
            w: inner.w as f32,
            size: 36.0,
            bold: false,
            align: Align::Start,
            lines: 4,
            gray: BLACK,
        });
        let [(_, cancel), (_, restart)] = l.confirm_buttons();
        self.button(cancel, "Cancel", false);
        self.button(restart, "Restart", true);
    }

    /// A full-screen message (e.g. "Starting the Kindle UI"): a title and a body, centred.
    pub fn message(&mut self, title: &str, body: &str) -> Vec<u8> {
        self.ctx.reset();
        let (w, h) = (self.width as i32, self.ctx.height() as i32);
        self.fill(Rect::new(0, 0, w, h), WHITE);
        self.text(Text { text: title, x: 60.0, y: h as f32 / 2.0 - 140.0, w: w as f32 - 120.0, size: 64.0, bold: true, align: Align::Center, lines: 2, gray: BLACK });
        self.text(Text { text: body, x: 80.0, y: h as f32 / 2.0, w: w as f32 - 160.0, size: 38.0, bold: false, align: Align::Center, lines: 4, gray: BLACK });
        self.finish_gray()
    }

    /// The launcher's own library icon (600 × 920): a 3 × 3 grid of rounded-square tiles, as on the home screen,
    /// with `logo` (the Primitive logo) in the centre tile, and the name.
    pub fn launcher_icon(&mut self, logo: Option<&GrayImage>) -> Vec<u8> {
        self.ctx.reset();
        let (w, h) = (self.width as i32, self.ctx.height() as i32);
        self.fill(Rect::new(0, 0, w, h), WHITE);
        self.rounded_frame(Rect::new(30, 30, w - 60, h - 60), 40.0, 10, BLACK, WHITE);
        let (cell, gap) = (140, 26);
        let (gx, gy) = ((w - 3 * cell - 2 * gap) / 2, 110);
        let tile = |i: i32| Rect::new(gx + (i % 3) * (cell + gap), gy + (i / 3) * (cell + gap), cell, cell);
        for i in 0..9 {
            let radius = cell as f64 * image::ICON_RADIUS as f64;
            if i == 4 && logo.is_none() {
                self.rounded(tile(i), radius, BLACK);
            } else {
                self.rounded_frame(tile(i), radius, 8, BLACK, WHITE);
            }
        }
        self.text(Text { text: "Launcher", x: 0.0, y: h as f32 - 220.0, w: w as f32, size: 84.0, bold: true, align: Align::Center, lines: 1, gray: BLACK });
        let mut out = self.finish_gray();
        if let Some(logo) = logo {
            let inner = tile(4).inset(18);
            let mut fitted = image::fit(logo, inner.w as u32, inner.h as u32);
            image::dither16(&mut fitted);
            blit(&mut out, w as usize, inner, &fitted, 0.0, None);
        }
        out
    }

    /// The sleep screen when there is no picture: only used for a battery warning (otherwise the driver draws
    /// nothing and the Kindle's screensaver, or the last screen, stays).
    fn sleep(&mut self, l: &Layout, warning: Option<String>) {
        let s = l.screen();
        self.frame(s.inset(40), 6, BLACK);
        let message = warning.unwrap_or_default();
        self.text(Text { text: "Connect the charger", x: 0.0, y: s.h as f32 / 2.0 - 120.0, w: s.w as f32, size: 80.0, bold: true, align: Align::Center, lines: 1, gray: BLACK });
        self.text(Text { text: &message, x: 60.0, y: s.h as f32 / 2.0 + 20.0, w: s.w as f32 - 120.0, size: 40.0, bold: false, align: Align::Center, lines: 2, gray: BLACK });
    }
}

/// The value a Settings list row shows for its page.
fn page_value(page: kl_core::layout::Page, ui: &Ui, s: &Status) -> String {
    use kl_core::layout::Page;
    let onoff = |on: bool| if on { "on" } else { "off" };
    match page {
        Page::Wifi => wifi_word(s),
        Page::ChooseModel => "Detected or chosen".into(),
        Page::Display => format!("Front light {} · Dark mode {}", light_description(s.light, s.display_support.front_light), onoff(s.dark_mode)),
        Page::Wallpaper => "Home, sleep, and startup pictures".into(),
        Page::Accessibility => format!("Larger text {} · Higher contrast {}", onoff(s.large_text), onoff(s.high_contrast)),
        Page::Sleep => if s.sleep_launcher { "The launcher's own".into() } else { "The Kindle's".into() },
        Page::Apps => format!("{} hidden", ui.hidden_count()),
        Page::InstalledApps => "Review and uninstall apps".into(),
        Page::AppVisibility => format!("{} hidden", ui.hidden_count()),
        Page::Battery => settings_management::battery_status(s),
        Page::Storage => settings_management::storage_status(ui, s),
        Page::Store if s.low_power && ui.store_daily_check => "Daily update check paused".into(),
        Page::Store => format!("Daily update check {}", onoff(ui.store_daily_check)),
        Page::Connection => format!("SSH {} · Public DNS {}", onoff(s.ssh), onoff(s.public_dns)),
        Page::FileTransfer => "Send documents from another device".into(),
        Page::Device => format!("Start with the Kindle {}", onoff(s.autostart)),
        Page::DateTime => "Clock and time zone".into(),
        Page::About => format!("{SYSTEM_NAME} · Launcher {}", ui.version),
    }
}

fn light_description(level: u32, available: bool) -> String {
    if available { format!("{level} / {QUICK_LEVEL_MAX}") } else { "Unavailable".into() }
}

/// The Wi-Fi state as a word: Off, On, or the network state.
fn wifi_word(s: &Status) -> String {
    match (s.wifi_on, s.wifi_state.as_str()) {
        (false, _) => "Off".into(),
        (true, "") => "On".into(),
        (true, state) => capitalise(state),
    }
}

/// The value a setting row shows.
fn row_value(row: Row, ui: &Ui, s: &Status) -> String {
    match row {
        Row::Frontlight => light_description(s.light, s.display_support.front_light),
        Row::Warmth => light_description(s.warmth, s.display_support.warmth),
        Row::LargeText => if s.large_text { "On".into() } else { "Off".into() },
        Row::HighContrast => if s.high_contrast { "On".into() } else { "Off".into() },
        Row::TextPreview => "Read at your own pace.".into(),
        Row::Wifi => wifi_word(s),
        Row::DarkMode => if s.dark_mode { "On: white on black".into() } else { "Off".into() },
        Row::WallpaperLink => "Home, sleep, and startup pictures".into(),
        Row::Wallpaper(target) => ui.wallpapers.selection(target).label(target).into(),
        Row::Animation => if s.animation { "On: a zoom when an app opens".into() } else { "Off".into() },
        Row::Rotation => match (s.auto_rotation, s.rotation) {
            (true, Rotation::Upright) => "Automatic: upright".into(),
            (true, Rotation::UpsideDown) => "Automatic: upside down".into(),
            (false, Rotation::Upright) => "Upright (fixed)".into(),
            (false, Rotation::UpsideDown) => "Upside down (fixed)".into(),
        },
        Row::Ssh => if s.ssh { "Running".into() } else { "Off".into() },
        Row::SshPassword => if s.ssh_password_set { "Set".into() } else { "Not set".into() },
        Row::SshAddress => if s.ssh && !s.ssh_address.is_empty() { s.ssh_address.clone() } else { "Off".into() },
        Row::SshCheck => "Tap to check".into(),
        Row::LocalTime => if s.clock.is_empty() || s.clock == "--:--" { "Unavailable".into() } else { s.clock.clone() },
        Row::LocalDate => s.clock_year.map_or_else(|| "Unavailable".into(), |year| format!("{} {year}", s.date)),
        Row::TimeZone => s.utc_offset.map_or_else(|| "Unavailable".into(), kl_core::tz::format_offset),
        Row::SyncClock if ui.clock_syncing => "Syncing over Wi-Fi…".into(),
        Row::SyncClock if !s.clock_sync_available => "Unavailable on this firmware".into(),
        Row::SyncClock if !s.wifi_on || s.wifi_state != "connected" => "Connect to Wi-Fi first".into(),
        Row::SyncClock => "Ready to sync".into(),
        Row::PublicDns => if s.public_dns { "On: Cloudflare and Google".into() } else { "Off".into() },
        Row::Autostart => if s.autostart { "On".into() } else { "Off".into() },
        Row::SleepScreen => if s.sleep_launcher { "The launcher's own".into() } else { "The Kindle's".into() },
        Row::Hidden => match ui.hidden_count() {
            0 => "None. Long-press to hide".into(),
            n => format!("{n} hidden"),
        },
        Row::AppVisibility => format!("{} hidden", ui.hidden_count()),
        Row::ManageInstalled => page_value(kl_core::layout::Page::InstalledApps, ui, s),
        Row::GridColumns => ui.layout.cols.to_string(),
        Row::GridRows => ui.layout.rows.to_string(),
        Row::BatteryReading => settings_management::battery_status(s),
        Row::LowPower => if s.low_power { "On".into() } else { "Off".into() },
        Row::BatteryDisplay => light_description(s.light, s.display_support.front_light),
        Row::BatteryWifi => wifi_word(s),
        Row::StoreSettings if s.low_power && ui.store_daily_check => "Daily check paused".into(),
        Row::StoreSettings => if ui.store_daily_check { "Daily check on".into() } else { "Daily check off".into() },
        Row::StorageUsage => settings_management::storage_status(ui, s),
        Row::DeviceAbout => ui.firmware.as_ref().map_or_else(|| "Device profile and firmware".into(), |version| format!("Kindle firmware {version}")),
        Row::DeviceName => ui.device_name.clone().unwrap_or_else(|| "Tap to set".into()),
        Row::DeviceModel => {
            let how = match ui.device_source.as_str() {
                "detected" => "detected",
                "selected" => "chosen",
                _ => "not detected",
            };
            let untested = if ui.device_untested { " · untested" } else { "" };
            format!("{} · {how}{untested}", ui.device_model_name)
        }
        Row::Model(i) => {
            let id = kl_core::models::MODELS[i as usize].id;
            if ui.device_model_id.as_deref() == Some(id) { "Selected".into() } else { String::new() }
        }
        Row::KindleUi => "Stock Kindle UI".into(),
        Row::StoreDailyCheck if s.low_power && ui.store_daily_check => "Paused by Low power mode".into(),
        Row::StoreDailyCheck => if ui.store_daily_check { "On".into() } else { "Off".into() },
        Row::StoreClearCache => "Remove offline catalog data".into(),
        Row::Restart => "Tap to restart".into(),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Shade the full Home photo for legibility behind the grid, status strip, and page dots.
/// App artwork and dialogs are drawn over this afterward.
fn shade_home_wallpaper(out: &mut [u8], layout: &Layout, picture: &GrayImage, dark: bool, opacity: u8) {
    let opacity = u32::from(opacity.min(100));
    if picture.width as i32 != layout.width || picture.height as i32 != layout.height || picture.pixels.len() != out.len() {
        return;
    }
    for (ink, &sample) in out.iter_mut().zip(&picture.pixels) {
        if *ink == BLACK {
            continue;
        }
        // The final dark-mode pass inverts the entire chrome. Pre-invert the photo's samples so it
        // stays a darkened picture in dark mode instead of becoming a negative.
        let distance = u32::from(if dark { sample } else { WHITE - sample });
        let steps = (6 * distance * opacity + 12_750) / 25_500;
        let shaded = (15 - steps) * 17;
        *ink = (u32::from(*ink) * shaded / 255) as u8;
    }
}

fn download_count_label(downloads: Option<u64>) -> String {
    let Some(count) = downloads else { return "Unavailable".into() };
    let digits = count.to_string();
    let mut label = String::with_capacity(digits.len() + (digits.len() - 1) / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            label.push(',');
        }
        label.push(digit);
    }
    label
}

/// Copy a tile into the screen buffer at `r`, clipped to the tile, the screen and `clip`, and to rounded corners of
/// `radius` (0 for square corners).
fn blit(out: &mut [u8], width: usize, r: Rect, tile: &GrayImage, radius: f32, clip: Option<Rect>) {
    let height = (out.len() / width) as i32;
    let clip = clip.unwrap_or(Rect::new(0, 0, width as i32, height));
    let left = r.x.max(clip.x).max(0);
    let right = r.right().min(r.x + tile.width as i32).min(clip.right()).min(width as i32);
    if left >= right {
        return;
    }
    let top = r.y.max(clip.y).max(0);
    let bottom = r.bottom().min(r.y + tile.height as i32).min(clip.bottom()).min(height);
    let n = (right - left) as usize;
    for y in top..bottom {
        let source = (y - r.y) as usize * tile.width as usize + (left - r.x) as usize;
        let destination = y as usize * width + left as usize;
        let src = &tile.pixels[source..source + n];
        let dst = &mut out[destination..destination + n];
        if radius == 0.0 {
            dst.copy_from_slice(src);
            continue;
        }
        for (i, (ink, &sample)) in dst.iter_mut().zip(src).enumerate() {
            if image::rect_distance(left - r.x + i as i32, y - r.y, r.w, r.h, radius) < 0.0 {
                *ink = sample;
            }
        }
    }
}

/// Blit an icon tile at `r` (inside its rounded square only over a wallpaper, so the corners show it) and return
/// its rounded square, which dark mode keeps as it is: the tile is already in the form for the theme.
/// An icon's tile with its picture rectangle and corner radius (the tests draw icons this way).
#[cfg(test)]
fn blit_icon(out: &mut [u8], width: usize, r: Rect, tile: &GrayImage, over_wallpaper: bool) -> (image::Area, f32) {
    let radius = r.w as f32 * image::ICON_RADIUS;
    blit(out, width, r, tile, if over_wallpaper { radius } else { 0.0 }, None);
    ((r.x, r.y, r.w, r.h), radius)
}

/// Turn a screen buffer upside down (rotation by 180°), in place.
pub fn turn_half(buf: &mut [u8]) {
    buf.reverse();
}

/// The pixels to show for `rotation`, given the framebuffer's own rotation (`fb_turn`, 0 or 2).
pub fn for_panel(mut buf: Vec<u8>, rotation: Rotation, fb_turn: u32) -> Vec<u8> {
    if (rotation.turn() + fb_turn) % 4 == 2 {
        turn_half(&mut buf);
    }
    buf
}

/// The rows that differ between two frames: `(first, count)`, or `None` when equal.
pub fn changed_rows(old: &[u8], new: &[u8], width: usize) -> Option<(usize, usize)> {
    if old.len() != new.len() {
        return Some((0, new.len() / width));
    }
    let rows = new.len() / width;
    let first = (0..rows).find(|&y| old[y * width..(y + 1) * width] != new[y * width..(y + 1) * width])?;
    let last = (0..rows).rev().find(|&y| old[y * width..(y + 1) * width] != new[y * width..(y + 1) * width])?;
    Some((first, last - first + 1))
}

/// The box around every changed pixel: `(x, y, w, h)`, with `x` and `w` on 8-pixel columns (the e-ink
/// driver's update regions), or `None` when the frames are equal. A frame of another size changes everything.
pub fn changed_rect(old: &[u8], new: &[u8], width: usize) -> Option<(usize, usize, usize, usize)> {
    let rows = new.len() / width;
    if old.len() != new.len() {
        return Some((0, 0, width, rows));
    }
    let (y, h) = changed_rows(old, new, width)?;
    let (mut left, mut right) = (width, 0);
    for row in y..y + h {
        let (o, n) = (&old[row * width..(row + 1) * width], &new[row * width..(row + 1) * width]);
        if let Some(first) = (0..left).find(|&x| o[x] != n[x]) {
            left = first;
        }
        if let Some(last) = (right..width).rev().find(|&x| o[x] != n[x]) {
            right = right.max(last + 1);
        }
    }
    let x = left / 8 * 8;
    let end = right.div_ceil(8) * 8;
    Some((x, y, end.min(width) - x, h))
}

/// Whether a band of pixels is pure black and white (the non-flashing DU waveform can draw it).
pub fn is_black_and_white(pixels: &[u8]) -> bool {
    pixels.iter().all(|&p| p == 0 || p == 255)
}


/// A sleep screen picture from a PNG file: fitted to `w` × `h` (centred, white around it) and dithered to the
/// panel's 16 grays.
pub fn sleep_image(path: &std::path::Path, w: u32, h: u32) -> Result<GrayImage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    image::picture_tile(&bytes, w, h)
}

/// The dithered rounded-square icon tile for `app`, of side `w` (`h` is the same), from its `# Icon:` data or file:
/// its light form. With `cache`, tiles are read from and written to that folder (keyed by the source bytes and
/// size). `Ok(None)` when the app has no icon.
pub fn app_icon(app: &App, w: u32, h: u32, cache: Option<&std::path::Path>) -> Result<Option<GrayImage>, String> {
    match app.icon.light_png()? {
        Some(bytes) => form_tile(&bytes, w, h, false, cache).map(Some),
        None => Ok(None),
    }
}

/// The tile of `app`'s dark form (`app_icon` for dark mode), when it has one: a bundled icon's `-dark` PNG, or
/// `icon-dark.png` beside an `icon.png`. `Ok(None)` when it has none; the light form is drawn in both themes.
pub fn dark_app_icon(app: &App, w: u32, h: u32, cache: Option<&std::path::Path>) -> Result<Option<GrayImage>, String> {
    match app.icon.dark_png() {
        Some(bytes) => form_tile(&bytes, w, h, true, cache).map(Some),
        None => Ok(None),
    }
}

fn form_tile(bytes: &[u8], w: u32, h: u32, dark: bool, cache: Option<&std::path::Path>) -> Result<GrayImage, String> {
    let key = image::form_cache_key(bytes, w, h, dark);
    if let Some(dir) = cache {
        if let Some(tile) = std::fs::read(dir.join(&key)).ok().and_then(|b| image::from_cache(&b)) {
            if (tile.width, tile.height) == (w, h) {
                return Ok(tile);
            }
        }
    }
    let tile = if dark { image::dark_icon_tile(bytes, w.min(h))? } else { image::icon_tile(bytes, w.min(h))? };
    if let Some(dir) = cache {
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join(format!("{key}.tmp"));
        if std::fs::write(&tmp, image::to_cache(&tile)).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(&key));
        }
    }
    Ok(tile)
}

/// Load the icon tiles of every app into the renderer, both forms: the light form, and the dark form when the
/// app has one (drawn in dark mode, so a theme change needs no reload). Returns the problems (bad icons draw as
/// monograms; a bad dark form leaves the light one in both themes).
pub fn load_icons(renderer: &mut Renderer, apps: &[App], layout: &Layout, cache: Option<&std::path::Path>) -> Vec<String> {
    let icon = layout.icon(0);
    let (w, h) = (icon.w as u32, icon.h as u32);
    let mut problems = Vec::new();
    for app in apps {
        renderer.icons.remove(&dark_icon_key(&app.id));
        match app_icon(app, w, h, cache) {
            Ok(Some(tile)) => renderer.set_icon(&app.id, tile),
            Ok(None) => continue,
            Err(e) => {
                problems.push(format!("{}: {e}", app.name));
                continue;
            }
        }
        match dark_app_icon(app, w, h, cache) {
            Ok(Some(tile)) => renderer.set_dark_icon(&app.id, tile),
            Ok(None) => {}
            Err(e) => problems.push(format!("{} (dark icon): {e}", app.name)),
        }
    }
    problems
}

/// Reverse the pixels of a control (black and white swapped), inside its rounded shape only, so the corners of the
/// screen behind it keep their colour.
fn invert_control(out: &mut [u8], width: usize, r: Rect, radius: f64) {
    if width == 0 {
        return;
    }
    let rows = out.len() / width;
    let x0 = r.x.max(0) as usize;
    let x1 = (r.x + r.w).clamp(0, width as i32) as usize;
    let y0 = r.y.max(0) as usize;
    let y1 = (r.y + r.h).clamp(0, rows as i32) as usize;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            if inside_rounded(x as i32 - r.x, y as i32 - r.y, r.w, r.h, radius) {
                let i = y * width + x;
                out[i] = 255 - out[i];
            }
        }
    }
}

/// Whether the point (x, y), measured from a rectangle's top-left corner, lies inside its rounded shape.
fn inside_rounded(x: i32, y: i32, w: i32, h: i32, radius: f64) -> bool {
    let rad = radius.min(w as f64 / 2.0).min(h as f64 / 2.0);
    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
    let corner = |cx: f64, cy: f64| (px - cx).powi(2) + (py - cy).powi(2) <= rad * rad;
    if rad <= 0.0 {
        return true;
    }
    if px < rad && py < rad {
        return corner(rad, rad);
    }
    if px > w as f64 - rad && py < rad {
        return corner(w as f64 - rad, rad);
    }
    if px < rad && py > h as f64 - rad {
        return corner(rad, h as f64 - rad);
    }
    if px > w as f64 - rad && py > h as f64 - rad {
        return corner(w as f64 - rad, h as f64 - rad);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tap on one row of a settings card reverses that row only, not the other rows or the card around them.
    #[test]
    fn a_tap_on_a_row_in_a_card_reverses_the_row_only() {
        let Ok(dir) = std::env::var("KL_TEST_FONTS") else { return };
        let fonts = vec![format!("{dir}/NotoSans-Regular.ttf"), format!("{dir}/NotoSans-Bold.ttf")];
        let (width, height) = (1264, 1680);
        let mut renderer = Renderer::new(width, height, &fonts).unwrap();
        let mut ui = Ui::new(Vec::new(), Layout::new(width, height, 3, 3));
        ui.screen = Screen::Page(kl_core::layout::Page::Display);
        let status = Status::default();
        let plain = renderer.draw(&ui, &status);
        let rows: Vec<Rect> = renderer.targets.iter().map(|(r, _)| *r).collect();
        assert!(rows.len() >= 2, "a settings page has several rows");
        let row = rows[0];
        let (x, y) = (row.x as f32 + row.w as f32 / 2.0, row.y as f32 + row.h as f32 / 2.0);
        let pressed = renderer.press_frame(&ui, &status, x, y).unwrap();
        let changed_outside = (0..height as i32)
            .flat_map(|py| (0..width as i32).map(move |px| (px, py)))
            .filter(|&(px, py)| {
                let i = (py * width as i32 + px) as usize;
                plain[i] != pressed[i] && !row.contains(px as f32 + 0.5, py as f32 + 0.5)
            })
            .count();
        assert_eq!(changed_outside, 0, "only the touched row changed");
    }

    /// A tap on a control reverses that control and nothing else. Needs the Noto Sans fonts: set `KL_TEST_FONTS` to
    /// the folder that holds them (the launcher's `assets/fonts`), otherwise the test does nothing.
    #[test]
    fn a_tap_on_a_control_reverses_that_control_only() {
        let Ok(dir) = std::env::var("KL_TEST_FONTS") else { return };
        let fonts = vec![format!("{dir}/NotoSans-Regular.ttf"), format!("{dir}/NotoSans-Bold.ttf")];
        let (width, height) = (1264, 1680);
        let mut renderer = Renderer::new(width, height, &fonts).unwrap();
        let mut ui = Ui::new(Vec::new(), Layout::new(width, height, 3, 3));
        ui.screen = Screen::Settings;
        let status = Status::default();
        let plain = renderer.draw(&ui, &status);
        let done = ui.layout.done_button();
        let (x, y) = (done.x as f32 + done.w as f32 / 2.0, done.y as f32 + done.h as f32 / 2.0);
        let pressed = renderer.press_frame(&ui, &status, x, y).expect("the Done button is a control");
        let (mut inside, mut outside) = (0, 0);
        for py in 0..height as i32 {
            for px in 0..width as i32 {
                let i = (py * width as i32 + px) as usize;
                if plain[i] == pressed[i] {
                    continue;
                }
                if done.contains(px as f32 + 0.5, py as f32 + 0.5) {
                    inside += 1;
                    assert_eq!(pressed[i], 255 - plain[i], "a pressed pixel is reversed");
                } else {
                    outside += 1;
                }
            }
        }
        assert!(inside > 0, "the pressed control changed");
        assert_eq!(outside, 0, "nothing outside the control changed");
    }

    #[test]
    fn changed_rows_finds_the_band() {
        let a = vec![0u8; 4 * 5];
        let mut b = a.clone();
        assert_eq!(changed_rows(&a, &b, 4), None);
        b[4 + 2] = 9;
        b[4 * 3] = 9;
        assert_eq!(changed_rows(&a, &b, 4), Some((1, 3)));
    }

    #[test]
    fn for_panel_turns_only_when_needed() {
        let buf = vec![1, 2, 3];
        assert_eq!(for_panel(buf.clone(), Rotation::Upright, 0), vec![1, 2, 3]);
        assert_eq!(for_panel(buf.clone(), Rotation::UpsideDown, 0), vec![3, 2, 1]);
        assert_eq!(for_panel(buf.clone(), Rotation::UpsideDown, 2), vec![1, 2, 3]);
        assert_eq!(for_panel(buf, Rotation::Upright, 2), vec![3, 2, 1]);
    }

    #[test]
    fn the_bundled_kindle_ui_icon_makes_a_tile() {
        let kindle = App {
            id: "kindle".into(),
            name: "Kindle UI".into(),
            icon: IconSource::Bundled { light: kl_core::apps::KINDLE_UI_ICON, dark: None },
            launch: Launch::KindleMode,
            dir: "/".into(),
            order: None,
            hidden: false,
            background: false,
        };
        let tile = app_icon(&kindle, 184, 184, None).unwrap().unwrap();
        assert_eq!((tile.width, tile.height), (184, 184));
        // The reader's silhouette is black; the sky above it is a mid gray.
        assert!(tile.pixels.iter().filter(|&&p| p == 0).count() > 184 * 184 / 5);
        assert!(tile.pixels.iter().any(|&p| p > 60 && p < 200));
    }

    fn bundled(id: &str, icon: IconSource) -> App {
        App { id: id.into(), name: id.into(), icon, launch: Launch::Shell { command: "true".into() }, dir: "/".into(),
            order: None, hidden: false, background: false }
    }

    fn store_app() -> App {
        let (apps, _) = kl_core::apps::merge(Vec::new(), &kl_core::Config::default());
        apps.into_iter().find(|app| app.launch == Launch::Store).unwrap()
    }

    #[test]
    fn the_theme_picks_the_dark_form_when_there_is_one() {
        let mut icons = HashMap::new();
        icons.insert("a.sh".to_string(), GrayImage::blank(4, 4));
        icons.insert("b.sh".to_string(), GrayImage::blank(4, 4));
        icons.insert(dark_icon_key("b.sh"), GrayImage::blank(4, 4));
        assert_eq!(icon_key_for_theme(&icons, "b.sh", false), "b.sh");
        assert_eq!(icon_key_for_theme(&icons, "b.sh", true), dark_icon_key("b.sh"));
        assert_eq!(icon_key_for_theme(&icons, "a.sh", true), "a.sh", "no dark form: the light one in dark mode");
    }

    #[test]
    fn a_bundled_icon_makes_a_tile_of_each_form_and_a_file_icon_takes_the_file_beside_it() {
        let store = store_app();
        let light = app_icon(&store, 184, 184, None).unwrap().unwrap();
        let dark = dark_app_icon(&store, 184, 184, None).unwrap().unwrap();
        assert_eq!((light.pixels[92 * 184 + 1], dark.pixels[92 * 184 + 1]), (BLACK, WHITE), "the outline");
        assert_eq!((light.pixels[92 * 184 + 8], dark.pixels[92 * 184 + 8]), (WHITE, BLACK), "the tile inside it");
        let dir = std::env::temp_dir().join(format!("kl-render-file-forms-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("icon.png"), kl_core::apps::KINDLE_UI_ICON).unwrap();
        let file = bundled("f.sh", IconSource::File(dir.join("icon.png").to_string_lossy().into_owned()));
        assert!(app_icon(&file, 184, 184, None).unwrap().is_some());
        assert!(dark_app_icon(&file, 184, 184, None).unwrap().is_none(), "no icon-dark.png beside it");
        std::fs::write(dir.join("icon-dark.png"), store.icon.dark_png().unwrap()).unwrap();
        assert_eq!(dark_app_icon(&file, 184, 184, None).unwrap().unwrap(), dark);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_bundled_icon_is_a_white_tile_in_light_form_and_a_black_tile_in_dark_form() {
        let dir = std::env::temp_dir().join(format!("kl-render-forms-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["Blocks", "Calculator", "Cards", "Cascade", "Chess", "Crossword", "Focus", "Hello", "Minesweeper",
            "Notes", "Ships", "Solitaire", "Sudoku", "Words"] {
            std::fs::write(dir.join(format!("{name}.sh")), "# Author: Primitive AI\n").unwrap();
        }
        let (apps, _) = kl_core::discover(&dir, &kl_core::Config::default());
        assert_eq!(apps.iter().filter(|app| matches!(app.icon, IconSource::Bundled { .. })).count(), 16);
        for app in apps.iter().filter(|app| matches!(app.icon, IconSource::Bundled { .. })) {
            let ring = |png: Vec<u8>| {
                let img = image::decode_png(&png).unwrap();
                let (w, h) = (img.width as usize, img.height as usize);
                let edge: Vec<u32> = (0..w).flat_map(|x| [x, (h - 1) * w + x]).map(|i| img.pixels[i] as u32).collect();
                edge.iter().sum::<u32>() / edge.len() as u32
            };
            assert_eq!(ring(app.icon.light_png().unwrap().unwrap()), 255, "{}: light form", app.id);
            assert_eq!(ring(app.icon.dark_png().unwrap()), 0, "{}: dark form", app.id);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_icon_in_the_form_for_the_theme_shows_as_its_png() {
        let store = store_app();
        let (size, width) = (184, 200);
        let r = Rect::new(8, 8, size as i32, size as i32);
        let centre = |screen: &[u8]| screen[(r.y as usize + 92) * width + r.x as usize + 92];
        let png_centre = |png: Vec<u8>| {
            let img = image::decode_png(&png).unwrap();
            img.pixels[(img.height / 2 * img.width + img.width / 2) as usize]
        };
        for over_wallpaper in [false, true] {
            // Light mode: the light form, drawn as it is.
            let light = app_icon(&store, size, size, None).unwrap().unwrap();
            let mut screen = vec![WHITE; width * width];
            blit_icon(&mut screen, width, r, &light, over_wallpaper);
            assert_eq!(centre(&screen), png_centre(store.icon.light_png().unwrap().unwrap()));

            // Dark mode: the dark form, kept out of the inversion (not inverted a second time).
            let dark = dark_app_icon(&store, size, size, None).unwrap().unwrap();
            let mut screen = vec![WHITE; width * width];
            let picture = blit_icon(&mut screen, width, r, &dark, over_wallpaper);
            image::invert_except(&mut screen, width, &[picture], None);
            let dark_png = store.icon.dark_png().unwrap();
            assert_eq!(centre(&screen), png_centre(dark_png.clone()));
            assert_ne!(centre(&screen), png_centre(store.icon.light_png().unwrap().unwrap()));
            for y in 0..size as usize {
                for x in 0..size as usize {
                    let inside = image::rect_distance(x as i32, y as i32, r.w, r.h, picture.1) < 0.0;
                    if inside {
                        assert_eq!(screen[(r.y as usize + y) * width + r.x as usize + x], dark.pixels[y * size as usize + x]);
                    }
                }
            }
            assert_eq!(screen[0], BLACK, "the page around it is dark");
            assert_eq!(screen[(r.y as usize + 1) * width + r.x as usize + 1], BLACK, "so is the corner outside the tile");
        }
    }

    #[test]
    fn download_counts_show_exact_totals_and_distinguish_zero_from_missing() {
        assert_eq!(download_count_label(None), "Unavailable");
        assert_eq!(download_count_label(Some(0)), "0");
        assert_eq!(download_count_label(Some(1_234_567)), "1,234,567");
        assert_eq!(download_count_label(Some(u64::MAX)), "18,446,744,073,709,551,615");
    }

    #[test]
    fn blit_stays_inside() {
        let mut out = vec![255u8; 10 * 4];
        let tile = GrayImage { width: 3, height: 3, pixels: vec![0; 9] };
        blit(&mut out, 10, Rect::new(8, 2, 3, 3), &tile, 0.0, None);
        assert_eq!(out.iter().filter(|&&p| p == 0).count(), 4);
    }

    #[test]
    fn rounded_icon_corners_show_the_wallpaper_in_both_themes() {
        let tile = GrayImage::blank(20, 20);
        let mut frame = vec![170; tile.pixels.len()];
        let radius = 20.0 * image::ICON_RADIUS;
        blit(&mut frame, 20, Rect::new(0, 0, 20, 20), &tile, radius, None);
        assert_eq!((frame[0], frame[10 * 20 + 10]), (170, WHITE));
        image::invert_except(&mut frame, 20, &[((0, 0, 20, 20), radius)], None);
        assert_eq!((frame[0], frame[10 * 20 + 10]), (85, WHITE));
    }

    #[test]
    fn wallpaper_spans_the_home_screen_with_adjustable_opacity() {
        let layout = Layout::new(128, 600, 3, 3);
        let mut picture = GrayImage::blank(128, 600);
        picture.pixels.fill(BLACK);
        let mut frame = vec![WHITE; picture.pixels.len()];
        let top = layout.status().bottom() as usize * 128;
        let bottom = layout.home_footer().y as usize * 128;
        frame[0] = BLACK;
        frame[top] = BLACK;
        frame[bottom] = BLACK;
        picture.pixels[top + 2] = WHITE;
        picture.pixels[1] = WHITE;
        picture.pixels[bottom + 2] = WHITE;
        let mut faded = frame.clone();
        let mut dark = frame.clone();
        shade_home_wallpaper(&mut faded, &layout, &picture, false, 0);
        assert_eq!(faded, frame);
        shade_home_wallpaper(&mut faded, &layout, &picture, false, 50);
        assert_eq!((faded[2], faded[top + 1], faded[bottom + 1]), (204, 204, 204));
        shade_home_wallpaper(&mut frame, &layout, &picture, false, 100);
        shade_home_wallpaper(&mut dark, &layout, &picture, true, 100);
        image::invert_except(&mut dark, 128, &[], None);
        assert_eq!(&frame[..3], &[BLACK, WHITE, 153]);
        assert_eq!(&dark[..3], &[WHITE, 102, BLACK]);
        for y in [top, bottom] {
            assert_eq!(&frame[y..y + 3], &[BLACK, 153, WHITE]);
            assert_eq!(&dark[y..y + 3], &[WHITE, BLACK, 102]);
        }
    }

    #[test]
    fn scrolling_store_picture_keeps_its_grays_inside_the_viewport_only() {
        let width = 8;
        let mut frame = vec![WHITE; width * 8];
        let tile = GrayImage {
            width: 4, height: 8,
            pixels: (0..8).flat_map(|y| [70 + y * 10; 4]).collect(),
        };
        let rect = Rect::new(1, -1, 4, 8);
        let clip = Rect::new(0, 2, width as i32, 3);
        let sample = |pixels: &[u8]| (1..=5).map(|y| pixels[y * width + 2]).collect::<Vec<_>>();
        blit(&mut frame, width, rect, &tile, 0.0, Some(clip));
        assert_eq!(sample(&frame), [WHITE, 100, 110, 120, WHITE]);
        image::invert_except_clipped(
            &mut frame, width, &[((rect.x, rect.y, rect.w, rect.h), 1.0)], None,
            Some((clip.x, clip.y, clip.w, clip.h)),
        );
        assert_eq!(sample(&frame), [BLACK, 100, 110, 120, BLACK]);
    }

    #[test]
    fn changed_rect_is_the_box_around_the_changes_on_8_pixel_columns() {
        let a = vec![0u8; 32 * 6];
        let mut b = a.clone();
        assert_eq!(changed_rect(&a, &b, 32), None);
        b[32 + 9] = 9;
        b[32 * 4 + 20] = 9;
        assert_eq!(changed_rect(&a, &b, 32), Some((8, 1, 16, 4)));
        assert_eq!(changed_rect(&a, &[0u8; 4], 32), Some((0, 0, 32, 0)));
    }
}
