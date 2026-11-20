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
    notify::Notifier as _,
    ui,
};

#[derive(Debug)]
pub enum Msg {
    ZonesLoaded(Result<Vec<Zone>>),
    MonthLoaded { code: String, year: i32, month: u32, result: Result<SolatResponse> },
    AutoZoneResolved(Result<Zone, String>),
    ReminderFailed(String),
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
    /// Picker rows: an auto-detect row pinned first (empty query only),
    /// then all matching zones.
    pub picker_rows: Vec<PickerRow>,
    pub picker_sel: usize,
    pub should_quit: bool,

    /// Vim-style numeric prefix, e.g. the `12` in `12j`. 0 = no prefix.
    count: u32,
    /// Rows visible in the table (set by the renderer); used for half-page scrolls.
    pub viewport_rows: u16,
    /// Zone code from `--zone` / positional arg; takes priority over config.
    zone_override: Option<String>,
    /// An auto-detection task is in flight.
    pub detecting: bool,

    /// Desktop reminder state (Milestone 7).
    pub notify_enabled: bool,
    pub notify_lead: u32,
    last_notified: Option<i64>,
    notify_available: Option<bool>,
    /// Transient footer message with a tick countdown.
    pub status: Option<(String, u8)>,

    /// Hijri date rendering style (Milestone 8).
    pub hijri_style: crate::hijri::HijriStyle,
}

/// One selectable row in the zone picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerRow {
    /// "Use my location" — pinned to the top when the query is empty.
    Auto,
    /// Index into `App::zones`.
    Zone(usize),
}

