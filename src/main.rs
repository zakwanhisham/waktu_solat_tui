mod api;
mod app;
mod cache;
mod cli;
mod geo;
mod hijri;
mod notify;
mod ui;

use anyhow::{Context, Result};

const USAGE: &str = "\
waktu_solat — Malaysia prayer times in your terminal

Usage: waktu_solat [ZONE] | [--zone CODE] [--now] [--auto-zone]
       waktu_solat -h | --help
       waktu_solat -V | --version

Arguments:
  ZONE          JAKIM zone code to start with (e.g. WLY01, SGR03).
                Overrides the zone persisted in the config file.

Options:
  --zone CODE   Same as the positional ZONE argument.
  --now         Print today's prayer times and exit (no TUI).
  --auto-zone   Detect the nearest zone and exit. With --now, print today's
                prayer times for the detected zone. IP geolocation is used
                unless coordinates are given.
  --lat DEG     Manual latitude (requires --lng); skips IP geolocation.
  --lng DEG     Manual longitude (requires --lat); skips IP geolocation.
  -h, --help    Show this help.
  -V, --version Show the version.
";

#[tokio::main]
async fn main() -> Result<()> {
    let mut zone_override: Option<String> = None;
    let mut now_mode = false;
    let mut auto_zone = false;
    let mut lat: Option<f64> = None;
    let mut lng: Option<f64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("waktu_solat {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--now" => now_mode = true,
            "--auto-zone" => auto_zone = true,
            "--zone" => {
                zone_override = Some(
                    args.next().context("--zone requires a zone code (e.g. --zone WLY01)")?,
                );
            }
            a if a.starts_with("--zone=") => {
                zone_override = Some(a.trim_start_matches("--zone=").to_string());
            }
            "--lat" => {
                lat = Some(
                    args.next()
                        .context("--lat requires a value (decimal degrees)")?
                        .parse()
                        .context("--lat must be a number (decimal degrees)")?,
                );
            }
            "--lng" => {
                lng = Some(
                    args.next()
                        .context("--lng requires a value (decimal degrees)")?
                        .parse()
                        .context("--lng must be a number (decimal degrees)")?,
                );
            }
            a if a.starts_with('-') => {
                eprintln!("error: unknown option `{a}` (see --help)");
                std::process::exit(2);
            }
            a => zone_override = Some(a.to_string()),
        }
    }

    if lat.xor(lng).is_some() {
        eprintln!("error: --lat and --lng must be given together (see --help)");
        std::process::exit(2);
    }
    // Manual coordinates imply auto mode.
    let auto_zone = auto_zone || lat.is_some();

    if auto_zone {
        // An explicit --zone wins over detection (applies to --now too).
        if let Some(code) = &zone_override
            && now_mode {
                return cli::print_now(Some(code.clone())).await;
            }
        let manual = lat.zip(lng);
        let zone = cli::detect_zone_endpoint(manual).await?;
        if now_mode {
            return cli::print_now(Some(zone.jakim_code.clone())).await;
        }
        println!("Detected zone: {}", zone.display());
        return Ok(());
    }

    // `--now` is headless: no terminal init, works without a TTY.
    if now_mode {
        return cli::print_now(zone_override).await;
    }

    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal, zone_override).await;
    ratatui::restore();
    result
}
