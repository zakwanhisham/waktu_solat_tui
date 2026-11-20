use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Wrapper stored in every cache file so we can track freshness.
#[derive(Serialize, Deserialize)]
struct Entry<T> {
    fetched_at: u64,
    payload: T,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub zone: Option<String>,
    /// Desktop reminder before the next prayer (Milestone 7). All fields are
    /// optional so older config files keep working.
    pub notify: Option<bool>,
    pub notify_minutes: Option<u32>,
    pub last_notified: Option<i64>,
    pub notify_available: Option<bool>,
}

impl Config {
    /// Reminder enabled? Default: off.
    pub fn notify_enabled(&self) -> bool {
        self.notify.unwrap_or(false)
    }

    /// Lead time in minutes. Default: 10.
    pub fn notify_lead_minutes(&self) -> u32 {
        self.notify_minutes.unwrap_or(crate::notify::DEFAULT_LEAD_MINUTES)
    }
}

#[derive(Clone)]
pub struct Cache {
    cache_dir: PathBuf,
    config_dir: PathBuf,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn xdg(env_var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(env_var)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
        .unwrap_or_else(|| PathBuf::from("."))
}

impl Cache {
    pub fn new() -> Result<Self> {
        let cache_dir = xdg("XDG_CACHE_HOME", ".cache").join("waktu_solat");
        let config_dir = xdg("XDG_CONFIG_HOME", ".config").join("waktu_solat");
        fs::create_dir_all(&cache_dir)?;
        fs::create_dir_all(&config_dir)?;
        Ok(Self { cache_dir, config_dir })
    }

    /// Load `name` from the cache dir, returning `(fetched_at_epoch, payload)`.
    pub fn load<T: DeserializeOwned>(&self, name: &str) -> Option<(u64, T)> {
        let bytes = fs::read(self.cache_dir.join(name)).ok()?;
        let entry: Entry<T> = serde_json::from_slice(&bytes).ok()?;
        Some((entry.fetched_at, entry.payload))
    }

    /// Like `load`, but only returns the payload when it is younger than `ttl`.
    pub fn load_fresh<T: DeserializeOwned>(&self, name: &str, ttl: u64) -> Option<T> {
        let (fetched_at, payload) = self.load(name)?;
        (now_secs() < fetched_at + ttl).then_some(payload)
    }

    pub fn store<T: Serialize>(&self, name: &str, payload: &T) {
        let entry = Entry { fetched_at: now_secs(), payload };
        if let Ok(bytes) = serde_json::to_vec(&entry) {
            let tmp = self.cache_dir.join(format!("{name}.tmp"));
            let target = self.cache_dir.join(name);
            if fs::write(&tmp, &bytes).is_ok() {
                let _ = fs::rename(&tmp, &target);
            }
        }
    }

    pub fn load_config(&self) -> Config {
        fs::read(self.config_dir.join("config.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn store_config(&self, config: &Config) {
        if let Ok(bytes) = serde_json::to_vec(config) {
            let _ = fs::write(self.config_dir.join("config.json"), &bytes);
        }
    }
}

pub fn solat_cache_name(code: &str, year: i32, month: u32) -> String {
    format!("solat-{code}-{year}-{month:02}.json")
}

pub const ZONES_FILE: &str = "zones.json";
pub const MONTH_TTL: u64 = 24 * 60 * 60; // the API asks for >= 24h caching
pub const ZONES_TTL: u64 = 7 * 24 * 60 * 60;
