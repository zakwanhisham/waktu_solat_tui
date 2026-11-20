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

/// Output format for `--now`-style headless output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
    Csv,
}

/// Current prayer window + upcoming prayer, shared by all output formats.
struct PrayerStatus {
    /// Active window label, or "Night" (after Isyak, before Imsak).
    current: String,
    /// `(name, epoch)` of the upcoming prayer; `None` when the next
    /// occurrence is beyond the loaded month.
    next: Option<(String, i64)>,
}

fn prayer_status(data: &SolatResponse, row: &PrayerDay, now_ts: i64) -> PrayerStatus {
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
    PrayerStatus { current, next }
}

/// `Now: <current window> · Next: <prayer> at HH:MM (in Xh YYm)`.
fn status_line(status: &PrayerStatus, now_ts: i64) -> String {
    match &status.next {
        Some((label, secs)) => {
            let diff = (secs - now_ts).max(0) as u64;
            format!(
                "Now: {} · Next: {} at {} (in {})",
                status.current,
                label,
                fmt_time(*secs),
                fmt_countdown(diff),
            )
        }
        None => format!("Now: {} · Next: Imsak tomorrow", status.current),
    }
}

/// Print today's prayer times (or JSON/CSV) for the resolved zone and exit.
pub async fn print_now(zone_override: Option<String>, format: OutputFormat) -> Result<()> {
    let api = ApiClient::new();
    let cache = Cache::new()?;
    let now = Local::now();

    let zones = load_zones(&api, &cache).await?;

    let config = cache.load_config();
    let zone = resolve_zone(&zones, config.zone.as_deref(), zone_override.as_deref())
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
        .with_context(|| format!("no prayer data for today ({})", now.format("%d %b %Y")))?
        .to_owned();

    let out = match format {
        OutputFormat::Human => human_now(&data, &row, &zone, now.timestamp(), config.hijri_style()),
        OutputFormat::Json => now_json(&zone, &data, &row, now.timestamp()),
        OutputFormat::Csv => now_csv(&row, now.timestamp()),
    };

    // Single write; a closed pipe (e.g. `| head`) must not panic.
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(out.as_bytes());
    let _ = lock.flush();
    Ok(())
}

fn human_now(
    data: &SolatResponse,
    row: &PrayerDay,
    zone: &Zone,
    now_ts: i64,
    hijri_style: crate::hijri::HijriStyle,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Waktu Solat · {}\n", zone.display()));
    out.push_str(&format!(
        "{}\n",
        chrono::DateTime::from_timestamp(now_ts, 0)
            .map(|t| t.with_timezone(&Local).format("%a, %d %b %Y").to_string())
            .unwrap_or_default()
    ));
    out.push_str(&format!("Hijri {}\n\n", crate::hijri::format_full(row.hijri.trim(), hijri_style)));
    for (label, secs) in PrayerDay::KINDS.iter().zip(row.times()) {
        out.push_str(&format!("  {:<8} {}\n", label, fmt_time(secs)));
    }
    out.push('\n');
    let status = prayer_status(data, row, now_ts);
    out.push_str(&status_line(&status, now_ts));
    out.push('\n');
    out
}

/// `[derive(Serialize)]` models for `--json`.
mod json_model {
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct TimeEntry {
        pub epoch: i64,
        pub local: String,
    }

    #[derive(Serialize)]
    pub struct NextEntry {
        pub name: String,
        pub epoch: i64,
        pub local: String,
        pub in_seconds: i64,
    }

    #[derive(Serialize)]
    pub struct Now {
        pub zone: String,
        pub negeri: String,
        pub daerah: String,
        pub date: String,
        pub hijri: String,
        pub current: String,
        pub times: std::collections::BTreeMap<String, TimeEntry>,
        pub next: Option<NextEntry>,
    }
}

