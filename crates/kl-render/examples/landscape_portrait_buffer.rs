//! Off-device proof. Run with an output directory, Noto Sans regular and bold.
use kl_core::{Action, DeviceCmd, Input, Layout, Rect, Screen, Status, Ui};
use kl_render::component::{Canvas, TextStyle};
use std::{fs, path::Path};

const WIDTH: usize = 1264;
const HEIGHT: usize = 1680;
const PITCH: usize = 1280;
const MAP_BYTES: usize = 4_587_520;
const PAD: u8 = 0xa5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    Upright,
    Clockwise,
    UpsideDown,
    Counterclockwise,
}

impl Turn {
    fn size(self, w: usize, h: usize) -> (usize, usize) {
        match self {
            Self::Upright | Self::UpsideDown => (w, h),
            Self::Clockwise | Self::Counterclockwise => (h, w),
        }
    }

    fn native_to_view(self, (x, y): (usize, usize), w: usize, h: usize) -> (usize, usize) {
        match self {
            Self::Upright => (x, y),
            Self::Clockwise => (h - 1 - y, x),
            Self::UpsideDown => (w - 1 - x, h - 1 - y),
            Self::Counterclockwise => (y, w - 1 - x),
        }
    }

    fn to_native(self, (x, y): (usize, usize), w: usize, h: usize) -> (usize, usize) {
        match self {
            Self::Upright => (x, y),
            Self::Clockwise => (y, h - 1 - x),
            Self::UpsideDown => (w - 1 - x, h - 1 - y),
            Self::Counterclockwise => (w - 1 - y, x),
        }
    }

