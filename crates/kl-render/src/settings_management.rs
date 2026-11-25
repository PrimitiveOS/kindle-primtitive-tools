use super::{
    krect, switch_rect, Align, Layout, Rect, Renderer, Status, Text, Ui, BLACK, MARGIN_X, POPUP_RADIUS, WHITE,
};
use kl_core::{
    battery::{Sample, Source, HISTORY_SECS},
    layout::Page,
    wallpaper::WallpaperTarget,
    Usb,
};
use vello_cpu::kurbo::{self, Affine, Shape};

impl Renderer {
    pub(super) fn settings_scrollbar(&mut self, viewport: Rect, scroll: i32, max_scroll: i32) {
        let track_height = viewport.h - 24;
        if max_scroll <= 0 || track_height <= 0 {
            return;
        }
        let content_height = i64::from(viewport.h) + i64::from(max_scroll);
        let thumb_height =
            (i64::from(track_height) * i64::from(viewport.h) / content_height) as i32;
        let thumb_height = thumb_height.clamp(32.min(track_height), track_height);
        let travel =
            i64::from(track_height - thumb_height) * i64::from(scroll.clamp(0, max_scroll));
        let y = viewport.y + 12 + (travel / i64::from(max_scroll)) as i32;
        let ink = if self.high_contrast { BLACK } else { 125 };
        self.rounded(
            Rect::new(viewport.right() - 20, y, 6, thumb_height),
            3.0,
            ink,
        );
    }

    pub(super) fn settings_visibility(&mut self, ui: &Ui) {
        let layout = ui.layout;
        let scroll = ui.visibility_scroll;
        let count = ui.visibility_len();
        let first_row = layout.visibility_row(0, scroll);
        let viewport = layout.visibility_viewport();
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.push_clip_rect(&krect(viewport));
        if count == 0 {
            self.text(Text {
                text: "Apps that can be hidden will appear here.",
                x: (first_row.x + 24) as f32,
                y: (first_row.y + 25) as f32,
                w: (first_row.w - 48) as f32,
                size: 34.0,
                bold: false,
                align: Align::Start,
                lines: 2,
                gray: BLACK,
            });
            self.ctx.pop_clip();
            return;
        }
        let mut card = first_row;
        card.h = layout.visibility_row(count - 1, scroll).bottom() - card.y;
        self.card(card);
        for (i, app) in ui.visible_visibility_apps() {
            let r = layout.visibility_row(i, scroll);
            self.text(Text {
                text: &app.name,
                x: (r.x + 32) as f32,
                y: (r.y + 22) as f32,
                w: (r.w - 240) as f32,
                size: 36.0,
                bold: true,
                align: Align::Start,
                lines: 1,
                gray: BLACK,
            });
            self.text(Text {
                text: if app.hidden {
                    "Hidden"
                } else {
                    "On the home screen"
                },
                x: (r.x + 32) as f32,
                y: (r.y + 69) as f32,
                w: (r.w - 240) as f32,
                size: 26.0,
                bold: false,
                align: Align::Start,
                lines: 1,
                gray: 85,
            });
            self.switch(switch_rect(r), !app.hidden);
            if i + 1 < count {
                self.hairline(r.bottom() - 1, card.x + 32, card.right() - 32);
            }
        }
        self.ctx.pop_clip();
        self.settings_scrollbar(viewport, scroll, ui.visibility_scroll_max());
    }

