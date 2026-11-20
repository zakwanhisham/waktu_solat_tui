use chrono::{DateTime, Datelike, Local};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::{
    api::PrayerDay,
    app::{App, Mode, PickerRow},
};

const ACCENT: Color = Color::Cyan;

pub fn draw(f: &mut Frame, app: &mut App) {
    let [header, next, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(4),
        Constraint::Min(10),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_header(f, app, header);
    draw_next_prayer(f, app, next);
    draw_table(f, app, body);
    draw_footer(f, app, footer);

    if app.mode == Mode::Help {
        draw_help(f, app);
    } else if app.mode == Mode::Picker {
        draw_picker(f, app);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let now = Local::now();
    let date = now.format("%a, %d %b %Y");
    let mut line = vec![Span::styled("Waktu Solat ", Style::new().fg(ACCENT).bold())];
    match &app.zone {
        Some(z) => line.push(Span::raw(format!("· {} ", z.display()))),
        None => line.push(Span::raw("· loading zones… ")),
    }
    let spans = Line::from(line).right_aligned();
    // Active prayer window, highlighted; night renders dim (it isn't a
    // prayer). The Next panel below carries the details.
    let active = match app.current_prayer() {
        Some((label, _)) => Span::styled(
            format!("Now: {label}  ·  "),
            Style::new().fg(ACCENT).bold(),
        ),
        None => Span::styled("Now: Night  ·  ", Style::new().dim()),
    };
    f.render_widget(
        Paragraph::new(vec![
            spans,
            Line::from(vec![active, Span::styled(format!("{date}"), Style::new().dim())]),
        ]),
        area,
    );
}

fn draw_next_prayer(f: &mut Frame, app: &App, area: Rect) {
    // Permanent notification-state indicator, right-aligned in the title row
    // (the footer hint line has no room for it).
    let notify_state = if app.notify_enabled {
        Line::styled(" notify: on ", Style::new().fg(ACCENT)).right_aligned()
    } else {
        Line::styled(" notify: off ", Style::new().dim()).right_aligned()
    };
    let block = Block::bordered()
        .title(Line::styled(" Next ", Style::new().fg(ACCENT).bold()))
        .title(notify_state);

    let code = app.zone.as_ref().map(|z| z.jakim_code.clone());
    let now = Local::now();
    let today = now.date_naive();
    let todays = code
        .and_then(|c| app.months.get(&(c, now.year(), now.month())).cloned())
        .and_then(|(_, d)| d.prayers.iter().find(|p| p.day == today.day()).cloned());

    let content = match app.next_prayer() {
        Some((label, secs, tomorrow)) => {
            let target = chrono::DateTime::from_timestamp(secs, 0)
                .map(|t| t.with_timezone(&Local))
                .unwrap();
            let diff = (secs - now.timestamp()).max(0);
            let countdown = if diff == 0 { "now".to_string() } else { format_secs(diff as u64) };
            let line1 = Line::from(vec![
                Span::raw("Now: "),
                match app.current_prayer() {
                    Some((current, _)) => Span::styled(current, Style::new().bold().fg(ACCENT)),
                    None => Span::raw("Night"),
                },
                Span::raw("  ·  Next: "),
                Span::styled(label, Style::new().bold().fg(ACCENT)),
                if tomorrow { Span::raw(" (tomorrow)") } else { Span::raw("") },
                Span::raw(format!(" at {}, in ", target.format("%H:%M"))),
                Span::styled(countdown, Style::new().bold()),
            ]);
            // The night gap belongs to Isyak's window — `current_prayer()`
            // returns Isyak then, so the list stays highlighted.
            let line2 = times_line(todays.as_ref(), app.current_prayer().map(|(l, _)| l));
            vec![line1, line2]
        }
        None => vec![Line::from(match (app.loading, &app.error) {
            (true, _) => Span::raw("Loading prayer times…"),
            (_, Some(e)) => Span::styled(e.clone(), Style::new().fg(Color::Red)),
            _ => Span::raw("No data yet — press `r` to retry."),
        })],
    };
    f.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }).block(block), area);
}

/// The `Imsak … · Isyak …` line in the Next panel. The prayer window we are
/// currently in is highlighted (bold accent); the rest stay dim.
fn times_line(todays: Option<&PrayerDay>, active: Option<&str>) -> Line<'static> {
    let Some(t) = todays else {
        return Line::from(Span::styled("—", Style::new().dim()));
    };
    let mut spans = Vec::new();
    for (i, (label, secs)) in PrayerDay::KINDS.iter().zip(t.times()).enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::new().dim()));
        }
        let text = format!("{label} {}", fmt_epoch(secs));
        spans.push(if Some(*label) == active {
            Span::styled(text, Style::new().bold().fg(ACCENT))
        } else {
            Span::styled(text, Style::new().dim())
        });
    }
    Line::from(spans)
}

