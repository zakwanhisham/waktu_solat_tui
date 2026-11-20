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

## Stretch ideas

- [ ] Auto-detect the nearest zone via GPS coords (API supports geographic resolution)
- [ ] Desktop notification a few minutes before the next prayer
- [ ] RTL-friendly Hijri month rendering

## Non-goals

- No prayer-time calculation client-side — always trust the JAKIM data from the API.
- No accounts, no server of our own.
