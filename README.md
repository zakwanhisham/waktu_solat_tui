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
waktu_solat --now         print today's prayer times and exit (no TUI)
waktu_solat --auto-zone   detect the nearest zone and exit (works with --now)
waktu_solat --lat --lng   manual coordinates; skips IP geolocation
waktu_solat --help        usage
waktu_solat --version     version
```

### Auto-detection

In the TUI, press `a` (or pick the pinned **Use my location** row in the zone
picker, `s`) to resolve your nearest JAKIM zone. Detection is **opt-in only** —
it never runs at startup. What it does:

1. Looks up your approximate location via IP geolocation (`ipwho.is`) — or
   uses the coordinates you passed with `--lat`/`--lng`, if any
2. Asks the waktusolat API to resolve those coordinates to a zone
   (`GET /zones/{lat}/{lng}`, server-side polygon matching)
3. Falls back to a bundled nearest-centroid table if the API is unavailable;
   if you appear to be more than 250 km from any zone (e.g. a VPN exit
   node), it refuses to guess and asks you to pick a zone manually

`--now --auto-zone` prints today's prayer times for wherever you are.

`--now` output (headless, script-friendly — works through pipes):

```text
$ waktu_solat --now --zone WLY01
Waktu Solat · WLY01 — Wilayah Persekutuan, Kuala Lumpur, Putrajaya
Sat, 21 Nov 2026
Hijri 1448-06-11

  Imsak    05:40
  Subuh    05:50
  Syuruk   06:58
  Dhuha    07:23
  Zohor    13:02
  Asar     16:24
  Maghrib  19:00
  Isyak    20:13

Now: Night · Next: Imsak at 05:40 (in 4h 12m)
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
- `src/cli.rs` — headless `--now` output, shared zone resolver
- `src/geo.rs` — location detection, coordinate→zone matching, centroid fallback
- `src/app.rs` — application state, event loop, key handling, next-prayer logic
- `src/ui.rs` — ratatui rendering

Roadmap lives in [`TODO.md`](TODO.md).

## License

[MIT](LICENSE)
