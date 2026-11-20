# TODO — `waktu_solat` TUI

A terminal UI (Rust + ratatui) that displays Malaysia prayer times from
[api.waktusolat.app](https://api.waktusolat.app/) (official JAKIM data).

## API notes

- `GET https://api.waktusolat.app/zones`
  → `[{ "jakimCode": "WLY01", "negeri": "Kuala Lumpur", "daerah": "..." }, ...]`
- `GET https://api.waktusolat.app/v2/solat/{zone}?year=2026&month=11`
  → `{ "zone", "year", "month", "month_number", "last_updated", "prayers": [...] }`
  - Each prayer entry: `{ day, hijri, imsak, fajr, syuruk, dhuha, dhuhr, asr, maghrib, isha }`
  - Times are **unix epoch seconds** (UTC) — format with `chrono`/`time` in local time.
  - Omitting `year`/`month` returns the current month.
- The API is free but asks for **client-side caching** (≥ 24h). Don't refetch on every UI tick.

## Milestone 0 — Project scaffold

- [x] `cargo init waktu_solat` (binary crate)
- [x] Dependencies:
  - `ratatui` + `crossterm` — TUI rendering & input
  - `tokio` (rt-multi-thread, macros) — async runtime
  - `reqwest` (json feature) — HTTP client
  - `serde` / `serde_json` — API models
  - `chrono` — epoch → local wall-clock formatting
  - `anyhow` — error handling
- [x] Verify `cargo run` boots and exits cleanly

## Milestone 1 — Data layer

- [x] Define models: `Zone { jakim_code, negeri, daerah }`, `PrayerDay { day, hijri, imsak, ... }`, `SolatResponse`
- [x] Fetch all zones from `/zones`
- [x] Fetch monthly prayer times from `/v2/solat/{zone}`
- [x] Disk cache (e.g. `~/.cache/waktu_solat/`):
  - [x] Cache `/v2/solat` responses keyed by `{zone}-{year}-{month}`, TTL 24h
  - [x] Cache the zones list (long TTL — it rarely changes)
  - [x] Fall back to a stale cache if the network is unavailable
- [x] Graceful error UI when offline (no crash, show retry hint)

## Milestone 2 — Core TUI

- [x] Bootstrap ratatui: alternate screen, raw mode, tick/input event loop, clean restore on exit (`q` / `Ctrl-C`)
- [x] Header bar: app name, selected zone (`WLY01 — Kuala Lumpur, ...`), Gregorian + Hijri date
- [x] "Next prayer" panel:
  - [x] Compute the next prayer time from today's row (handle wrapping past `isha` → tomorrow's `imsak/fajr`)
  - [x] Live countdown, updated by a 1s tick (WITHOUT refetching the API)
