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
    f.render_widget(
        Paragraph::new(vec![
            spans,
            Line::from(Span::styled(format!("{date}"), Style::new().dim())),
        ]),
        area,
    );
}

fn draw_next_prayer(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered().title(" Next ").title_style(Style::new().fg(ACCENT).bold());

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
            let line2 = match todays {
                Some(t) => Line::from(Span::styled(
                    format!(
                        "Imsak {} · Subuh {} · Syuruk {} · Dhuha {} · Zohor {} · Asar {} · Maghrib {} · Isyak {}",
                        fmt_epoch(t.imsak), fmt_epoch(t.fajr), fmt_epoch(t.syuruk), fmt_epoch(t.dhuha),
                        fmt_epoch(t.dhuhr), fmt_epoch(t.asr), fmt_epoch(t.maghrib), fmt_epoch(t.isha),
                    ),
                    Style::new().dim(),
                )),
                None => Line::from(Span::styled("—", Style::new().dim())),
            };
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

fn draw_table(f: &mut Frame, app: &mut App, area: Rect) {
    // Rows visible inside the bordered block: height - 2 borders - 1 header
    // row. The app uses this to size half-page scrolls (Ctrl-d / Ctrl-u).
    app.viewport_rows = area.height.saturating_sub(3).max(1);

    let title = match (&app.zone, app.view_data()) {
        (Some(z), Some((fetched, data))) => {
            let age = cache_age(fetched);
            let updated = data
                .last_updated
                .and_then(|ts| DateTime::from_timestamp(ts, 0))
                .map(|t| format!(" · JAKIM updated {}", t.with_timezone(&Local).format("%d %b %Y %H:%M")))
                .unwrap_or_default();
            format!(
                " {} {:04} · {} {} · {} ({}){} ",
                month_name(app.view_month), app.view_year, z.jakim_code, z.negeri, z.daerah, age, updated
            )
        }
        (Some(_), None) => {
            format!(" {} {:04} — no data ", month_name(app.view_month), app.view_year)
        }
        _ => format!(" {} {:04} ", month_name(app.view_month), app.view_year),
    };

    let widths = [
        Constraint::Length(5),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(7),
    ];

    let header = Row::new([
        "Day", "Hijri", "Imsak", "Subuh", "Syuruk", "Dhuha", "Zohor", "Asar", "Maghrib", "Isyak",
    ])
    .style(Style::new().bold().fg(ACCENT))
    .bottom_margin(0);

    let today = Local::now().date_naive();
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
                    Row::new(prayer_cells(p)).style(style)
                })
                .collect()
        })
        .unwrap_or_default();

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(Span::styled(title, Style::new().bold())),
        )
        .column_spacing(1)
        // White on strong blue: readable in any terminal theme (fixed
        // 256-palette entries, not theme-remappable 16-color values).
        .row_highlight_style(
            Style::new().bold().fg(Color::Indexed(231)).bg(Color::Indexed(25)),
        )
        .highlight_symbol("› ");

    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn prayer_cells(p: &PrayerDay) -> Vec<Cell<'static>> {
    vec![
        Cell::from(format!("{:2}", p.day)),
        Cell::from(short_hijri(&p.hijri)),
        Cell::from(fmt_epoch(p.imsak)),
        Cell::from(fmt_epoch(p.fajr)),
        Cell::from(fmt_epoch(p.syuruk)),
        Cell::from(fmt_epoch(p.dhuha)),
        Cell::from(fmt_epoch(p.dhuhr)),
        Cell::from(fmt_epoch(p.asr)),
        Cell::from(fmt_epoch(p.maghrib)),
        Cell::from(fmt_epoch(p.isha)),
    ]
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

fn short_hijri(hijri: &str) -> String {
    let parts: Vec<&str> = hijri.split('-').collect();
    if parts.len() == 3 {
        format!("{}/{}", parts[2], parts[1])
    } else {
        hijri.to_string()
    }
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