fn draw_table(f: &mut Frame, app: &mut App, area: Rect) {
    // Rows visible inside the bordered block: height - 2 borders - 1 header
    // row. The app uses this to size half-page scrolls (Ctrl-d / Ctrl-u).
    app.viewport_rows = area.height.saturating_sub(3).max(1);
    let today = Local::now().date_naive();

    let title = match (&app.zone, app.view_data()) {
        (Some(z), Some((fetched, data))) => {
            let age = cache_age(fetched);
            let updated = data
                .last_updated
                .and_then(|ts| DateTime::from_timestamp(ts, 0))
                .map(|t| format!(" · JAKIM updated {}", t.with_timezone(&Local).format("%d %b %Y %H:%M")))
                .unwrap_or_default();
            // Hijri month of the viewed month. Hijri months straddle
            // Gregorian ones, so anchor on today's row when viewing the
            // current month, else the first row. Romanized names only — the
            // title line is LTR context (bidi-safe).
            let anchor = if data.year == today.year() && data.month_number == today.month() {
                data.prayers.iter().find(|p| p.day == today.day())
            } else {
                data.prayers.first()
            };
            let hijri_month = anchor
                .and_then(|p| crate::hijri::parse(&p.hijri))
                .map(|(y, m, _)| format!(" · {} {y} ", crate::hijri::MONTHS_MALAY[m as usize - 1]))
                .unwrap_or_default();
            format!(
                " {} {:04} · {} {} · {} ({}){}{}",
                month_name(app.view_month), app.view_year, z.jakim_code, z.negeri, z.daerah, age, updated, hijri_month
            )
        }
        (Some(_), None) => {
            format!(" {} {:04} — no data ", month_name(app.view_month), app.view_year)
        }
        _ => format!(" {} {:04} ", month_name(app.view_month), app.view_year),
    };

    // Adaptive Hijri column: full month names when the terminal is wide
    // enough, the numeric short form when tight, and the column dropped
    // entirely on very narrow terminals (prayer times matter most).
    // avail = table width minus borders (2) and the selection symbol (2).
    let avail = area.width.saturating_sub(4);
    let (hijri_width, hijri_style_eff, spacing) = if avail >= 89 {
        (Some(15_u16), app.hijri_style, 1_u16)
    } else if avail >= 69 {
        // Compact the inter-column spacing below 78 to buy the numeric
        // column room.
        (Some(7_u16), crate::hijri::HijriStyle::Numeric, if avail >= 78 { 1 } else { 0 })
    } else {
        (None, crate::hijri::HijriStyle::Numeric, 1)
    };

    let mut widths: Vec<Constraint> = vec![Constraint::Length(5)];
    let mut header_cells: Vec<&str> = vec!["Day"];
    if let Some(w) = hijri_width {
        widths.push(Constraint::Length(w));
        header_cells.push("Hijri");
    }
    // Imsak, Subuh, Syuruk, Dhuha, Zohor, Asar, Maghrib, Isyak
    widths.extend([
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(7),
    ]);
    header_cells.extend(["Imsak", "Subuh", "Syuruk", "Dhuha", "Zohor", "Asar", "Maghrib", "Isyak"]);

    // Which prayer column (0-7 within the prayer cells) is active right now —
    // styled per-cell below so empty rows never get a highlight stripe.
    let active_prayer = app
        .current_prayer()
        .and_then(|(label, _)| PrayerDay::KINDS.iter().position(|k| *k == label));

    let header = Row::new(header_cells)
    .style(Style::new().bold().fg(ACCENT))
    .bottom_margin(0);

    let rows: Vec<Row> = app
        .view_data()
        .map(|(_, data)| {
            data.prayers
                .iter()
                .map(|p| {
                    let is_today =
                        data.year == today.year() && data.month_number == today.month() && p.day == today.day();
                    // Today pops: bright gold text on a dark block, clearly
                    // distinct from the blue selection highlight. Fixed 256
                    // palette entries so themes can't make it unreadable.
                    let style = if is_today {
                        Style::new().bold().fg(Color::Indexed(220)).bg(Color::Indexed(238))
                    } else {
                        Style::new()
                    };
                    Row::new(prayer_cells(
                        p,
                        hijri_style_eff,
                        hijri_width.is_some(),
                        active_prayer,
                        is_today,
                    ))
                    .style(style)
                })
                .collect()
        })
        .unwrap_or_default();

    let content_w = widths
        .iter()
        .filter_map(|c| match c {
            Constraint::Length(l) => Some(*l),
            _ => None,
        })
        .sum::<u16>()
        + spacing * (widths.len().saturating_sub(1) as u16)
        + 2; // "› " selection symbol

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(title, Style::new().bold()));
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(spacing)
        // White on strong blue: readable in any terminal theme (fixed
        // 256-palette entries, not theme-remappable 16-color values).
        .row_highlight_style(
            Style::new().bold().fg(Color::Indexed(231)).bg(Color::Indexed(25)),
        )
        .highlight_symbol("› ");

    // The block spans the full terminal; the table is clipped to its content
    // width inside it, so row/cell highlights stop at the last column instead
    // of spilling across the empty area to the right.
    f.render_widget(block.clone(), area);
    let inner = block.inner(area);
    let table_area = Rect { width: inner.width.min(content_w), ..inner };
    f.render_stateful_widget(table, table_area, &mut app.table_state);
}
fn prayer_cells(
    p: &PrayerDay,
    hijri_style: crate::hijri::HijriStyle,
    include_hijri: bool,
    active_prayer: Option<usize>,
    is_today: bool,
) -> Vec<Cell<'static>> {
    let mut cells = vec![Cell::from(format!("{:2}", p.day))];
    if include_hijri {
        cells.push(Cell::from(crate::hijri::format_cell(&p.hijri, hijri_style)));
    }
    for (i, secs) in p.times().into_iter().enumerate() {
        let mut cell = Cell::from(fmt_epoch(secs));
        // Subtle fixed-palette bg on the active prayer's cell — per-cell, so
        // empty rows below the data never get a highlight stripe. Skipped on
        // today's row, which already carries its own full-row highlight.
        if active_prayer == Some(i) && !is_today {
            cell = cell.style(Style::new().bg(Color::Indexed(236)));
        }
        cells.push(cell);
    }
    cells
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints = vec![
        Span::styled("q", Style::new().bold()),
        Span::raw(" quit  "),
        Span::styled("s", Style::new().bold()),
        Span::raw(" zone  "),
        Span::styled("?", Style::new().bold()),
        Span::raw(" help  "),
        Span::styled("j/k", Style::new().bold()),
        Span::raw(" move  "),
        Span::styled("g/G", Style::new().bold()),
        Span::raw(" ends  "),
        Span::styled("^d/^u", Style::new().bold()),
        Span::raw(" page  "),
        Span::styled("h/l", Style::new().bold()),
        Span::raw(" month  "),
        Span::styled("t", Style::new().bold()),
        Span::raw(" today  "),
        Span::styled("r", Style::new().bold()),
        Span::raw(" refresh"),
    ];
    let mut line = Line::from(hints);
    if app.detecting {
        line = Line::from(Span::styled("Detecting location…", Style::new().fg(ACCENT)));
    } else if let Some(err) = &app.error {
        line = Line::from(Span::styled(err.clone(), Style::new().fg(Color::Red)));
    } else if let Some((status, _)) = &app.status {
        line = Line::from(Span::styled(status.clone(), Style::new().fg(ACCENT)));
    }
    f.render_widget(Paragraph::new(line), area);
}