/// The prayer window `now_ts` falls inside: the most recent prayer whose
/// time has passed. The night gap (after Isyak, before tomorrow's Imsak)
/// belongs to Isyak's window.
pub fn current_prayer_from(row: &PrayerDay, now_ts: i64) -> (&'static str, i64) {
    for (label, t) in PrayerDay::KINDS.iter().zip(row.times()).rev() {
        if t <= now_ts {
            return (label, t);
        }
    }
    ("Isyak", row.isha)
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
        let cache = Cache::new()?;
        let cfg = cache.load_config();
        Ok(Self {
            api: ApiClient::new(),
            cache,
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
            picker_rows: Vec::new(),
            picker_sel: 0,
            should_quit: false,
            count: 0,
            viewport_rows: 12,
            zone_override,
            detecting: false,
            notify_enabled: cfg.notify_enabled(),
            notify_lead: cfg.notify_lead_minutes(),
            last_notified: cfg.last_notified,
            notify_available: cfg.notify_available,
            status: None,
            hijri_style: cfg.hijri_style(),
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
                // Offline parity with the CLI: fall back to a stale cache
                // (TTL ignored) before giving up with an error.
                let cache_name = cache::solat_cache_name(&code, year, month);
                if let Some((fetched_at, stale)) = self.cache.load::<SolatResponse>(&cache_name) {
                    self.months.insert(key, (fetched_at, stale));
                    self.sync_table_selection();
                    let age = cache::now_secs().saturating_sub(fetched_at);
                    let age_txt = if age >= 3600 {
                        format!("{}h", age / 3600)
                    } else {
                        format!("{}m", age / 60)
                    };
                    self.set_status(format!("Offline — showing cached data from {age_txt} ago"));
                } else {
                    self.error = Some(format!(
                        "Failed to load {code} {year}-{month:02}: {e:#} — press `r` to retry"
                    ));
                }
            }
        }
    }

    // ---------- navigation ----------

    fn select_zone(&mut self, zone: Zone) {
        let is_new = self.zone.as_ref() != Some(&zone);
        self.zone = Some(zone);
        if is_new {
            self.cache.store_config(&self.build_config());
            self.jump_to_today();
        }
        if let Some(z) = self.zone.clone() {
            self.request_month(z.jakim_code.clone(), self.view_year, self.view_month, false);
        }
    }

    /// Full config snapshot from current app state (used for every persist).
    fn build_config(&self) -> Config {
        Config {
            zone: self.zone.as_ref().map(|z| z.jakim_code.clone()),
            notify: Some(self.notify_enabled),
            notify_minutes: Some(self.notify_lead),
            last_notified: self.last_notified,
            notify_available: self.notify_available,
            hijri_style: Some(
                match self.hijri_style {
                    crate::hijri::HijriStyle::Malay => "malay",
                    crate::hijri::HijriStyle::Numeric => "numeric",
                    crate::hijri::HijriStyle::Arabic => "arabic",
                }
                .to_string(),
            ),
        }
    }

    /// Transient footer message, cleared after a few seconds.
    fn set_status(&mut self, msg: String) {
        self.status = Some((msg, 5));
    }

    /// Cycle the Hijri rendering style and persist it.
    fn cycle_hijri_style(&mut self) {
        self.hijri_style = match self.hijri_style {
            crate::hijri::HijriStyle::Malay => crate::hijri::HijriStyle::Numeric,
            crate::hijri::HijriStyle::Numeric => crate::hijri::HijriStyle::Arabic,
            crate::hijri::HijriStyle::Arabic => crate::hijri::HijriStyle::Malay,
        };
        let name = match self.hijri_style {
            crate::hijri::HijriStyle::Malay => "malay (21 Syawal)",
            crate::hijri::HijriStyle::Numeric => "numeric (21/10)",
            crate::hijri::HijriStyle::Arabic => "arabic (٢١ شوال)",
        };
        self.set_status(format!("Hijri style: {name}"));
        self.cache.store_config(&self.build_config());
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

    /// The prayer window we are currently inside:
    /// `(prayer, epoch_secs_of_its_start)` for the most recent prayer whose
    /// time has passed. The night gap (after Isyak, before tomorrow's Imsak)
    /// belongs to Isyak's window — its entry is returned so the header, the
    /// prayer list and the table column stay consistent. `None` only when
    /// there is no data for today.
    pub fn current_prayer(&self) -> Option<(&'static str, i64)> {
        let code = self.zone.as_ref()?.jakim_code.clone();
        let now = Local::now();
        let (_, today_data) = self.months.get(&(code, now.year(), now.month()))?;
        let row = today_data.prayers.iter().find(|p| p.day == now.day())?;
        Some(current_prayer_from(row, now.timestamp()))
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
        self.tick_status();
        self.check_reminder();
    }

    /// Expire the transient footer message.
    fn tick_status(&mut self) {
        if let Some((_, remaining)) = &mut self.status {
            if *remaining == 0 {
                self.status = None;
            } else {
                *remaining -= 1;
            }
        }
    }

    /// Fire the desktop reminder when we enter the lead window of the next
    /// prayer (once per occurrence).
    fn check_reminder(&mut self) {
        if !self.notify_enabled || self.notify_available == Some(false) {
            return;
        }
        let Some((label, next_at, _)) = self.next_prayer() else {
            return;
        };
        let now_ts = Local::now().timestamp();
        if !crate::notify::should_notify(now_ts, next_at, self.notify_lead, self.last_notified) {
            return;
        }
        // Mark this occurrence immediately (in-memory dedup); the task below
        // persists it. If the send fails, we disable notifications entirely.
        self.last_notified = Some(next_at);
        let zone_display = self.zone.as_ref().map(|z| z.display()).unwrap_or_default();
        let (title, body) = crate::notify::reminder_text(label, next_at, now_ts, &zone_display);
        // NOTE: snapshot taken after the dedup update. A concurrent `n` toggle
        // writing the config in the same second would be overwritten by this
        // task's snapshot — harmless (last write wins within milliseconds).
        let cfg = self.build_config();
        let cache = self.cache.clone();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            match crate::notify::DesktopNotifier.send(&title, &body) {                Ok(()) => cache.store_config(&cfg),
                Err(e) => {
                    let mut cfg = cfg;
                    cfg.notify_available = Some(false);
                    cache.store_config(&cfg);
                    let _ = tx.send(Msg::ReminderFailed(format!("{e:#}")));
                }
            }
        });
    }

    fn update_picker_filter(&mut self) {
        let q = self.picker_query.to_lowercase();
        let mut rows = Vec::new();
        // The auto-detect row is only meaningful with no search text.
        if q.is_empty() {
            rows.push(PickerRow::Auto);
        }
        rows.extend(
            self.zones
                .iter()
                .enumerate()
                .filter(|(_, z)| {
                    z.jakim_code.to_lowercase().contains(&q)
                        || z.negeri.to_lowercase().contains(&q)
                        || z.daerah.to_lowercase().contains(&q)
                })
                .map(|(i, _)| PickerRow::Zone(i)),
        );
        self.picker_rows = rows;
        if self.picker_rows.is_empty() {
            self.picker_sel = 0;
        } else {
            self.picker_sel = self.picker_sel.min(self.picker_rows.len() - 1);
        }
    }

    fn open_picker(&mut self) {
        self.mode = Mode::Picker;
        self.picker_query.clear();
        self.picker_sel = 0;
        self.update_picker_filter();
    }

    fn picker_move(&mut self, delta: i32) {
        if self.picker_rows.is_empty() {
            return;
        }
        let len = self.picker_rows.len() as i32;
        self.picker_sel = ((self.picker_sel as i32 + delta).rem_euclid(len)) as usize;
    }

    fn picker_confirm(&mut self) {
        match self.picker_rows.get(self.picker_sel) {
            Some(PickerRow::Zone(idx)) => {
                if let Some(zone) = self.zones.get(*idx).cloned() {
                    self.mode = Mode::Normal;
                    self.select_zone(zone);
                }
            }
            Some(PickerRow::Auto) => {
                self.mode = Mode::Normal;
                self.start_detect();
            }
            None => {}
        }
    }

    /// Kick off an auto-detection task (IP geolocation → server geo route →
    /// centroid fallback). Idempotent while one is in flight.
    fn start_detect(&mut self) {
        if self.detecting {
            return;
        }
        self.detecting = true;
        self.error = None;
        let tx = self.tx.clone();
        let zones = self.zones.clone();
        tokio::spawn(async move {
            let result = async {
                let api = ApiClient::new();
                crate::geo::detect_zone(&api, &zones, None).await
            }
            .await;
            let _ = tx.send(Msg::AutoZoneResolved(result.map_err(|e| format!("{e:#}"))));
        });
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
            KeyCode::Char('s') | KeyCode::Char('a') => self.open_picker(),
            KeyCode::Char('c') => self.cycle_hijri_style(),
            KeyCode::Char('n') => {
                self.notify_enabled = !self.notify_enabled;
                // Allow retrying D-Bus if it failed on a previous session.
                if self.notify_enabled {
                    self.notify_available = None;
                }
                let state = if self.notify_enabled { "on" } else { "off" };
                self.set_status(format!("Desktop notifications {state} (lead {}m)", self.notify_lead));
                self.cache.store_config(&self.build_config());
            }
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
            Msg::AutoZoneResolved(result) => {
                self.detecting = false;
                match result {
                    Ok(zone) => {
                        self.error = None;
                        self.select_zone(zone);
                    }
                    Err(e) => {
                        self.error = Some(format!("Auto-detect failed: {e} — pick a zone manually"));
                    }
                }
            }
            Msg::ReminderFailed(e) => {
                self.notify_available = Some(false);
                self.set_status(format!("Notifications unavailable: {e}"));
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row() -> PrayerDay {
        // 2026-11-21 (MYT): Imsak 05:40 … Isyak 20:13.
        PrayerDay {
            day: 21,
            hijri: "1448-06-11".into(),
            imsak: 1795210800,
            fajr: 1795211400,
            syuruk: 1795215540,
            dhuha: 1795217040,
            dhuhr: 1795237320,
            asr: 1795249440,
            maghrib: 1795258800,
            isha: 1795263180,
        }
    }

    #[test]
    fn window_between_prayers() {
        // 14:00-ish: inside Zohor's window.
        assert_eq!(
            current_prayer_from(&sample_row(), 1795240000),
            ("Zohor", 1795237320)
        );
        // Exactly at Maghrib: its window starts.
        assert_eq!(
            current_prayer_from(&sample_row(), 1795258800),
            ("Maghrib", 1795258800)
        );
    }

    #[test]
    fn night_belongs_to_isyak() {
        // 02:00-ish: before Imsak — Isyak's window.
        assert_eq!(
            current_prayer_from(&sample_row(), 1795197083),
            ("Isyak", 1795263180)
        );
        // Just after Isyak: still Isyak.
        assert_eq!(
            current_prayer_from(&sample_row(), 1795263181),
            ("Isyak", 1795263180)
        );
        // Exactly at Imsak: Imsak's window starts (night ends).
        assert_eq!(
            current_prayer_from(&sample_row(), 1795210800),
            ("Imsak", 1795210800)
        );
    }
}