    pub(super) fn settings_installed_apps(&mut self, ui: &Ui) {
        let layout = ui.layout;
        let scroll = ui.installed_scroll;
        let first_row = layout.visibility_row(0, scroll);
        let apps = match ui.installed_apps() {
            Some(apps) if !apps.is_empty() => apps,
            apps => {
                let text = if apps.is_some() { "No installed apps." } else { "Installed apps are unavailable." };
                self.text(Text { text, x: (first_row.x + 24) as f32, y: (first_row.y + 25) as f32, w: (first_row.w - 48) as f32, size: 34.0, bold: false, align: Align::Start, lines: 2, gray: BLACK });
                return;
            }
        };

        let viewport = layout.visibility_viewport();
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.push_clip_rect(&krect(viewport));
        let last_row = layout.visibility_row(apps.len() - 1, scroll);
        self.card(Rect { h: last_row.bottom() - first_row.y, ..first_row });
        let first_visible = ((viewport.y - first_row.y) / first_row.h).max(0) as usize;
        for (index, app) in apps.iter().enumerate().skip(first_visible) {
            let row = layout.visibility_row(index, scroll);
            if row.y >= viewport.bottom() {
                break;
            }
            let button = layout.installed_remove_button(row);
            let x = (row.x + 32) as f32;
            let w = (button.x - row.x - 56) as f32;
            self.text(Text { text: &app.name, x, y: (row.y + 22) as f32, w, size: 36.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
            self.text(Text { text: &app.version, x, y: (row.y + 69) as f32, w, size: 26.0, bold: false, align: Align::Start, lines: 1, gray: 85 });
            if !app.can_remove {
                self.text(Text { text: "Protected", x: button.x as f32, y: (row.y + 42) as f32, w: button.w as f32, size: 28.0, bold: false, align: Align::Center, lines: 1, gray: 85 });
            } else if !ui.installed_busy {
                self.button(button, "Uninstall", false);
            }
            if index + 1 < apps.len() {
                self.hairline(row.bottom() - 1, row.x + 32, row.right() - 32);
            }
        }
        self.ctx.pop_clip();
        self.settings_scrollbar(viewport, scroll, ui.installed_scroll_max());
    }

    pub(super) fn settings_uninstall_sheet(&mut self, ui: &Ui) {
        let name = ui.installed_confirmation().map(|app| app.name.as_str());
        if !ui.installed_busy && name.is_none() {
            return;
        }
        let sheet = ui.layout.confirm_box();
        let inner = sheet.inset(48);
        self.rounded_frame(sheet, POPUP_RADIUS as f64, 6, BLACK, WHITE);
        let title = if ui.installed_busy { "Uninstalling app…" } else { "Uninstall this app?" };
        self.text(Text { text: title, x: inner.x as f32, y: inner.y as f32, w: inner.w as f32, size: 50.0, bold: true, align: Align::Start, lines: 1, gray: BLACK });
        if !ui.installed_busy {
            if let Some(name) = name {
                self.text(Text { text: name, x: inner.x as f32, y: (inner.y + 100) as f32, w: inner.w as f32, size: 36.0, bold: false, align: Align::Start, lines: 3, gray: BLACK });
            }
            let [(_, cancel), (_, uninstall)] = ui.layout.confirm_buttons();
            self.button(cancel, "Cancel", false);
            self.button(uninstall, "Uninstall", true);
        }
    }

    pub(super) fn settings_wallpaper_picker(&mut self, ui: &Ui, target: WallpaperTarget) {
        let layout = ui.layout;
        let title = layout.title();
        let back = layout.done_button();
        self.text(Text {
            text: target.title(),
            x: MARGIN_X as f32,
            y: 36.0,
            w: (back.x - 2 * MARGIN_X) as f32,
            size: 56.0,
            bold: true,
            align: Align::Start,
            lines: 1,
            gray: BLACK,
        });
        self.button(back, "Back", true);
        self.fill(Rect::new(0, title.bottom() - 4, title.w, 4), BLACK);

        let viewport = layout.visibility_viewport();
        let scroll = ui.wallpaper_scroll;
        let choices: Vec<_> = ui.wallpapers.choices(target).collect();
        let max_scroll = layout.visibility_scroll_max(choices.len());
        let first = layout.visibility_row(0, scroll);
        let last = layout.visibility_row(choices.len() - 1, scroll);
        let card = Rect::new(first.x, first.y, first.w, last.bottom() - first.y);
        let active = ui.wallpapers.selection(target);
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.push_clip_rect(&krect(viewport));
        self.card(card);
        for (index, choice) in choices.iter().enumerate() {
            let row = layout.visibility_row(index, scroll);
            if row.bottom() <= viewport.y || row.y >= viewport.bottom() {
                continue;
            }
            self.text(Text {
                text: choice.label(target),
                x: (row.x + 32) as f32,
                y: (row.y + (row.h - 50) / 2) as f32,
                w: (row.w - 140) as f32,
                size: 36.0,
                bold: choice == active,
                align: Align::Start,
                lines: 1,
                gray: BLACK,
            });
            let radio = Rect::new(row.right() - 82, row.y + (row.h - 42) / 2, 42, 42);
            self.rounded_frame(radio, 21.0, 3, BLACK, WHITE);
            if choice == active {
                self.rounded(radio.inset(10), 11.0, BLACK);
            }
            if index + 1 < choices.len() {
                self.hairline(row.bottom() - 1, card.x + 32, card.right() - 32);
            }
        }
        self.ctx.pop_clip();
        self.settings_scrollbar(viewport, scroll, max_scroll);

        let (hint, bold, y) = if let Some(notice) = &ui.notice {
            (notice.as_str(), true, viewport.bottom() + 12)
        } else {
            let hint = if ui.wallpapers.images.is_empty() {
                "No PNG files found. Copy pictures to launcher/wallpapers/."
            } else {
                "Copy more PNG files to launcher/wallpapers/."
            };
            (hint, false, (card.bottom() + 24).min(viewport.bottom() + 12))
        };
        self.text(Text {
            text: hint,
            x: MARGIN_X as f32,
            y: y as f32,
            w: (layout.width - 2 * MARGIN_X) as f32,
            size: 28.0,
            bold,
            align: Align::Start,
            lines: 2,
            gray: if bold { BLACK } else { 85 },
        });
    }

    pub(super) fn level_meter(&mut self, row: Rect, level: u64, total: u64) {
        if total == 0 {
            return;
        }
        let width = (row.w / 4).min(220);
        let bar = Rect::new(
            row.right() - 44 - width,
            row.y + (row.h - 24) / 2,
            width,
            24,
        );
        self.rounded_frame(bar, 12.0, 2, BLACK, WHITE);
        let inner = bar.inset(4);
        let filled = (inner.w as f64 * level.min(total) as f64 / total as f64).round() as i32;
        if filled > 0 {
            self.rounded(
                Rect::new(inner.x, inner.y, filled, inner.h),
                f64::from(filled.min(inner.h)) / 2.0,
                BLACK,
            );
        }
    }

    /// The usage card below the 24-hour chart: the last seven days, one line per item (screen on, then the most used apps).
    pub(super) fn battery_usage(&mut self, layout: Layout, status: &Status) {
        let page_end = layout.page_end(Page::Battery);
        let chart_h = (layout.height - page_end - 156).min(390);
        let y = page_end + 52 + chart_h + 40;
        let lines = status.usage.len().max(1) as i32;
        let height = 120 + lines * 60;
        if y + height > layout.height - 40 {
            return;
        }
        self.section_header(MARGIN_X, y, "Usage, last 7 days");
        let card = Rect::new(MARGIN_X, y + 52, layout.width - 2 * MARGIN_X, height);
        self.card(card);
        let inset = card.inset(28);
        if status.usage.is_empty() {
            self.history_text(inset, "No use yet", 34.0, BLACK);
        }
        for (i, (label, value)) in status.usage.iter().enumerate() {
            let row = Rect::new(inset.x, inset.y + i as i32 * 60, inset.w, 44);
            self.history_text(row, &format!("{label} · {value}"), 30.0, BLACK);
        }
        self.history_text(
            Rect::new(inset.x, card.bottom() - 62, inset.w, 36),
            "Estimated from CPU time and time in front. Not a measurement of battery.",
            24.0,
            85,
        );
    }

    pub(super) fn battery_history(&mut self, layout: Layout, status: &Status) {
        let y = layout.page_end(Page::Battery);
        let card = Rect::new(
            MARGIN_X,
            y + 52,
            layout.width - 2 * MARGIN_X,
            (layout.height - y - 156).min(390),
        );
        if card.h < 200 {
            return;
        }
        self.section_header(card.x, y, "Battery level over the last 24 hours");
        self.card(card);
        let start = status.battery_now.saturating_sub(HISTORY_SECS);
        let entries: Vec<_> = status
            .battery_history
            .iter()
            .filter(|item| (start..=status.battery_now).contains(&item.observed_at))
            .collect();
        let inset = card.inset(28);
        if entries.len() < 2 {
            self.history_text(inset, "Collecting history", 34.0, BLACK);
            self.history_text(
                Rect::new(inset.x, inset.y + 56, inset.w, 36),
                "Readings are saved while Launcher is awake.",
                27.0,
                85,
            );
            return;
        }
        let plot = Rect::new(card.x + 96, card.y + 34, card.w - 134, card.h - 155);
        self.frame(plot, 2, 195);
        self.history_text(Rect::new(inset.x, plot.y - 14, 64, 35), "100%", 22.0, 85);
        self.history_text(
            Rect::new(inset.x, plot.bottom() - 16, 64, 35),
            "0%",
            22.0,
            85,
        );
        let point = |item: &Sample| {
            let x = f64::from(plot.x)
                + f64::from(plot.w) * (item.observed_at - start) as f64 / HISTORY_SECS as f64;
            let y = f64::from(plot.bottom())
                - f64::from(plot.h) * f64::from(item.percent.min(100)) / 100.0;
            kurbo::Point::new(x, y)
        };
        for (i, item) in entries.iter().enumerate() {
            let end = point(item);
            if i > 0 {
                let previous = entries[i - 1];
                let begin = point(previous);
                // A short gap (the Kindle asleep) is a dotted line; a long one has no reading and breaks the line.
                let short_gap = item.observed_at - previous.observed_at <= ASLEEP_GAP_SECS;
                if !item.gap_before || short_gap {
                    let stroke = if item.gap_before {
                        kurbo::Stroke::new(3.0).with_dashes(0.0, [6.0, 8.0])
                    } else {
                        kurbo::Stroke::new(4.0)
                    };
                    self.ctx.set_transform(Affine::IDENTITY);
                    self.ctx.set_stroke(stroke);
                    self.ctx.set_paint(super::gray(BLACK));
                    self.ctx
                        .stroke_path(&kurbo::Line::new(begin, end).to_path(0.1));
                }
                if !item.gap_before && previous.source == Source::Charging {
                    self.fill(
                        Rect::new(
                            begin.x as i32,
                            plot.bottom() + 9,
                            ((end.x - begin.x) as i32).max(3),
                            7,
                        ),
                        BLACK,
                    );
                }
            }
            self.fill(Rect::new(end.x as i32 - 3, end.y as i32 - 3, 6, 6), BLACK);
        }
        for (label, align) in [("24 hours ago", Align::Start), ("Now", Align::End)] {
            self.text(Text {
                text: label,
                x: plot.x as f32,
                y: (plot.bottom() + 19) as f32,
                w: plot.w as f32,
                size: 23.0,
                bold: false,
                align,
                lines: 1,
                gray: 85,
            });
        }
        let description = entries
            .iter()
            .rev()
            .find(|item| item.source == Source::Charging)
            .map_or_else(
                || "Gaps show periods without readings.".into(),
                |item| {
                    let minutes = (status.battery_now - item.observed_at) / 60;
                    match minutes {
                        0 => "Charging observed just now".into(),
                        1..=59 => format!("Charging observed {minutes} min ago"),
                        _ => format!("Charging observed {} hr ago", minutes / 60),
                    }
                },
            );
        self.history_text(
            Rect::new(inset.x, card.bottom() - 57, inset.w, 35),
            &description,
            25.0,
            85,
        );
    }

    fn history_text(&mut self, rect: Rect, text: &str, size: f32, gray: u8) {
        self.text(Text {
            text,
            x: rect.x as f32,
            y: rect.y as f32,
            w: rect.w as f32,
            size,
            bold: false,
            align: Align::Start,
            lines: 1,
            gray,
        });
    }
}

/// A gap up to this long (the Kindle asleep, between two readings) is drawn dotted on the chart.
const ASLEEP_GAP_SECS: u64 = 2 * 60 * 60;

pub(super) fn battery_status(status: &Status) -> String {
    let level = status
        .power
        .percent
        .map_or_else(|| "Level unavailable".into(), |value| format!("{value}%"));
    format!("{level} · {}", status.power.source.label())
}

pub(super) fn storage_status(ui: &Ui, status: &Status) -> String {
    if ui.usb == Usb::Drive {
        return "Kindle drive is connected to a computer".into();
    }
    let Some((used, total)) = status.system.storage.filter(|(_, total)| *total != 0) else {
        return "Storage unavailable".into();
    };
    let gb = |bytes: u64| bytes as f64 / 1_000_000_000.0;
    format!(
        "{:.1} of {:.1} GB used · {:.1} GB free",
        gb(used),
        gb(total),
        gb(total.saturating_sub(used))
    )
}

pub(super) fn usb_status(usb: Usb) -> &'static str {
    match usb {
        Usb::Unplugged => "Unplugged",
        Usb::Charging => "Cable connected",
        Usb::Drive => "Kindle drive connected",
    }
}
