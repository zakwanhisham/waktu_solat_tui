# waktu_solat

A terminal TUI (Rust + ratatui) that displays Malaysia prayer times (*waktu solat*)
from the free [api.waktusolat.app](https://api.waktusolat.app/) API — official
JAKIM (*Jabatan Kemajuan Islam Malaysia*) e-solat data for all 60 zones in
Peninsular Malaysia, Sabah and Sarawak.

```
                         Waktu Solat · WLY01 — Wilayah Persekutuan, Kuala Lumpur, Putrajaya
                         Thu, 19 Nov 2026
┌ Next ────────────────────────────────────────────────────────────────────────────────┐
│Next: Imsak at 05:39, in 04:35:09                                                    │
│Imsak 05:39 · Subuh 05:49 · Syuruk 06:59 · Dhuha 07:24 · Zohor 13:01 · Asar 16:23 …  │
└──────────────────────────────────────────────────────────────────────────────────────┘
╭ November 2026 · WLY01 Wilayah Persekutuan · Kuala Lumpur, Putrajaya (cached 2s ago) ─╮
│  Day   Hijri   Imsak   Subuh   Syuruk  Dhuha   Zohor   Asar   Maghrib  Isyak         │
│   1    21/05   05:25   05:35   06:45   07:12   12:48   16:07  18:48    19:59          │
│  ...
│› 19    09/06   05:26   05:36   06:48   07:15   12:50   16:12  18:48    20:01          │
╰──────────────────────────────────────────────────────────────────────────────────────╯
q quit  s zone  ←→ month  j/k move  t today  r refresh
```

## Features

- **Full month table** — Day, Hijri, Imsak, Subuh, Syuruk, Dhuha, Zohor, Asar,
  Maghrib, Isyak, with today's row highlighted and auto-selected
- **Next-prayer countdown** — live countdown that ticks every second (no API
  calls per tick), wrapping past Isyak to tomorrow's Imsak (next month's data is
  pre-fetched on month boundaries)
- **Searchable zone picker** — filter all 60 JAKIM zones by state or district;
  your selection persists across restarts
- **Month browsing** — navigate past/future months, jump back to today
- **Client-side caching** — month data cached for 24 h and the zone list for
  7 days (as the API asks), with a stale-cache fallback when offline

## Installation

Requires a Rust toolchain (1.85+ / edition 2024).

```bash
git clone <this repo>
cd waktu_solat
cargo run --release
```

## Keybindings

| Key | Action |
|---|---|
| `q` / `Esc` / `Ctrl-C` | Quit |
| `s` | Open zone picker (type to filter, `↑`/`↓` or `Ctrl-n`/`Ctrl-p` to move, `Enter` to select) |
| `?` | Toggle the keybindings help overlay |
| `←` / `→` (or `h` / `l`) | Previous / next month |
| `j` / `k` (or `↑` / `↓`) | Move row selection (counts work: `5j` = 5 rows) |
| `g` / `G` | Jump to the first / last day of the month |
| `Ctrl-d` / `Ctrl-u` | Scroll half a page down / up |
| `t` | Jump back to the current month / today |
| `r` | Force refresh (bypass cache; also retries zone loading) |

## Command line

```text
waktu_solat [ZONE]        start with a specific JAKIM zone code (e.g. WLY01)
waktu_solat --zone CODE   same as above, overrides the persisted config
waktu_solat --help        usage
waktu_solat --version     version
```

## Storage locations

| Path | Contents |
|---|---|
| `~/.cache/waktu_solat/solat-<ZONE>-<YEAR>-<MONTH>.json` | Cached monthly prayer times (24 h TTL) |
| `~/.cache/waktu_solat/zones.json` | Cached zone list (7 day TTL) |
| `~/.config/waktu_solat/config.json` | Persisted zone selection |

(`XDG_CACHE_HOME` / `XDG_CONFIG_HOME` are respected.)

## API

Data comes from [api.waktusolat.app](https://api.waktusolat.app/) — free,
no API key required:

- `GET /zones` — all JAKIM zones (`jakimCode`, `negeri`, `daerah`)
- `GET /v2/solat/{zone}?year=&month=` — monthly prayer times, epoch seconds

The API asks clients to cache responses for at least 24 hours; this app does so
by default. See the [API docs](https://api.waktusolat.app/docs) and
[project page](https://waktusolat.app).

## Development

```bash
cargo build    # debug build
cargo clippy  # lint
cargo run     # run in place
```

Project structure:

- `src/api.rs` — API models and HTTP client
- `src/cache.rs` — disk cache + config persistence
- `src/app.rs` — application state, event loop, key handling, next-prayer logic
- `src/ui.rs` — ratatui rendering

Roadmap lives in [`TODO.md`](TODO.md).

## License

[MIT](LICENSE)