fn now_json(zone: &Zone, data: &SolatResponse, row: &PrayerDay, now_ts: i64) -> String {
    use json_model::{NextEntry, Now, TimeEntry};
    let date = chrono::DateTime::from_timestamp(now_ts, 0)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let times: std::collections::BTreeMap<String, TimeEntry> = PrayerDay::KINDS
        .iter()
        .zip(row.times())
        .map(|(label, secs)| {
            (
                label.to_lowercase(),
                TimeEntry { epoch: secs, local: fmt_time(secs) },
            )
        })
        .collect();
    let status = prayer_status(data, row, now_ts);
    let next = status.next.map(|(name, epoch)| NextEntry {
        local: fmt_time(epoch),
        in_seconds: (epoch - now_ts).max(0),
        name,
        epoch,
    });
    let now = Now {
        zone: zone.jakim_code.clone(),
        negeri: zone.negeri.clone(),
        daerah: zone.daerah.clone(),
        date,
        hijri: row.hijri.trim().to_string(),
        current: status.current,
        times,
        next,
    };
    serde_json::to_string(&now).expect("NowJson serialization cannot fail") + "\n"
}

fn now_csv(row: &PrayerDay, now_ts: i64) -> String {
    let date = chrono::DateTime::from_timestamp(now_ts, 0)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let mut out = String::from("date,hijri,imsak,fajr,syuruk,dhuha,dhuhr,asar,maghrib,isha\n");
    out.push_str(&date);
    out.push(',');
    out.push_str(row.hijri.trim());
    for secs in row.times() {
        out.push(',');
        out.push_str(&fmt_time(secs));
    }
    out.push('\n');
    out
}

/// `--list-zones`: grep-able `CODE | state | district` lines.
pub async fn list_zones() -> Result<()> {
    let api = ApiClient::new();
    let cache = Cache::new()?;
    let zones = load_zones(&api, &cache).await?;
    let mut out = String::new();
    for z in &zones {
        out.push_str(&zone_line(z));
    }
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(out.as_bytes());
    let _ = lock.flush();
    Ok(())
}

fn zone_line(z: &Zone) -> String {
    format!("{} | {} | {}\n", z.jakim_code, z.negeri, z.daerah)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PrayerDay;

    fn sample_row() -> PrayerDay {
        // 2026-11-21: Imsak 05:40, … Isyak 20:13 local (MYT).
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

    fn sample_zone() -> Zone {
        Zone {
            jakim_code: "WLY01".into(),
            negeri: "Wilayah Persekutuan".into(),
            daerah: "Kuala Lumpur, Putrajaya".into(),
        }
    }

    #[test]
    fn json_output_parses_and_has_expected_shape() {
        let json = now_json(&sample_zone(), &dummy_data(), &sample_row(), 1795197083);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["zone"], "WLY01");
        assert_eq!(v["hijri"], "1448-06-11");
        assert_eq!(v["times"]["maghrib"]["local"], "19:00");
        assert_eq!(v["times"]["maghrib"]["epoch"], 1795258800);
        let next = v["next"].as_object().expect("next present");
        assert_eq!(next["name"], "Imsak");
        assert!(next["in_seconds"].as_i64().unwrap() > 0);
    }

    #[test]
    fn json_current_is_night_after_isha() {
        // Deep night timestamp (after Isyak, before Imsak).
        let json = now_json(&sample_zone(), &dummy_data(), &sample_row(), 1795197083);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["current"], "Night");
    }

    #[test]
    fn json_next_null_when_no_tomorrow_row() {
        let json = now_json(&sample_zone(), &dummy_data(), &sample_row(), 1795263181);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["next"].is_null());
        assert_eq!(v["current"], "Isyak");
    }

    #[test]
    fn csv_field_order() {
        let csv = now_csv(&sample_row(), 1795197083);
        let mut lines = csv.lines();
        assert_eq!(
            lines.next(),
            Some("date,hijri,imsak,fajr,syuruk,dhuha,dhuhr,asar,maghrib,isha")
        );
        assert_eq!(
            lines.next(),
            Some("2026-11-21,1448-06-11,05:40,05:50,06:59,07:24,13:02,16:24,19:00,20:13")
        );
    }

    #[test]
    fn zone_line_format() {
        assert_eq!(
            zone_line(&sample_zone()),
            "WLY01 | Wilayah Persekutuan | Kuala Lumpur, Putrajaya\n"
        );
    }

    /// Minimal SolatResponse for status lookups.
    fn dummy_data() -> SolatResponse {
        SolatResponse {
            zone: "WLY01".into(),
            year: 2026,
            month_number: 11,
            month: "NOV".into(),
            last_updated: None,
            prayers: vec![sample_row()],
        }
    }
}
