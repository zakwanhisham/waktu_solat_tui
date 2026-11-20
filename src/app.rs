use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::widgets::TableState;
use tokio::sync::mpsc;

use crate::{
    api::{ApiClient, PrayerDay, SolatResponse, Zone},
    cache::{self, Cache, Config},
    ui,
};

#[derive(Debug)]
pub enum Msg {
    ZonesLoaded(Result<Vec<Zone>>),
    MonthLoaded { code: String, year: i32, month: u32, result: Result<SolatResponse> },
}

/// Terminal input forwarded from the blocking reader thread.
#[derive(Debug)]
pub enum InputEvent {
    Key(KeyEvent),
    Resize,
}

type MonthKey = (String, i32, u32);

pub struct App {
    pub api: ApiClient,
    pub cache: Cache,
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    pub zones: Vec<Zone>,
    pub zone: Option<Zone>,

    /// All month data we have, keyed by (zone code, year, month).
    pub months: HashMap<MonthKey, (u64, SolatResponse)>,
    /// Months for which a fetch task is already running.
    pending: HashSet<MonthKey>,
    pub loading: bool,
    pub error: Option<String>,

    /// View cursor (which month the table shows).
    pub view_year: i32,
    pub view_month: u32,

    pub table_state: TableState,

    pub mode: Mode,
    pub picker_query: String,
    pub picker_filtered: Vec<usize>,
    pub picker_sel: usize,
    pub should_quit: bool,

    /// Vim-style numeric prefix, e.g. the `12` in `12j`. 0 = no prefix.
    count: u32,
    /// Rows visible in the table (set by the renderer); used for half-page scrolls.
    pub viewport_rows: u16,
    /// Zone code from `--zone` / positional arg; takes priority over config.
    zone_override: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Picker,
    Help,
}

