//! Location detection and zone matching (Milestone 6).
//!
//! Two sources of truth for "coordinates → JAKIM zone":
//! 1. the server's polygon endpoint `GET /zones/{lat}/{lng}` (accurate), and
//! 2. a bundled centroid table (offline fallback, approximate).
//!
//! Coordinates come either from manual CLI flags or from IP geolocation
//! (`ipwho.is`, free, no key). Detection is opt-in only — never run implicitly.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::api::{ApiClient, GeoZone, Zone};

/// Approximate district centroids for every JAKIM zone, derived from the
/// districts listed per zone in `GET /zones`. Used only as a fallback when
/// the server-side geo endpoint is unavailable.
pub const ZONE_CENTROIDS: [(&str, f64, f64); 60] = [
    ("JHR01", 2.50, 104.10),  // Pulau Aur, Pulau Pemanggil
    ("JHR02", 1.83, 103.77),  // JB, Kota Tinggi, Mersing, Kulai
    ("JHR03", 1.92, 103.40),  // Kluang, Pontian
    ("JHR04", 2.17, 102.78),  // Batu Pahat, Muar, Segamat, Gemas Johor, Tangkak
    ("KDH01", 6.19, 100.40),  // Kota Setar, Kubang Pasu, Pokok Sena
    ("KDH02", 5.76, 100.50),  // Kuala Muda, Yan, Pendang
    ("KDH03", 6.01, 100.68),  // Padang Terap, Sik
    ("KDH04", 5.70, 100.93),  // Baling
    ("KDH05", 5.30, 100.60),  // Bandar Baharu, Kulim
    ("KDH06", 6.35, 99.78),   // Langkawi
    ("KDH07", 5.79, 100.66),  // Puncak Gunung Jerai
    ("KTN01", 5.85, 102.20),  // Bachok…Kota Bharu…Kuala Krai
    ("KTN02", 5.30, 101.92),  // Gua Musang, Jeli, Lojing
    ("MLK01", 2.19, 102.25),  // Seluruh Melaka
    ("NGS01", 2.62, 102.32),  // Tampin, Jempol
    ("NGS02", 2.74, 102.15),  // Jelebu, Kuala Pilah, Rembau
    ("NGS03", 2.63, 101.87),  // Port Dickson, Seremban
    ("PHG01", 2.82, 104.16),  // Pulau Tioman
    ("PHG02", 3.48, 103.28),  // Kuantan, Pekan, Muadzam Shah
    ("PHG03", 3.53, 102.52),  // Jerantut, Temerloh, Maran, Bera, Jengka
    ("PHG04", 3.81, 101.95),  // Bentong, Lipis, Raub
    ("PHG05", 3.42, 101.84),  // Genting Sempah, Janda Baik, Bukit Tinggi
    ("PHG06", 3.87, 101.64),  // Cameron/Genting Highlands, Bukit Fraser
    ("PHG07", 2.66, 103.38),  // Zon Khas Rompin (Rompin, Endau, Pontian)
    ("PRK01", 3.91, 101.44),  // Tapah, Slim River, Tanjung Malim
    ("PRK02", 4.56, 101.06),  // Kuala Kangsar, Ipoh, Batu Gajah, Kampar
    ("PRK03", 5.20, 101.11),  // Lenggong, Pengkalan Hulu, Grik
    ("PRK04", 5.70, 101.55),  // Temengor, Belum
    ("PRK05", 4.20, 100.88),  // Teluk Intan, Lumut, Sitiawan, Pangkor…
    ("PRK06", 5.01, 100.63),  // Selama, Taiping, Bagan Serai, Parit Buntar
    ("PRK07", 4.87, 100.79),  // Bukit Larut
    ("PLS01", 6.44, 100.20),  // Seluruh Perlis
    ("PNG01", 5.41, 100.33),  // Seluruh Pulau Pinang
    ("SBH01", 5.70, 118.05),  // Sandakan (Timur), Sukau
    ("SBH02", 5.40, 117.35),  // Beluran, Telupid, Sandakan (Barat)
    ("SBH03", 4.73, 118.40),  // Lahad Datu, Kunak, Semporna
    ("SBH04", 4.34, 117.75),  // Tawau (Barat), Kalabakan
    ("SBH05", 6.71, 116.92),  // Kudat, Kota Marudu, Pitas, Banggi
    ("SBH06", 6.08, 116.56),  // Gunung Kinabalu
    ("SBH07", 5.89, 116.23),  // KK, Ranau, Kota Belud, Tuaran, Penampang, Papar
    ("SBH08", 5.59, 116.29),  // Pensiangan, Keningau, Tambunan, Nabawan
    ("SBH09", 5.20, 115.74),  // Beaufort, Sipitang, Tenom, Kuala Penyu
    ("SWK01", 4.80, 115.20),  // Limbang, Lawas, Sundar, Trusan
    ("SWK02", 4.28, 114.15),  // Miri, Niah, Marudi
    ("SWK03", 3.00, 113.25),  // Bintulu, Belaga, Tatau
    ("SWK04", 2.34, 112.25),  // Sibu, Mukah, Kanowit, Kapit
    ("SWK05", 2.24, 111.56),  // Sarikei, Matu, Julau, Daro
    ("SWK06", 1.74, 111.45),  // Sri Aman, Betong, Saratok, Lubok Antu
    ("SWK07", 1.29, 110.60),  // Serian, Simunjan, Samarahan
    ("SWK08", 1.55, 110.16),  // Kuching, Bau, Lundu, Sematan
    ("SWK09", 1.70, 111.50),  // Zon Khas (Kampung Patarikan)
    ("SGR01", 3.15, 101.65),  // Gombak, Petaling, Sepang, Hulu Langat, Shah Alam
    ("SGR02", 3.51, 101.12),  // Kuala Selangor, Sabak Bernam
    ("SGR03", 2.92, 101.47),  // Klang, Kuala Langat
    ("TRG01", 5.33, 103.13),  // Kuala Terengganu, Marang, Kuala Nerus
    ("TRG02", 5.69, 102.66),  // Besut, Setiu
    ("TRG03", 5.06, 103.03),  // Hulu Terengganu
    ("TRG04", 4.50, 103.43),  // Dungun, Kemaman
    ("WLY01", 3.15, 101.69),  // Kuala Lumpur, Putrajaya
    ("WLY02", 5.28, 115.24),  // Labuan
];