fn draw_picker(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 70, 80);
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            " Select zone ",
            Style::new().bold().fg(ACCENT),
        ))
        .title_bottom(Line::from(format!(
            " {} zones ",
            app.picker_rows.iter().filter(|r| matches!(r, PickerRow::Zone(_))).count()
        ))
        .right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [input, list_area] = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(inner);

    f.render_widget(
        Paragraph::new(vec![Line::from(vec![
            Span::raw("Search: "),
            Span::styled(app.picker_query.clone(), Style::new().fg(ACCENT)),
            Span::styled("▌", Style::new().add_modifier(Modifier::SLOW_BLINK)),
        ])]),
        input,
    );

    let items: Vec<ListItem> = app
        .picker_rows
        .iter()
        .filter_map(|row| match row {
            PickerRow::Auto => Some(ListItem::new(Line::from(vec![
                Span::styled("📍 ", Style::new()),
                Span::styled("Use my location", Style::new().bold().fg(ACCENT)),
                Span::styled("  (IP-based, opt-in)", Style::new().dim()),
            ]))),
            PickerRow::Zone(i) => {
                let z = app.zones.get(*i)?;
                Some(ListItem::new(Line::from(vec![
                    Span::styled(format!("{: <7}", z.jakim_code), Style::new().bold()),
                    Span::styled(format!("{: <16}", z.negeri), Style::new().fg(ACCENT)),
                    Span::raw(truncate(&z.daerah, inner.width.saturating_sub(30) as usize)),
                ])))
            }
        })
        .collect();

    let mut list_state = ListState::default().with_selected(Some(app.picker_sel));
    let list = List::new(items)
        .highlight_style(Style::new().bold().fg(Color::Indexed(231)).bg(Color::Indexed(25)))
        .highlight_symbol("› ");
    f.render_stateful_widget(list, list_area, &mut list_state);
}