impl App {
    pub fn new(zone_override: Option<String>) -> Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let now = Local::now();
        Ok(Self {
            api: ApiClient::new(),
            cache: Cache::new()?,
            tx,
            rx,
            zones: Vec::new(),
            zone: None,
            months: HashMap::new(),
            pending: HashSet::new(),
            loading: false,
            error: None,
            view_year: now.year(),
            view_month: now.month(),
            table_state: TableState::default(),
            mode: Mode::Normal,
            picker_query: String::new(),
            picker_filtered: Vec::new(),
            picker_sel: 0,
            should_quit: false,
            count: 0,
            viewport_rows: 12,
            zone_override,
        })
    }

    /// Receive the next message from any background fetch task.
    pub async fn recv_msg(&mut self) -> Option<Msg> {
        self.rx.recv().await
    }

    /// Load persisted zone, then zones (cache-first), then the month data.
    pub fn bootstrap(&mut self) {
        // 1) Restore zones from cache if fresh.
        let mut zones_ok = false;
        if let Some((fetched_at, zones)) = self.cache.load::<Vec<Zone>>(cache::ZONES_FILE)
            && cache::now_secs() < fetched_at + cache::ZONES_TTL {
                zones_ok = true;
                self.set_zones(zones);
            }
        if !zones_ok {
            self.request_zones();
        }

        // 2) Ask for the current month even while zones load; the zone is
        // resolved when zones arrive (CLI override > persisted config > first).
        self.set_zones(self.zones.clone());
        if let Some(z) = self.zone.clone() {
            self.jump_to_today();
            self.request_month(z.jakim_code.clone(), self.view_year, self.view_month, false);
        }
    }

    // ---------- data plumbing ----------

    fn request_zones(&mut self) {
        let tx = self.tx.clone();
        let api = self.api.clone();
        tokio::spawn(async move {
            let result = api.zones().await;
            let _ = tx.send(Msg::ZonesLoaded(result));
        });
    }

    pub fn request_month(&mut self, code: String, year: i32, month: u32, force: bool) {
        let key = (code.clone(), year, month);
        if !force
            && let Some((fetched_at, payload)) = self.cache.load::<SolatResponse>(&cache::solat_cache_name(&code, year, month))
                && cache::now_secs() < fetched_at + cache::MONTH_TTL {
                    self.months.insert(key, (fetched_at, payload));
                    self.sync_table_selection();
                    return;
                }
        if self.pending.insert(key.clone()) {
            self.loading = true;
            let (tx, api, cache) = (self.tx.clone(), self.api.clone(), self.cache.clone());
            tokio::spawn(async move {
                let result = match api.solat(&code, year, month).await {
                    Ok(data) => {
                        cache.store(&cache::solat_cache_name(&code, year, month), &data);
                        Ok(data)
                    }
                    Err(e) => Err(e),
                };
                let _ = tx.send(Msg::MonthLoaded { code, year, month, result });
            });
        }
    }

    fn set_zones(&mut self, zones: Vec<Zone>) {
        self.zones = zones;
        if self.zone.is_some() {
            return;
        }
        let wanted = self.cache.load_config().zone;
        match crate::cli::resolve_zone(
            &self.zones,
            wanted.as_deref(),
            self.zone_override.as_deref(),
        ) {
            Ok(zone) => self.zone = Some(zone),
            Err(e) => {
                self.error = Some(format!("{e} — falling back to default"));
                self.zone = self.zones.first().cloned();
            }
        }
    }

    fn on_zones_loaded(&mut self, result: Result<Vec<Zone>>) {
        match result {
            Ok(zones) => {
                self.error = None;
                self.set_zones(zones);
                self.cache.store(cache::ZONES_FILE, &self.zones);
                if let Some(z) = self.zone.clone() {
                    self.request_month(z.jakim_code.clone(), self.view_year, self.view_month, false);
                }
            }
            Err(e) => {
                self.error = Some(format!("Failed to load zones: {e:#} — press `r` to retry"));
            }
        }
    }

    fn on_month_loaded(&mut self, code: String, year: i32, month: u32, result: Result<SolatResponse>) {
        let key = (code.clone(), year, month);
        self.pending.remove(&key);
        match result {
            Ok(data) => {
                self.error = None;
                self.loading = false;
                self.months.insert(key, (cache::now_secs(), data));
                self.sync_table_selection();
            }
            Err(e) => {
                self.loading = false;
                self.error = Some(format!("Failed to load {code} {year}-{month:02}: {e:#} — press `r` to retry"));
            }
        }
    }

    // ---------- navigation ----------

    fn select_zone(&mut self, zone: Zone) {
        let is_new = self.zone.as_ref() != Some(&zone);
        self.zone = Some(zone);
        if is_new {
            self.cache.store_config(&Config {
                zone: self.zone.as_ref().map(|z| z.jakim_code.clone()),
            });
            self.jump_to_today();
        }
        if let Some(z) = self.zone.clone() {
            self.request_month(z.jakim_code.clone(), self.view_year, self.view_month, false);
        }
    }

    fn jump_to_today(&mut self) {
        let now = Local::now();
        self.view_year = now.year();
        self.view_month = now.month();
    }

    fn shift_month(&mut self, delta: i32) {
        let total = self.view_year * 12 + (self.view_month as i32 - 1) + delta;
        self.view_year = total.div_euclid(12);
        self.view_month = (total.rem_euclid(12) + 1) as u32;
        if let Some(z) = self.zone.clone() {
            self.request_month(z.jakim_code.clone(), self.view_year, self.view_month, false);
        }
    }

    pub fn view_key(&self) -> Option<MonthKey> {
        self.zone.as_ref().map(|z| (z.jakim_code.clone(), self.view_year, self.view_month))
    }

    // ---------- table ----------

    fn sync_table_selection(&mut self) {
        if self.view_key().is_none() {
            return;
        }
        let (view_year, view_month) = (self.view_year, self.view_month);
        let today = Local::now();
        let idx = self
            .view_data()
            .map(|(_, data)| {
                data.prayers
                    .iter()
                    .position(|p| {
                        today.year() == view_year && today.month() == view_month && p.day == today.day()
                    })
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        self.table_state.select(Some(idx));
    }

    /// `(fetched_at, data)` for the month currently displayed.
    pub fn view_data(&self) -> Option<(u64, &SolatResponse)> {
        self.view_key().and_then(|k| self.months.get(&k)).map(|(t, d)| (*t, d))
    }

    fn table_move(&mut self, delta: i32) {
        let len = self.view_data().map(|(_, d)| d.prayers.len()).unwrap_or(0);
        if len == 0 {
            return;
        }
        let current = self.table_state.selected().unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, len as i32 - 1);
        self.table_state.select(Some(next as usize));
    }

    // ---------- next prayer ----------

    /// `(prayer, epoch_secs, is_tomorrow)` for the upcoming prayer at the
    /// selected zone, regardless of which month is being viewed.
    pub fn next_prayer(&self) -> Option<(&'static str, i64, bool)> {
        let code = self.zone.as_ref()?.jakim_code.clone();
        let now = Local::now();
        let (_, today_data) = self.months.get(&(code.clone(), now.year(), now.month()))?;
        let row = today_data.prayers.iter().find(|p| p.day == now.day())?;

        let now_ts = now.timestamp();
        for (label, t) in PrayerDay::KINDS.iter().zip(row.times()) {
            if t > now_ts {
                return Some((label, t, false));
            }
        }
        // After Isyak → tomorrow's Imsak.
        let tomorrow = NaiveDate::from_ymd_opt(now.year(), now.month(), now.day())?
            .succ_opt()?;
        if tomorrow.month() == now.month() {
            let r2 = today_data.prayers.iter().find(|p| p.day == tomorrow.day())?;
            return Some(("Imsak", r2.imsak, true));
        }
        let (_, next_data) = self.months.get(&(code, tomorrow.year(), tomorrow.month()))?;
        let r2 = next_data.prayers.first()?;
        Some(("Imsak", r2.imsak, true))
    }

    /// The prayer window we are currently inside, if any:
    /// `(prayer, epoch_secs_of_its_start)` for the most recent prayer whose
    /// time has passed. `None` = night (after Isyak, before tomorrow's Imsak).
    pub fn current_prayer(&self) -> Option<(&'static str, i64)> {
        let code = self.zone.as_ref()?.jakim_code.clone();
        let now = Local::now();
        let (_, today_data) = self.months.get(&(code, now.year(), now.month()))?;
        let row = today_data.prayers.iter().find(|p| p.day == now.day())?;
        let now_ts = now.timestamp();
        PrayerDay::KINDS
            .iter()
            .zip(row.times())
            .rev()
            .find(|(_, t)| *t <= now_ts)
            .map(|(label, t)| (*label, t))
    }

    fn ensure_tomorrow_data(&mut self) {
        // Prime the cache with next month's data when today is the last day of
        // the month, so the countdown survives midnight.
        let Some(zone) = self.zone.clone() else { return };
        let now = Local::now();
        let Some(today) = NaiveDate::from_ymd_opt(now.year(), now.month(), now.day()) else {
            return;
        };
        let Some(tomorrow) = today.succ_opt() else { return };
        if tomorrow.month() == today.month() {
            return; // tomorrow is in the same (already cached) month
        }
        let key = (zone.jakim_code.clone(), tomorrow.year(), tomorrow.month());
        if !self.months.contains_key(&key) {
            self.request_month(zone.jakim_code, tomorrow.year(), tomorrow.month(), false);
        }
    }

    // ---------- events ----------

    pub fn on_tick(&mut self) {
        self.ensure_tomorrow_data();
    }

    fn update_picker_filter(&mut self) {
        let q = self.picker_query.to_lowercase();
        self.picker_filtered = self
            .zones
            .iter()
            .enumerate()
            .filter(|(_, z)| {
                z.jakim_code.to_lowercase().contains(&q)
                    || z.negeri.to_lowercase().contains(&q)
                    || z.daerah.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
        if self.picker_filtered.is_empty() {
            self.picker_sel = 0;
        } else {
            self.picker_sel = self.picker_sel.min(self.picker_filtered.len() - 1);
        }
    }

    fn open_picker(&mut self) {
        self.mode = Mode::Picker;
        self.picker_query.clear();
        self.picker_sel = 0;
        self.update_picker_filter();
    }

    fn picker_move(&mut self, delta: i32) {
        if self.picker_filtered.is_empty() {
            return;
        }
        let len = self.picker_filtered.len() as i32;
        self.picker_sel = ((self.picker_sel as i32 + delta).rem_euclid(len)) as usize;
    }

    fn picker_confirm(&mut self) {
        if let Some(&idx) = self.picker_filtered.get(self.picker_sel)
            && let Some(zone) = self.zones.get(idx).cloned() {
                self.mode = Mode::Normal;
                self.select_zone(zone);
            }
    }

    pub async fn on_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.kind == KeyEventKind::Release {
            return Ok(false);
        }
        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            return Ok(true);
        }
        match self.mode {
            Mode::Normal => self.on_key_normal(key),
            Mode::Picker => self.on_key_picker(key),
            Mode::Help => self.on_key_help(key),
        }
        Ok(self.should_quit)
    }

    fn on_key_normal(&mut self, key: KeyEvent) {
        // Vim-style numeric prefix (`12j` moves 12 rows down). `0` only starts
        // a count when one is already in progress (bare `0` is ignored).
        if let KeyCode::Char(c) = key.code
            && ((c.is_ascii_digit() && c != '0') || (c == '0' && self.count > 0)) {
                self.count = (self.count.saturating_mul(10) + u32::from(c as u8 - b'0')).min(999);
                return;
            }
        let n = if self.count == 0 { 1 } else { self.count };
        self.count = 0;
        let half_page = (self.viewport_rows / 2).max(1) as i32;

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('s') => self.open_picker(),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('j') | KeyCode::Down => self.table_move(n as i32),
            KeyCode::Char('k') | KeyCode::Up => self.table_move(-(n as i32)),
            KeyCode::Char('g') => self.table_select_top(),
            KeyCode::Char('G') => self.table_select_bottom(),
            KeyCode::Char('d') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                self.table_move(half_page);
            }
            KeyCode::Char('u') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                self.table_move(-half_page);
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char(',') => self.shift_month(-1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('.') => self.shift_month(1),
            KeyCode::Char('t') => {
                self.jump_to_today();
                if let Some(z) = self.zone.clone() {
                    self.request_month(z.jakim_code.clone(), self.view_year, self.view_month, false);
                }
            }
            KeyCode::Char('r') => {
                if self.zones.is_empty() {
                    self.request_zones();
                }
                if let Some(z) = self.zone.clone() {
                    self.request_month(z.jakim_code.clone(), self.view_year, self.view_month, true);
                }
            }
            _ => {}
        }
    }

    fn table_select_top(&mut self) {
        if self.view_data().is_some_and(|(_, d)| !d.prayers.is_empty()) {
            self.table_state.select(Some(0));
        }
    }

    fn table_select_bottom(&mut self) {
        if let Some((_, d)) = self.view_data() {
            let len = d.prayers.len();
            if len > 0 {
                self.table_state.select(Some(len - 1));
            }
        }
    }

    fn on_key_picker(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => self.picker_confirm(),
            KeyCode::Up => self.picker_move(-1),
            KeyCode::Down => self.picker_move(1),
            // Plain letters go into the search box, so picker navigation uses
            // Ctrl-n / Ctrl-p (readline convention) or the arrow keys.
            KeyCode::Char('n') if ctrl => self.picker_move(1),
            KeyCode::Char('p') if ctrl => self.picker_move(-1),
            KeyCode::Backspace => {
                self.picker_query.pop();
                self.update_picker_filter();
            }
            KeyCode::Char(c) => {
                self.picker_query.push(c);
                self.update_picker_filter();
            }
            _ => {}
        }
    }

    fn on_key_help(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::ZonesLoaded(result) => self.on_zones_loaded(result),
            Msg::MonthLoaded { code, year, month, result } => self.on_month_loaded(code, year, month, result),
        }
    }
}