    fn page_key(self, code: u16) -> Option<Input> {
        let exchanged = matches!(self, Self::Clockwise | Self::UpsideDown);
        match (code, exchanged) {
            (104, false) | (109, true) => Some(Input::PageBack),
            (109, false) | (104, true) => Some(Input::PageForward),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Firmware {
    Upright,
    UpsideDown,
}

impl Firmware {
    /// A half turn is its own inverse.
    fn to_memory(self, (x, y): (usize, usize), w: usize, h: usize) -> (usize, usize) {
        match self {
            Self::Upright => (x, y),
            Self::UpsideDown => (w - 1 - x, h - 1 - y),
        }
    }
}

#[derive(Clone, Copy)]
struct Transform {
    turn: Turn,
    firmware: Firmware,
    w: usize,
    h: usize,
}

impl Transform {
    fn logical_size(self) -> (usize, usize) {
        self.turn.size(self.w, self.h)
    }

    fn logical_index(self, memory_index: usize) -> usize {
        let point = (memory_index % self.w, memory_index / self.w);
        let native = self.firmware.to_memory(point, self.w, self.h);
        let (x, y) = self.turn.native_to_view(native, self.w, self.h);
        let (width, _) = self.logical_size();
        y * width + x
    }

    fn to_panel(self, logical: &[u8]) -> Vec<u8> {
        assert_eq!(logical.len(), self.w * self.h);
        (0..logical.len())
            .map(|index| logical[self.logical_index(index)])
            .collect()
    }

    fn to_view(self, panel: &[u8]) -> Vec<u8> {
        assert_eq!(panel.len(), self.w * self.h);
        let mut logical = vec![0; panel.len()];
        for (index, &pixel) in panel.iter().enumerate() {
            logical[self.logical_index(index)] = pixel;
        }
        logical
    }

    fn screen_point(self, point: (usize, usize)) -> (usize, usize) {
        self.turn.native_to_view(point, self.w, self.h)
    }

    fn screen_swipe(self, start: (usize, usize), end: (usize, usize)) -> Input {
        let (x, y) = self.screen_point(start);
        let (end_x, end_y) = self.screen_point(end);
        Input::Swipe {
            dx: end_x as f32 - x as f32,
            dy: end_y as f32 - y as f32,
            start_y: y as f32,
        }
    }
}

fn write_png(path: &Path, (w, h): (usize, usize), frame: &[u8]) {
    let mut png = png::Encoder::new(fs::File::create(path).unwrap(), w as u32, h as u32);
    png.set_color(png::ColorType::Grayscale);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(frame).unwrap();
}

fn field_notes((w, h): (usize, usize), fonts: &[String]) -> Vec<u8> {
    let (w, h) = (w as i32, h as i32);
    let mut canvas = Canvas::new(w as u32, h as u32, fonts).unwrap();
    canvas.begin();
    canvas.text(
        Rect::new(48, 30, w - 96, 88),
        "Harbour watch",
        TextStyle::new(59).bold(),
    );
    canvas.fill(Rect::new(47, 138, w - 94, 2), 0);
    let column = (w - 170) / 2;
    let columns = [
        (48, "North wall", [
            "At dawn the harbour looked shallower than it was. A line of silver held between the last two posts.",
            "Mina marked the tide on the stone beside the ladder. The first mark was a handspan above yesterday’s.",
            "A boat turned beyond the breakwater. Its engine stopped before the sound reached the steps.",
        ]),
        (w / 2 + 36, "Outer channel", [
            "By noon the wind had pushed the loose weed against the outer wall. The navigation light stayed clear.",
            "From the steps we counted three crossings. A fourth boat waited where the channel bends.",
            "We left the notebook under a flat stone and followed the road inland as the water rose.",
        ]),
    ];
    for (x, heading, paragraphs) in columns {
        canvas.text(
            Rect::new(x, 172, column, 72),
            heading,
            TextStyle::new(42).bold(),
        );
        for (row, sentence) in paragraphs.into_iter().enumerate() {
            canvas.text(
                Rect::new(x, 270 + 270 * row as i32, column, 267),
                sentence,
                TextStyle::new(39).lines(5),
            );
        }
    }
    canvas.fill(Rect::new(48, h - 94, w - 96, 2), 0);
    canvas.text(
        Rect::new(48, h - 80, w - 96, 55),
        "Chapter 2 of 4 - Page 6 of 19",
        TextStyle::new(32),
    );
    canvas.finish(false)
}

fn apply_native_region(
    memory: &mut [u8],
    y_offset: usize,
    frame: &[u8],
    (x, y, w, h): (usize, usize, usize, usize),
) {
    assert!(x + w <= WIDTH && y + h <= HEIGHT);
    assert_eq!(frame.len(), WIDTH * HEIGHT);
    for row in y..y + h {
        let dest = (y_offset + row) * PITCH + x;
        let src = row * WIDTH + x;
        memory[dest..dest + w].copy_from_slice(&frame[src..src + w]);
    }
}

fn visible_frame(memory: &[u8], y_offset: usize) -> Vec<u8> {
    let start = y_offset * PITCH;
    memory[start..start + HEIGHT * PITCH]
        .chunks_exact(PITCH)
        .flat_map(|row| row[..WIDTH].iter().copied())
        .collect()
}

#[derive(Clone, Copy)]
struct Snapshot {
    generation: u64,
    transform: Transform,
}

impl Snapshot {
    fn backdrop(self, generation: u64, panel: &[u8]) -> Option<Vec<u8>> {
        (self.generation == generation && panel.len() == self.transform.w * self.transform.h)
            .then(|| self.transform.to_view(panel))
    }
}

fn exercise(transform: Transform, page: &[u8], fonts: &[String], out: &Path, label: &str) {
    let size = transform.logical_size();
    let panel = transform.to_panel(page);
    let mut memory = vec![PAD; MAP_BYTES];
    let y_offset = 17;
    apply_native_region(&mut memory, y_offset, &panel, (0, 0, WIDTH, HEIGHT));
    assert_eq!(visible_frame(&memory, y_offset), panel);
    let saved = Snapshot {
        generation: 17,
        transform,
    };
    let backdrop = saved
        .backdrop(17, &visible_frame(&memory, y_offset))
        .unwrap();
    assert_eq!(backdrop, page);
    assert!(saved.backdrop(18, &panel).is_none());
    assert!(saved.backdrop(17, &panel[..15]).is_none());

    let top = transform.turn.to_native((size.0 / 2, 24), WIDTH, HEIGHT);
    let inside = transform.turn.to_native((size.0 / 2, 302), WIDTH, HEIGHT);
    let layout = Layout::new(size.0 as u32, size.1 as u32, 4, 5);
    let mut ui = Ui::new(Vec::new(), layout);
    let bottom = transform
        .turn
        .to_native((size.0 / 2, size.1 - 24), WIDTH, HEIGHT);
    let upward = transform
        .turn
        .to_native((size.0 / 2, size.1 - 302), WIDTH, HEIGHT);
    ui.handle(transform.screen_swipe(bottom, upward));
    assert_eq!(ui.screen, Screen::Home);
    ui.handle(transform.screen_swipe(top, inside));
    assert_eq!(ui.screen, Screen::QuickSettings);
    assert!(layout.quick_panel().bottom() <= size.1 as i32);
    let mut renderer = kl_render::Renderer::new(size.0 as u32, size.1 as u32, fonts).unwrap();
    renderer.set_backdrop(Some(backdrop));
    let status = Status {
        clock: "9:41".into(),
        date: "Tuesday, November 24".into(),
        battery: Some(72),
        light: 9,
        warmth: 6,
        wifi_on: true,
        wifi_state: "connected".into(),
        wifi_signal: Some((4, 5)),
        auto_rotation: true,
        ..Status::default()
    };
    let overlay = renderer.draw(&ui, &status);
    let first_uncovered_row = layout.quick_panel().bottom() as usize * size.0;
    assert_eq!(
        &overlay[first_uncovered_row..],
        &page[first_uncovered_row..]
    );
    let overlay_panel = transform.to_panel(&overlay);
    let changed = kl_render::changed_rect(&panel, &overlay_panel, WIDTH).unwrap();
    apply_native_region(&mut memory, y_offset, &overlay_panel, changed);
    assert_eq!(visible_frame(&memory, y_offset), overlay_panel);

    let plus = layout.quick_plus(0);
    let point = (
        (plus.x + plus.w / 2) as usize,
        (plus.y + plus.h / 2) as usize,
    );
    let (x, y) = transform.screen_point(transform.turn.to_native(point, WIDTH, HEIGHT));
    assert_eq!(
        ui.handle(Input::Tap(x as f32, y as f32)),
        Action::Device(DeviceCmd::Light(1))
    );
    let handle = layout.quick_handle();
    let close = transform.turn.to_native(
        (size.0 / 2, (handle.y + handle.h / 2) as usize),
        WIDTH,
        HEIGHT,
    );
    let upward = transform
        .turn
        .to_native((size.0 / 2, handle.y as usize - 220), WIDTH, HEIGHT);
    ui.handle(transform.screen_swipe(close, upward));
    assert_eq!(ui.screen, Screen::Home);
    apply_native_region(&mut memory, y_offset, &panel, changed);
    assert_eq!(visible_frame(&memory, y_offset), panel);
    assert!(memory[..y_offset * PITCH].iter().all(|&byte| byte == PAD));
    assert!(memory[(y_offset + HEIGHT) * PITCH..]
        .iter()
        .all(|&byte| byte == PAD));
    assert!(memory[y_offset * PITCH..(y_offset + HEIGHT) * PITCH]
        .chunks_exact(PITCH)
        .all(|row| row[WIDTH..].iter().all(|&byte| byte == PAD)));

    write_png(
        &out.join(format!("{label}-panel.png")),
        (WIDTH, HEIGHT),
        &panel,
    );
    write_png(
        &out.join(format!("{label}-overlay-panel.png")),
        (WIDTH, HEIGHT),
        &overlay_panel,
    );
    if transform.firmware == Firmware::Upright {
        write_png(&out.join(format!("{label}-overlay.png")), size, &overlay);
    }
    println!(
        "{label} logical {}×{}, panel {}×{} pitch {PITCH}, update rect {changed:?}",
        size.0, size.1, WIDTH, HEIGHT
    );
}

fn verify_page_keys() {
    for (turn, back, forward) in [
        (Turn::Upright, 104, 109),
        (Turn::Clockwise, 109, 104),
        (Turn::UpsideDown, 109, 104),
        (Turn::Counterclockwise, 104, 109),
    ] {
        assert_eq!(turn.page_key(back), Some(Input::PageBack));
        assert_eq!(turn.page_key(forward), Some(Input::PageForward));
        assert_eq!(turn.page_key(102), None);
    }
}

fn main() {
    verify_page_keys();
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(
        args.len(),
        3,
        "Usage: landscape_portrait_buffer OUTPUT NotoSans-Regular.ttf NotoSans-Bold.ttf"
    );
    let out = Path::new(&args[0]);
    let fonts = &args[1..];
    fs::create_dir_all(out).unwrap();
    let size = Turn::Clockwise.size(WIDTH, HEIGHT);
    let page = field_notes(size, fonts);
    write_png(&out.join("landscape-reading.png"), size, &page);
    for (name, turn) in [("cw", Turn::Clockwise), ("ccw", Turn::Counterclockwise)] {
        for (number, firmware) in [(0, Firmware::Upright), (2, Firmware::UpsideDown)] {
            let transform = Transform {
                turn,
                firmware,
                w: WIDTH,
                h: HEIGHT,
            };
            exercise(transform, &page, fonts, out, &format!("{name}-fb{number}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_stock_matrix_and_round_trips_four_turns() {
        let input: Vec<_> = (0..15).collect();
        let cases = [
            (Turn::Upright, input.clone()),
            (
                Turn::Clockwise,
                vec![4, 9, 14, 3, 8, 13, 2, 7, 12, 1, 6, 11, 0, 5, 10],
            ),
            (Turn::UpsideDown, (0..15).rev().collect()),
            (
                Turn::Counterclockwise,
                vec![10, 5, 0, 11, 6, 1, 12, 7, 2, 13, 8, 3, 14, 9, 4],
            ),
        ];
        for (turn, expected) in cases {
            let transform = Transform {
                turn,
                firmware: Firmware::Upright,
                w: 3,
                h: 5,
            };
            assert_eq!(transform.to_panel(&input), expected);
            assert_eq!(transform.to_view(&expected), input);
            let upside_down = Transform {
                firmware: Firmware::UpsideDown,
                ..transform
            };
            let reversed: Vec<_> = expected.into_iter().rev().collect();
            assert_eq!(upside_down.to_panel(&input), reversed);
            assert_eq!(upside_down.to_view(&reversed), input);
            let (w, h) = transform.logical_size();
            for x in 0..w {
                for y in 0..h {
                    let native = turn.to_native((x, y), 3, 5);
                    assert_eq!(transform.screen_point(native), (x, y));
                    assert_eq!(upside_down.screen_point(native), (x, y));
                }
            }
        }
    }

    #[test]
    fn preserves_portrait_keys_and_maps_landscape_keys() {
        verify_page_keys();
    }
}
