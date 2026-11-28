//! Common geometry and components for native PrimitiveOS apps.

pub mod editor;
mod widgets;

pub use kl_core::{Input, Layout, MultiTouch, Rect, Rotation, Status};
pub use kl_render::component::{icons, Align, Canvas, GrayImage, Icon, TextStyle};
pub use kl_render::{changed_rect, for_panel};
pub use primitive_keyboard::{Editor, Key, Layer, Press};
pub use widgets::{
    chevron, full_screen_message, section_header, PageTurn, Pager, Row, RowAccessory, Rows, Sheet,
    SheetAction, Tab, TabBar,
};

pub const BLACK: u8 = 0;
pub const PAPER: u8 = 255;
pub const SECONDARY: u8 = 85;
pub const HAIRLINE: u8 = 200;
pub const TITLE: TextStyle = TextStyle::new(64).bold();
pub const BODY: TextStyle = TextStyle::new(36);
pub const CAPTION: TextStyle = TextStyle::new(28).ink(SECONDARY);

pub fn screen_rect(width: i32, x: i32, y: i32, w: i32, h: i32) -> Rect {
    let scale = |n: i32| ((i64::from(n) * i64::from(width) + 632) / 1264) as i32;
    Rect::new(scale(x), scale(y), scale(w), scale(h))
}

pub fn header(canvas: &mut Canvas, title: &str, exit: &str) {
    let width = canvas.bounds().w;
    canvas.text(screen_rect(width, 40, 47, 910, 110), title, TITLE);
    canvas.button(back_button(width), exit, true, false);
}

pub fn back_button(width: i32) -> Rect {
    screen_rect(width, 1004, 40, 220, 100)
}

pub fn keyboard_key(layout: &Layout, editor: &Editor, done: &str, x: f32, y: f32) -> Option<Key> {
    layout
        .keyboard_keys_with(editor.layer, done)
        .into_iter()
        .find(|(_, r)| r.contains(x, y))
        .map(|(key, _)| key.key)
}

pub fn progress(canvas: &mut Canvas, bounds: Rect, filled: usize) {
    let gap = canvas.px(5).max(1);
    for index in 0..10 {
        let left = bounds.x + index * bounds.w / 10;
        let right = bounds.x + (index + 1) * bounds.w / 10;
        let ink = if (index as usize) < filled {
            BLACK
        } else {
            PAPER
        };
        canvas.panel(
            Rect::new(left, bounds.y, (right - left - gap).max(1), bounds.h),
            13,
            2,
            ink,
        );
    }
}

pub fn switch(canvas: &mut Canvas, bounds: Rect, enabled: bool) {
    canvas.panel(bounds, 32, 3, if enabled { BLACK } else { PAPER });
    let gap = canvas.px(8);
    let knob = bounds.h - gap * 2;
    let left = if enabled {
        bounds.right() - knob - gap
    } else {
        bounds.x + gap
    };
    canvas.circle(
        Rect::new(left, bounds.y + gap, knob, knob),
        if enabled { PAPER } else { BLACK },
    );
}