/// Beyond this distance from any centroid we assume the user is not in
/// Malaysia (e.g. a VPN exit node) instead of silently picking a wrong zone.
pub const MAX_DISTANCE_KM: f64 = 250.0;

/// IP geolocation via `ipwho.is` (free, no key, HTTPS).
pub async fn detect_location() -> Result<(f64, f64)> {
    #[derive(Deserialize)]
    struct IpWho {
        success: bool,
        latitude: Option<f64>,
        longitude: Option<f64>,
        message: Option<String>,
    }
    let http = reqwest::Client::builder()
        .user_agent(concat!("waktu_solat-tui/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let resp: IpWho = http
        .get("https://ipwho.is/")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .context("IP geolocation request failed")?
        .error_for_status()?
        .json()
        .await
        .context("failed to decode IP geolocation response")?;
    if !resp.success {
        bail!("IP geolocation failed: {}", resp.message.unwrap_or_else(|| "unknown error".into()));
    }
    match (resp.latitude, resp.longitude) {
        (Some(lat), Some(lng)) => Ok((lat, lng)),
        _ => bail!("IP geolocation returned no coordinates"),
    }
}

/// Great-circle distance between two points, in kilometres.
pub fn haversine_km(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const R: f64 = 6371.0;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let (dlat, dlng) = ((lat2 - lat1).to_radians(), (lng2 - lng1).to_radians());
    let a = (dlat / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlng / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

/// Nearest zone by centroid distance, for offline fallback.
/// Returns `(zone_code, distance_km)`; errors beyond the Malaysia threshold.
pub fn nearest_zone_code(lat: f64, lng: f64) -> Result<(&'static str, f64)> {
    let mut best: Option<(&'static str, f64)> = None;
    for (code, clat, clng) in ZONE_CENTROIDS {
        let d = haversine_km(lat, lng, clat, clng);
        if best.is_none_or(|(_, best_d)| d < best_d) {
            best = Some((code, d));
        }
    }
    let (code, dist) = best.expect("centroid table is not empty");
    if dist > MAX_DISTANCE_KM {
        bail!(
            "Location is {:.0} km from the nearest zone — you may be outside Malaysia",
            dist
        );
    }
    Ok((code, dist))
}

/// Full auto-detection: manual coordinates if given, else IP lookup; the
/// server's polygon endpoint first, the bundled centroid table as fallback.
pub async fn detect_zone(api: &ApiClient, zones: &[Zone], manual: Option<(f64, f64)>) -> Result<Zone> {
    let (lat, lng) = match manual {
        Some(coords) => coords,
        None => detect_location().await.context("could not determine your location")?,
    };
    // Server-side polygon resolution is the accurate path; a 500 here is also
    // how the API says "no zone for these coordinates".
    if let Ok(geo) = api.nearest_zone(lat, lng).await {
        return Ok(zone_from_geo(geo, zones));
    }
    let (code, _) = nearest_zone_code(lat, lng)?;
    zones
        .iter()
        .find(|z| z.jakim_code == code)
        .cloned()
        .with_context(|| format!("matched zone `{code}` but it is not in the zone list"))
}

fn zone_from_geo(geo: GeoZone, zones: &[Zone]) -> Zone {
    zones
        .iter()
        .find(|z| z.jakim_code == geo.zone)
        .cloned()
        .unwrap_or(Zone {
            jakim_code: geo.zone.clone(),
            negeri: geo.state.clone(),
            daerah: geo.district.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kl_resolves_to_wly01() {
        assert_eq!(nearest_zone_code(3.1390, 101.6869).unwrap().0, "WLY01");
    }

    #[test]
    fn klang_resolves_to_sgr03() {
        assert_eq!(nearest_zone_code(3.0333, 101.4430).unwrap().0, "SGR03");
    }

    #[test]
    fn george_town_resolves_to_png01() {
        assert_eq!(nearest_zone_code(5.4141, 100.3288).unwrap().0, "PNG01");
    }

    #[test]
    fn labuan_resolves_to_wly02() {
        assert_eq!(nearest_zone_code(5.2833, 115.2333).unwrap().0, "WLY02");
    }

    #[test]
    fn london_is_outside_malaysia() {
        assert!(nearest_zone_code(51.5072, -0.1276).is_err());
    }

    #[test]
    fn haversine_sanity() {
        // KL to Klang city ≈ 29 km.
        let d = haversine_km(3.1390, 101.6869, 3.0333, 101.4430);
        assert!((20.0..40.0).contains(&d), "got {d}");
        // Zero distance for identical points.
        assert_eq!(haversine_km(3.0, 101.0, 3.0, 101.0), 0.0);
    }
}
