mod api;
mod app;
mod cache;
mod cli;
mod ui;

use anyhow::{Context, Result};

const USAGE: &str = "\
waktu_solat — Malaysia prayer times in your terminal

Usage: waktu_solat [ZONE] | [--zone CODE] [--now]
       waktu_solat -h | --help
       waktu_solat -V | --version

Arguments:
  ZONE          JAKIM zone code to start with (e.g. WLY01, SGR03).
                Overrides the zone persisted in the config file.

Options:
  --zone CODE   Same as the positional ZONE argument.
  --now         Print today's prayer times and exit (no TUI).
  -h, --help    Show this help.
  -V, --version Show the version.
";

#[tokio::main]
async fn main() -> Result<()> {
    let mut zone_override: Option<String> = None;
    let mut now_mode = false;
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
            "--zone" => {
                zone_override = Some(
                    args.next().context("--zone requires a zone code (e.g. --zone WLY01)")?,
                );
            }
            a if a.starts_with("--zone=") => {
                zone_override = Some(a.trim_start_matches("--zone=").to_string());
            }
            a if a.starts_with('-') => {
                eprintln!("error: unknown option `{a}` (see --help)");
                std::process::exit(2);
            }
            a => zone_override = Some(a.to_string()),
        }
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