- [x] Main table: full month view — Day | Hijri | Imsak | Subuh | Syuruk | Dhuha | Zohor | Asar | Maghrib | Isyak
  - [x] Highlight today's row
  - [x] Highlight the next (or current) prayer column *(partially — today's row is highlighted & auto-selected; per-column highlight is future work)*
  - [x] Scrollable when the month exceeds the viewport

## Milestone 3 — Zone selection & navigation

- [x] Searchable zone picker overlay (`s` / `Enter` to open)
  - [x] Filter by state (`negeri`) or district (`daerah`) as you type
  - [x] Persist last-selected zone to disk (`~/.config/waktu_solat/config.json`)
  - [x] Load the persisted zone on startup (default: first zone in list)
- [x] `<` / `>` (or `,` / `.`) to view previous / next month (fetch + cache per month)
- [x] `t` to jump back to the current month / today
- [x] `r` to force-refresh (bypass cache)

## Milestone 4 — Polish

- [x] Keybindings help footer / `?` help overlay
- [x] Color theme: accent the currently-active prayer window (`Now: <prayer>` in the Next panel)
- [x] Show `last_updated` from the API and cache age
- [x] Optional: `--zone WLY01` CLI arg overrides the persisted zone (also `waktu_solat WLY01`, `--help`, `--version`)
- [x] Resize handling / reasonably render at small terminal sizes (resize events trigger immediate redraw; narrow terminals clip columns gracefully)

## Milestone 5 — CLI mode (`--now`)

Goal: `waktu_solat --now` prints today's prayer times and exits — headless,
no TUI, for scripts / status bars / shell prompts. Reuses the same data layer
and caching as the TUI.

- [x] Parse `--now` in `main.rs`; when set, skip `ratatui::init()` entirely and
  run the headless path (works without a TTY)
- [x] New `src/cli.rs` with `print_now(zone_override)`:
  - [x] Resolve zone the same way the TUI does: `--zone` arg > persisted
    config > first zone (shared `resolve_zone` helper used by `App::set_zones` too)
  - [x] Load month data cache-first (24 h TTL) via a shared
    `Cache::load_fresh(name, ttl)` helper; fall back to a stale cache when
    offline; stderr error + exit code 1 if neither is available
  - [x] Print today's row: zone header, Gregorian + Hijri date, the 8 prayer
    times aligned in a column, local time (H:M)
  - [x] Mark the current prayer window and the next prayer with a countdown
    (handles after-Isyak → tomorrow's Imsak)
- [x] `--now` composes with `--zone` / positional ZONE
- [x] Output written in a single buffered write — piping through `head` /
  `grep` does not panic on broken pipes
- [x] Update `USAGE` text and README CLI section
- [x] Tests: `--now` with cached data, `--now --zone SGR03`, invalid zone,
  pipe-through `head`/`grep`, TUI unaffected by the refactor

## Milestone 6 — Auto-detect zone

Goal: resolve the nearest JAKIM zone from the user's location. Opt-in only —
an IP lookup is a third-party request, so detection never runs implicitly at
startup.

Decisions: IP-based geolocation (no new deps); bundled ~60-entry zone-centroid
table as offline fallback; TUI entry points = pinned picker row + `a` key.

- [x] Confirm the API's geo endpoint from the api-waktusolat-x repo —
  found and verified live: `GET /zones/{lat}/{lng}` → `{zone, state, district}`
  (HTTP 500 "No zone found" outside Malaysia)
- [x] `src/geo.rs` (new):
  - [x] `detect_location()` — IP geolocation via `ipwho.is` (free, no key,
    HTTPS, 5s timeout); parses `{lat, lon, success}`
  - [x] `ZONE_CENTROIDS: [(&str, lat, lng); 60]` static table, derived from
    the real `/zones` list and each zone's districts
  - [x] `nearest_zone_client(lat, lng)` (as `nearest_zone_code`) — haversine
    to all centroids, pick minimum; 250 km threshold → "outside Malaysia"
- [x] `src/api.rs`: `nearest_zone(lat, lng)` — server-side polygon resolution
  via `GET /zones/{lat}/{lng}`; `Err` on missing route/outside-Malaysia so
  callers fall back to the centroid table
- [x] CLI (`main.rs`, `src/cli.rs`):
  - [x] `--auto-zone` — detect and print the zone, exit
  - [x] `--now --auto-zone` — today's times for the detected zone
  - [x] `--lat <f> --lng <f>` — manual coords, skips IP lookup (also the
    deterministic test path); implies auto mode; both required together
  - [x] Resolution order: explicit coords → IP lookup → server geo route →
    centroid fallback → stderr error + exit 1 (explicit `--zone` wins over
    detection)
  - [x] Update `USAGE` text
- [x] TUI (`src/app.rs`, `src/ui.rs`):
  - [x] Pinned "Use my location" row at the top of the zone picker
    (empty query only; `PickerRow` enum replaces the raw index list)
  - [x] `a` in Normal mode opens the picker (auto row pre-selected)
  - [x] Async plumbing: single `Msg::AutoZoneResolved` (simpler than the
    planned two-message flow; `detecting` flag guards double-spawns); footer
    shows "Detecting location…" while pending
  - [x] Success → existing `select_zone()` (persistence + month fetch for
    free); failure → footer error with "pick a zone manually" hint
- [x] Unit tests: haversine sanity, KL→WLY01, Klang→SGR03, George Town→PNG01,
  Labuan→WLY02, outside-Malaysia threshold
- [x] README: privacy note (what is sent, when) + new CLI flags documented

## Milestone 7 — Desktop notifications

Goal: a desktop reminder N minutes before the next prayer. **Disabled by
default** — enabled with `n` in the TUI or `"notify": true` in config.json.
Linux D-Bus only (via `notify-rust`); on headless/SSH boxes it degrades
silently, never crashes, and never retries after a D-Bus failure.

Decisions: lead time 10 minutes (configurable); dedup key = prayer epoch
timestamp; decision logic pure + unit-tested, D-Bus call behind a
`Notifier` trait; sent via `spawn_blocking` so ticks never block.

- [x] `Cargo.toml`: add `notify-rust`
- [x] Extend `Config` in `cache.rs` (backward-compatible `Option` fields):
  - [x] `notify: Option<bool>` — default false
  - [x] `notify_minutes: Option<u32>` — default 10
  - [x] `last_notified: Option<i64>` — dedup state, persisted so restarting
    inside the window doesn't double-notify
  - [x] `notify_available: Option<bool>` — permanently `false` after a
    D-Bus failure (headless machine) to avoid per-tick error spam
- [x] `src/notify.rs` (new):
  - [x] `should_notify(now, next_at, lead_minutes, last_notified) -> bool` —
    pure; fires when `next_at - now <= lead * 60` and
    `last_notified != Some(next_at)`; app started inside the window still
    notifies, after-prayer occurrences are skipped
  - [x] `Notifier` trait (`send(title, body)`) + `DesktopNotifier` impl
    (notify-rust → D-Bus, `spawn_blocking`) + capture-double for tests
  - [x] Text builder: title `Waktu Solat — Maghrib in 10 minutes`,
    body `Maghrib at 19:00 · WLY01 Wilayah Persekutuan`
- [x] Wire into `src/app.rs`:
  - [x] Check in `on_tick()` (1 Hz): only when `notify` enabled,
    `notify_available != Some(false)`, `should_notify()` true → persist
    `last_notified`, spawn send; on failure set `notify_available = false`
    + footer status
  - [x] `n` key toggles notifications (persisted); turning on arms the
    current window, turning off suppresses it
- [x] UI (`src/ui.rs`): permanent notification-state indicator right-aligned
  in the Next panel title (`notify: on` accent / `notify: off` dim), transient
  footer confirmation on toggle, notification state in the `?` help overlay
- [x] Unit tests (`should_notify`): boundary at exactly 10:00, inside
  window, dedup for the same occurrence, re-arm for the next one,
  post-prayer skip; plus a capture-double `Notifier` test
- [x] Docs: README (feature, config keys, headless behavior) + mark the
  notification line under *Stretch ideas* as superseded

## Milestone 8 — RTL-friendly Hijri rendering

Goal: render Hijri dates the way they read to humans — Hijri month names —
with an opt-in Arabic-script mode. Terminal bidi is inconsistent, so Arabic
text is always isolated in its own cell/line (never mixed into LTR
sentences), and the safe romanized style is the default.

Decisions: config key `hijri_style` — `malay` (default, JAKIM Takwim
spelling), `numeric` (status quo), `arabic` (Arabic month names +
Arabic-Indic digits); month name shown directly in the table column
(width 7 → 11).

- [x] `src/hijri.rs` (new, pure functions):
  - [x] `parse("1448-06-11") -> Option<(year, month, day)>`; malformed
    strings → None, callers fall back to raw display
  - [x] `MONTHS_MALAY[12]` — Muharram, Safar, Rabiulawal, Rabiulakhir,
    Jamadilawal, Jamadilakhir, Rejab, Syaaban, Ramadan, Syawal, Zulkaedah,
    Zulhijah
  - [x] `MONTHS_ARABIC[12]` — محرم, صفر, ربيع الأول, ربيع الآخر, جمادى
    الأولى, جمادى الآخرة, رجب, شعبان, رمضان, شوال, ذو القعدة, ذو الحجة
  - [x] `to_arabic_digits(s) -> String` (٠-٩)
  - [x] `format_cell(hijri, style)` — `21 Syawal` / `21/06` / `٢١ شوال`
  - [x] `format_full(hijri, style)` — `21 Syawal 1448` / `1448-06-11` /
    `٢١ شوال ١٤٤٨`
- [x] Config: `hijri_style: Option<String>` in `cache.rs`, default `malay`;
  loaded in `App::new()` and `cli::print_now`
- [x] `ui.rs`: Hijri column `Length(7) → Length(11)`, cells via
  `format_cell`; table title appends `· Syawal 1448` when the viewed month
  has data
- [x] Adaptive Hijri column — three tiers by terminal width (measured live):
  full month names when the table fits them (≈ ≥93 cols), numeric short form
  when tight (≈ 73–92 cols, inter-column spacing compacts to 0 below 82),
  and the column dropped entirely below ≈73 cols so prayer times stay
  visible; the effective rendering style follows the tier, not just the
  configured one
- [x] `cli.rs`: `--now` header line uses `format_full`
- [x] Unit tests: parsing (valid/malformed), 12-month mapping, digit
  conversion, all three styles, graceful fallback
- [x] `c` key cycles the Hijri style live in the TUI (Malay → numeric →
  Arabic → Malay), persisted to config on each press, with a footer
  confirmation showing the new style and a sample rendering
- [x] Docs: README config key + Arabic-style terminal bidi caveat; mark the
  RTL line under *Stretch ideas* as superseded

## Milestone 9 — Data modes, offline resilience, active-prayer column

Goal: turn `waktu_solat` from a display app into a scriptable data source,
fix the TUI's offline gap, and finish the last Milestone 2 leftover
(accenting the prayer column you're currently in).

- [x] CLI data modes (`src/cli.rs`, `main.rs`):
  - [x] `--json` — today's row as one compact JSON object: zone, negeri,
    daerah, gregorian date, hijri string, all 8 times as
    `{"epoch": i64, "local": "HH:MM"}` keyed by lowercase name, `current`
    window ("Night" after Isyak), and `next` `{name, epoch, local,
    in_seconds}` (null when beyond the loaded month)
  - [x] `--csv` — header `date,hijri,imsak,fajr,syuruk,dhuha,dhuhr,asar,
    maghrib,isha` + today's row (local `HH:MM` values)
  - [x] `--list-zones` — grep-able `CODE | state | district` lines; no month
    fetch, no geolocation
  - [x] `--json`/`--csv` imply headless mode (like `--now`); compose with
    `--zone`/`--auto-zone`; `--json --csv` together → argparse error, exit 2
  - [x] All output through the existing single-buffered-write path (no
    broken-pipe panics)
  - [x] Update `USAGE` + README with pipe examples (`jq`, status bars)
  - [x] Tests: JSON parses via `serde_json` (shape, times map, next
    entry, Night current, null next when no tomorrow row), CSV field order,
    zone-line format
- [x] TUI stale-cache fallback (`src/app.rs::on_month_loaded`) — offline
  parity with the CLI:
  - [x] On fetch error, `cache.load()` ignoring TTL; stale copy inserted
    into `months` + transient footer status
    ("Offline — showing cached data from Xh ago") — verified live via a
    broken proxy + backdated cache
  - [x] Red error only when there is no cache at all
- [x] Active-prayer column highlight (`src/ui.rs`):
  - [x] `active_column(label, hijri_present)` maps the current window label
    to the table column (KINDS order; shifts with the adaptive Hijri tier);
    `TableState::select_column` + `column_highlight_style` bg `Indexed(236)`
    — fixed palette, distinct from the gold today-row and the blue
    selection; Night → no highlight
  - [x] Column highlight reworked to per-cell styling (user report: the
    column stripe painted empty rows and the selected row spilled across the
    empty right side of wide terminals): the active prayer's cell carries the
    subtle bg only on real data rows, and the row-selection highlight stays
    within the columns
  - [x] Table block spans the full terminal width again (user follow-up):
    the block renders at full width while the table itself is clipped to its
    content width inside it — highlights stop at the last column, the block
    fills the terminal, and narrow terminals still fill completely
  - [x] Highlight walks across columns via the 1s tick; unit-tested mapping
    (including the shift when the Hijri column is hidden)
  - [x] The prayer-times list line in the Next panel highlights the active
    prayer too (`times_line` helper): its name+time render bold accent while
    the other seven stay dim. The night gap (after Isyak, before tomorrow's
    Imsak) belongs to Isyak's window — its entry stays highlighted
    (verified live); Night → header label only
  - [x] Night→Isyak made consistent everywhere via `current_prayer_from()`
    (pure, unit-tested): the header now reads `Now: Isyak` in bold accent
    during the night, the Next panel matches, and the Isyak column carries
    the subtle table highlight until Imsak (`Night` label remains only for
    the no-data state)

## Milestone 10 — Release engineering

Goal: turn a zero-commit local project into an installable, published one.

- [ ] Git history: initial commit of the current state; clean per-feature
  commits from Milestone 9 on; tag `v0.1.0`
- [ ] CI (GitHub Actions `ci.yml`): ubuntu + stable toolchain —
  `cargo build`, `cargo test`, `cargo clippy -- -D warnings`; on push/PR
- [ ] Release workflow (`release.yml` on tag push): build linux-x86_64 gnu
  and musl (static) binaries, attach to a GitHub Release (hand-rolled
  matrix — no `cargo-dist` tooling unless it proves trivial)
- [ ] Package naming: rename the *package* to `waktu-solat` (crates.io
  convention) while keeping the binary name `waktu_solat`; update
  `--version` output and check `cargo install` works
- [ ] README: prebuilt-binary install instructions, `cargo install`, CI
  status + license badges
- [ ] Prepare the api.waktusolat.app "Submit your app" blurb (repo link +
  feature summary) for manual submission

## Stretch ideas (new)

- [ ] Terminal title countdown (OSC): `Maghrib in 04:12` in the window title
  / tmux pane title
- [ ] Headless `daemon` mode + systemd user unit (always-on reminder box)
- [ ] Reminder hooks: `notify_command` config (e.g. play adzan audio),
  per-prayer lead times, D-Bus snooze actions
- [ ] Mouse support (scroll = move selection, click = picker row)
- [ ] Theme config keys for terminals that fight our palette
- [ ] README screenshots / asciinema GIF

## Non-goals

- No prayer-time calculation client-side — always trust the JAKIM data from the API.
- No accounts, no server of our own.