// ---------- helpers ----------

/// Centered popup area covering `vp`% of the height and `hp`% of the width.
fn centered(area: Rect, vp: u16, hp: u16) -> Rect {
    let [_, v] = Layout::vertical([
        Constraint::Percentage((100 - vp) / 2),
        Constraint::Percentage(vp),
    ])
    .areas(area);
    let [_, h] = Layout::horizontal([
        Constraint::Percentage((100 - hp) / 2),
        Constraint::Percentage(hp),
    ])
    .areas(v);
    h
}

fn draw_help(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 70, 60);
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(" Keybindings ", Style::new().bold().fg(ACCENT)))
        .title_bottom(Line::from(" Esc / q to close ").right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let key = |k: &str, desc: &str| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("{k: <10}"), Style::new().bold().fg(ACCENT)),
            Span::raw(desc.to_string()),
        ])
    };
    let text = vec![
        key("q / Esc", "Quit the app"),
        key("s / a", "Zone picker (`a` opens on the auto-detect row)"),
        key("?", "Toggle this help overlay"),
        key("j / k", "Move selection down / up (counts work: `5j` = 5 rows)"),
        key("g / G", "Jump to the first / last day of the month"),
        key("Ctrl-d", "Scroll half a page down"),
        key("Ctrl-u", "Scroll half a page up"),
        key("h / l", "Previous / next month (arrows work too)"),
        key("t", "Jump back to today"),
        key("c", &format!(
            "Cycle Hijri style ({})",
            match app.hijri_style {
                crate::hijri::HijriStyle::Malay => "malay",
                crate::hijri::HijriStyle::Numeric => "numeric",
                crate::hijri::HijriStyle::Arabic => "arabic",
            }
        )),
        key("n", &format!(
            "Toggle desktop reminders ({}, lead {}m)",
            if app.notify_enabled { "on" } else { "off" },
            app.notify_lead
        )),
        key("r", "Force refresh, bypassing the cache"),
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "Data: api.waktusolat.app (JAKIM e-solat) · zone {} · cached for 24 h",
                app.zone.as_ref().map(|z| z.jakim_code.as_str()).unwrap_or("—")
            ),
            Style::new().dim(),
        )),
    ];
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
}

fn fmt_epoch(secs: i64) -> String {
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(t) => t.with_timezone(&Local).format("%H:%M").to_string(),
        None => "—".to_string(),
    }
}

fn format_secs(total: u64) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

fn cache_age(fetched_at: u64) -> String {
    let age = crate::cache::now_secs().saturating_sub(fetched_at);
    if age < 60 {
        format!("cached {age}s ago")
    } else if age < 3600 {
        format!("cached {}m ago", age / 60)
    } else if age < 86_400 {
        format!("cached {}h ago", age / 3600)
    } else {
        format!("cached {}d ago", age / 86_400)
    }
}

fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "???",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_line_highlights_only_active_prayer() {
        let day = PrayerDay {
            day: 21,
            hijri: "1448-06-11".into(),
            imsak: 1,
            fajr: 2,
            syuruk: 3,
            dhuha: 4,
            dhuhr: 5,
            asr: 6,
            maghrib: 7,
            isha: 8,
        };
        // Zohor (index 4) is active.
        let line = times_line(Some(&day), Some("Zohor"));
        let bold_count = line.spans.iter().filter(|s| s.style.add_modifier.contains(Modifier::BOLD)).count();
        assert_eq!(bold_count, 1, "exactly one highlighted segment");
        assert!(line.spans.iter().any(|s| s.content.contains("Zohor")
            && s.style.add_modifier.contains(Modifier::BOLD)));
        // All other segments are dim.
        let others_dim = line
            .spans
            .iter()
            .filter(|s| !s.content.trim().is_empty() && s.content != " · ")
            .all(|s| s.style.add_modifier.contains(Modifier::DIM) || s.style.fg.is_some());
        assert!(others_dim);
        // Night → no highlight, all dim.
        let night = times_line(Some(&day), None);
        assert!(night.spans.iter().all(|s| s.style.add_modifier.contains(Modifier::DIM)));
        // Missing data → em dash.
        assert_eq!(times_line(None, Some("Zohor")).to_string(), "—");
    }
}