/// Signals the input-reader thread to stop when dropped (covers panics too).
struct QuitGuard(Arc<AtomicBool>);

impl Drop for QuitGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Run the TUI event loop until the user quits.
pub async fn run(terminal: &mut ratatui::DefaultTerminal, zone_override: Option<String>) -> Result<()> {
    let mut app = App::new(zone_override)?;
    app.bootstrap();

    // Keyboard events are read on a dedicated blocking thread that *polls*
    // with a short timeout instead of blocking forever in `event::read()`:
    // a permanently blocked blocking-task would hang tokio's runtime shutdown
    // when `main` returns, leaving the terminal stuck until Ctrl+C.
    let quit = Arc::new(AtomicBool::new(false));
    let _guard = QuitGuard(quit.clone());
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<InputEvent>();
    let reader = {
        let quit = quit.clone();
        tokio::task::spawn_blocking(move || {
            while !quit.load(Ordering::Relaxed) {
                match crossterm::event::poll(Duration::from_millis(100)) {
                    Ok(true) => match crossterm::event::read() {
                        Ok(Event::Key(key)) => {
                            if event_tx.send(InputEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                        // Redraw immediately on resize instead of waiting up
                        // to a second for the next tick.
                        Ok(Event::Resize(..)) => {
                            if event_tx.send(InputEvent::Resize).is_err() {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    },
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        })
    };

    let result = event_loop(&mut app, terminal, &mut event_rx).await;

    // Stop the reader and join it *before* the runtime shuts down, so the
    // process exits promptly on every path (quit key or error).
    quit.store(true, Ordering::Relaxed);
    let _ = reader.await;
    result
}

async fn event_loop(
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    event_rx: &mut mpsc::UnboundedReceiver<InputEvent>,
) -> Result<()> {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        tokio::select! {
            _ = ticker.tick() => app.on_tick(),
            Some(event) = event_rx.recv() => match event {
                InputEvent::Key(key) => {
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                        && app.on_key(key).await? {
                            break;
                        }
                }
                InputEvent::Resize => {}
            },
            Some(msg) = app.recv_msg() => app.on_msg(msg),
        }
    }
    Ok(())
}
