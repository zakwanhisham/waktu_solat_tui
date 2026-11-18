use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://api.waktusolat.app";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Zone {
    pub jakim_code: String,
    pub negeri: String,
    pub daerah: String,
}

impl Zone {
    /// `WLY01 — Kuala Lumpur`
    pub fn display(&self) -> String {
        format!("{} — {}, {}", self.jakim_code, self.negeri, self.daerah)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SolatResponse {
    pub zone: String,
    pub year: i32,
    #[serde(rename = "month_number")]
    pub month_number: u32,
    pub month: String,
    #[serde(rename = "last_updated")]
    pub last_updated: Option<i64>,
    pub prayers: Vec<PrayerDay>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PrayerDay {
    pub day: u32,
    pub hijri: String,
    pub imsak: i64,
    pub fajr: i64,
    pub syuruk: i64,
    pub dhuha: i64,
    pub dhuhr: i64,
    pub asr: i64,
    pub maghrib: i64,
    pub isha: i64,
}

impl PrayerDay {
    /// Prayer names in the order they occur during a day.
    pub const KINDS: [&str; 8] = ["Imsak", "Subuh", "Syuruk", "Dhuha", "Zohor", "Asar", "Maghrib", "Isyak"];

    pub fn times(&self) -> [i64; 8] {
        [self.imsak, self.fajr, self.syuruk, self.dhuha, self.dhuhr, self.asr, self.maghrib, self.isha]
    }
}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("waktu_solat-tui/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build http client");
        Self { http }
    }

    pub async fn zones(&self) -> Result<Vec<Zone>> {
        self.http
            .get(format!("{BASE_URL}/zones"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("failed to decode zones response")
    }

    pub async fn solat(&self, zone: &str, year: i32, month: u32) -> Result<SolatResponse> {
        self.http
            .get(format!("{BASE_URL}/v2/solat/{zone}?year={year}&month={month}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context(format!("failed to decode solat response for {zone} {year}-{month:02}"))
    }
}
