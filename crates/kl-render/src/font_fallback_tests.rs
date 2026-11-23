use super::*;
use component::{Canvas, TextStyle};

fn regular_fonts() -> Option<Vec<String>> {
    let directory = std::env::var_os("KL_TEST_FONTS")?;
    let directory = std::path::Path::new(&directory);
    let fonts = ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf"].map(|f| directory.join(f));
    fonts.iter().all(|p| p.is_file()).then(|| {
        fonts
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    })
}
fn missing(renderer: &mut Renderer, text: &str, bold: bool) -> usize {
    renderer
        .layout_text(text, 36., bold, 1000.)
        .lines()
        .flat_map(|line| {
            line.items()
                .filter_map(|item| {
                    if let PositionedLayoutItem::GlyphRun(run) = item {
                        Some(run.positioned_glyphs().filter(|g| g.id == 0).count())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .sum()
}
fn popup(fonts: &[String], text: &str) -> Vec<u8> {
    let mut canvas = Canvas::new(800, 600, fonts).unwrap();
    canvas.begin();
    canvas.fill(Rect::new(0, 0, 800, 600), 255);
    canvas.popup(Rect::new(40, 100, 720, 400));
    canvas.text(Rect::new(80, 145, 640, 90), text, TextStyle::new(36).bold());
    canvas.text(
        Rect::new(80, 240, 640, 180),
        "A dictionary definition over the book.",
        TextStyle::new(30),
    );
    canvas.finish(false)
}

#[test]
fn supplied_cjk_fallback_paints_real_popup_headwords_and_keeps_ascii_pixels() {
    let Some(fonts) = regular_fonts() else {
        return;
    };
    let Some(cjk) = std::env::var_os("KL_TEST_CJK_FONT") else {
        return;
    };
    let mut supplied = fonts.clone();
    supplied.push(cjk.to_string_lossy().into_owned());
    let text = "読み込む · 中文辞典";
    let mut before = Renderer::new(800, 600, &fonts).unwrap();
    let mut after = Renderer::new(800, 600, &supplied).unwrap();
    for bold in [false, true] {
        assert!(
            missing(&mut before, text, bold) > 0,
            "control requires Latin-only Noto Sans"
        );
        assert_eq!(
            missing(&mut after, text, bold),
            0,
            "provided CJK glyphs must reach shared UI text"
        );
    }
    assert_eq!(
        popup(&fonts, "Reading notes · Dictionary"),
        popup(&supplied, "Reading notes · Dictionary")
    );
    let baseline = popup(&fonts, text);
    let rendered = popup(&supplied, text);
    assert_ne!(rendered, baseline);
    assert!(rendered.contains(&0));
    assert!(rendered.contains(&255));
    // Duplicate supplied paths cannot alter fallback ordering or pixels.
    supplied.extend(fonts);
    assert_eq!(rendered, popup(&supplied, text));
}
