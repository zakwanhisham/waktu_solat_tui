//! Headless CLI output (`--now`), sharing the data layer with the TUI.

use std::io::Write;

use anyhow::{bail, Context, Result};
use chrono::{Datelike, Local};

use crate::api::{ApiClient, PrayerDay, SolatResponse, Zone};
use crate::cache::{self, Cache, MONTH_TTL, ZONES_FILE, ZONES_TTL};

/// Pick the zone to display: CLI arg > persisted config > first zone.
/// Shared by the TUI (`App::set_zones`) and the CLI.
pub fn resolve_zone(
    zones: &[Zone],
    config_zone: Option<&str>,
    override_zone: Option<&str>,
) -> Result<Zone, String> {
    if let Some(code) = override_zone {
        return zones
            .iter()
            .find(|z| z.jakim_code == code)
            .cloned()
            .ok_or_else(|| format!("Zone `{code}` not found"));
    }
    if let Some(code) = config_zone
        && let Some(zone) = zones.iter().find(|z| z.jakim_code == code)
    {
        return Ok(zone.clone());
    }
    zones.first().cloned().ok_or_else(|| "No zones available".to_string())
}

/// Zone list: fresh cache first, then the network.
pub async fn load_zones(api: &ApiClient, cache: &Cache) -> Result<Vec<Zone>> {
    if let Some(zones) = cache.load_fresh(ZONES_FILE, ZONES_TTL) {
        return Ok(zones);
    }
    let zones = api.zones().await.context("failed to fetch zone list")?;
    cache.store(ZONES_FILE, &zones);
    Ok(zones)
}

/// Detect the current zone (manual coords if given, else IP geolocation).
pub async fn detect_zone_endpoint(manual: Option<(f64, f64)>) -> Result<Zone> {
    let api = ApiClient::new();
    let cache = Cache::new()?;
    let zones = load_zones(&api, &cache).await?;
    crate::geo::detect_zone(&api, &zones, manual).await
}

/// Print today's prayer times for the resolved zone and exit.
pub async fn print_now(zone_override: Option<String>) -> Result<()> {
    let api = ApiClient::new();
    let cache = Cache::new()?;
    let now = Local::now();

    let zones = load_zones(&api, &cache).await?;

    let config_zone = cache.load_config().zone;
    let hijri_style = cache.load_config().hijri_style();
    let zone = resolve_zone(&zones, config_zone.as_deref(), zone_override.as_deref())
        .map_err(anyhow::Error::msg)?;

    // Today's month data: fresh cache first, then the network, then a stale
    // cache as last resort.
    let name = cache::solat_cache_name(&zone.jakim_code, now.year(), now.month());
    let data: SolatResponse = match cache.load_fresh(&name, MONTH_TTL) {
        Some(data) => data,
        None => match api.solat(&zone.jakim_code, now.year(), now.month()).await {
            Ok(data) => {
                cache.store(&name, &data);
                data
            }
            Err(e) => match cache.load(&name) {
                Some((_, stale)) => stale,
                None => bail!("failed to fetch prayer times: {e:#}"),
            },
        },
    };

    let row = data
        .prayers
        .iter()
        .find(|p| p.day == now.day())
        .with_context(|| format!("no prayer data for today ({})", now.format("%d %b %Y")))?;

    let mut out = String::new();
    out.push_str(&format!("Waktu Solat · {}\n", zone.display()));
    out.push_str(&format!("{}\n", now.format("%a, %d %b %Y")));
    out.push_str(&format!("Hijri {}\n\n", crate::hijri::format_full(row.hijri.trim(), hijri_style)));
    for (label, secs) in PrayerDay::KINDS.iter().zip(row.times()) {
        out.push_str(&format!("  {:<8} {}\n", label, fmt_time(secs)));
    }
    out.push('\n');
    out.push_str(&status_line(&data, row, now.timestamp()));
    out.push('\n');

    // Single write; a closed pipe (e.g. `| head`) must not panic.
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(out.as_bytes());
    let _ = lock.flush();
    Ok(())
}

/// `Now: <current window> · Next: <prayer> at HH:MM (in Xh YYm)`.
fn status_line(data: &SolatResponse, row: &PrayerDay, now_ts: i64) -> String {
    let current = PrayerDay::KINDS
        .iter()
        .zip(row.times())
        .rev()
        .find(|(_, t)| *t <= now_ts)
        .map(|(label, _)| label.to_string())
        .unwrap_or_else(|| "Night".to_string());

    let mut next = PrayerDay::KINDS
        .iter()
        .zip(row.times())
        .find(|(_, t)| *t > now_ts)
        .map(|(label, t)| (label.to_string(), t));

    if next.is_none() {
        // After Isyak → the next day's Imsak, if that day is in this month.
        if let Some(tomorrow) = data.prayers.iter().find(|p| p.day == row.day + 1) {
            next = Some(("Imsak".to_string(), tomorrow.imsak));
        }
    }

    match next {
        Some((label, secs)) => {
            let diff = (secs - now_ts).max(0) as u64;
            format!(
                "Now: {} · Next: {} at {} (in {})",
                current,
                label,
                fmt_time(secs),
                fmt_countdown(diff),
            )
        }
        None => format!("Now: {} · Next: Imsak tomorrow", current),
    }
}

fn fmt_time(secs: i64) -> String {
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(t) => t.with_timezone(&Local).format("%H:%M").to_string(),
        None => "—".to_string(),
    }
}

fn fmt_countdown(total: u64) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    if h == 0 && m == 0 {
        "<1m".to_string()
    } else {
        format!("{h}h {m:02}m")
    }
}
