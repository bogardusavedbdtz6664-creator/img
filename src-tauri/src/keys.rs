use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntry {
    pub id: String,
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub exhausted: bool,
    pub exhausted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default = "default_refresh_day")]
    pub refresh_day: u32,
    pub quota_period: Option<String>,
    pub active_key_id: Option<String>,
    #[serde(default)]
    pub keys: Vec<KeyEntry>,
    /// 兼容旧版单 key
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
}

fn default_refresh_day() -> u32 {
    1
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            refresh_day: 1,
            quota_period: None,
            active_key_id: None,
            keys: Vec::new(),
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshInfo {
    pub refreshed: bool,
    pub period: String,
    pub cleared: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicKey {
    pub id: String,
    pub label: String,
    pub masked: String,
    pub exhausted: bool,
    pub exhausted_at: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysView {
    pub ok: bool,
    pub refresh_day: u32,
    pub quota_period: Option<String>,
    pub refresh: RefreshInfo,
    pub active_key_id: Option<String>,
    pub keys: Vec<PublicKey>,
    pub has_key: bool,
}

pub fn mask_key(key: &str) -> String {
    if key.len() > 8 {
        format!("{}…{}", &key[..4], &key[key.len() - 4..])
    } else if key.is_empty() {
        String::new()
    } else {
        "****".into()
    }
}

pub fn current_period_id(refresh_day: u32, now: chrono::DateTime<Local>) -> String {
    let day = refresh_day.clamp(1, 28);
    let mut y = now.year();
    let mut m = now.month();
    if now.day() < day {
        if m == 1 {
            m = 12;
            y -= 1;
        } else {
            m -= 1;
        }
    }
    format!("{y}-{:02}", m)
}

pub fn config_path(base: &Path) -> PathBuf {
    base.join("config.json")
}

pub fn read_config(base: &Path) -> AppConfig {
    let path = config_path(base);
    let mut cfg = if path.exists() {
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => AppConfig::default(),
        }
    } else {
        AppConfig::default()
    };

    if cfg.keys.is_empty() {
        if let Some(old) = cfg.api_key.take() {
            let key = old.trim().to_string();
            if !key.is_empty() {
                let id = Uuid::new_v4().to_string();
                cfg.active_key_id = Some(id.clone());
                cfg.keys.push(KeyEntry {
                    id,
                    key,
                    label: String::new(),
                    exhausted: false,
                    exhausted_at: None,
                });
            }
        }
    }

    cfg.refresh_day = cfg.refresh_day.clamp(1, 28);
    cfg
}

pub fn write_config(base: &Path, cfg: &AppConfig) -> Result<(), String> {
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let path = config_path(base);
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn apply_monthly_refresh(base: &Path, cfg: &mut AppConfig) -> RefreshInfo {
    let period = current_period_id(cfg.refresh_day, Local::now());
    if cfg.quota_period.as_deref() != Some(period.as_str()) {
        let mut cleared = 0u32;
        for k in &mut cfg.keys {
            if k.exhausted {
                k.exhausted = false;
                k.exhausted_at = None;
                cleared += 1;
            }
        }
        cfg.quota_period = Some(period.clone());
        let _ = write_config(base, cfg);
        RefreshInfo {
            refreshed: true,
            period,
            cleared,
        }
    } else {
        RefreshInfo {
            refreshed: false,
            period,
            cleared: 0,
        }
    }
}

pub fn public_keys_view(base: &Path) -> KeysView {
    let mut cfg = read_config(base);
    let refresh = apply_monthly_refresh(base, &mut cfg);
    let keys: Vec<PublicKey> = cfg
        .keys
        .iter()
        .map(|k| PublicKey {
            id: k.id.clone(),
            label: k.label.clone(),
            masked: mask_key(&k.key),
            exhausted: k.exhausted,
            exhausted_at: k.exhausted_at.clone(),
            active: cfg.active_key_id.as_deref() == Some(k.id.as_str()),
        })
        .collect();
    let has_key = keys.iter().any(|k| !k.exhausted);
    KeysView {
        ok: true,
        refresh_day: cfg.refresh_day,
        quota_period: cfg.quota_period.clone(),
        refresh,
        active_key_id: cfg.active_key_id.clone(),
        keys,
        has_key,
    }
}

pub fn add_key(base: &Path, api_key: &str, label: &str) -> Result<KeysView, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("API Key 不能为空".into());
    }
    let mut cfg = read_config(base);
    apply_monthly_refresh(base, &mut cfg);
    if cfg.keys.iter().any(|k| k.key == key) {
        return Err("该 API Key 已存在".into());
    }
    let entry = KeyEntry {
        id: Uuid::new_v4().to_string(),
        key: key.to_string(),
        label: label.trim().to_string(),
        exhausted: false,
        exhausted_at: None,
    };
    let need_active = cfg.active_key_id.as_ref().map_or(true, |id| {
        !cfg.keys
            .iter()
            .any(|k| &k.id == id && !k.exhausted)
    });
    if need_active {
        cfg.active_key_id = Some(entry.id.clone());
    }
    cfg.keys.push(entry);
    write_config(base, &cfg)?;
    Ok(public_keys_view(base))
}

pub fn remove_key(base: &Path, id: &str) -> Result<KeysView, String> {
    let mut cfg = read_config(base);
    apply_monthly_refresh(base, &mut cfg);
    let before = cfg.keys.len();
    cfg.keys.retain(|k| k.id != id);
    if cfg.keys.len() == before {
        return Err("未找到该 Key".into());
    }
    if cfg.active_key_id.as_deref() == Some(id) {
        cfg.active_key_id = cfg
            .keys
            .iter()
            .find(|k| !k.exhausted)
            .or(cfg.keys.first())
            .map(|k| k.id.clone());
    }
    write_config(base, &cfg)?;
    Ok(public_keys_view(base))
}

pub fn set_active_key(base: &Path, id: &str) -> Result<KeysView, String> {
    let mut cfg = read_config(base);
    apply_monthly_refresh(base, &mut cfg);
    if !cfg.keys.iter().any(|k| k.id == id) {
        return Err("未找到该 Key".into());
    }
    cfg.active_key_id = Some(id.to_string());
    write_config(base, &cfg)?;
    Ok(public_keys_view(base))
}

pub fn mark_exhausted(base: &Path, id: &str) -> KeysView {
    let mut cfg = read_config(base);
    if let Some(entry) = cfg.keys.iter_mut().find(|k| k.id == id) {
        entry.exhausted = true;
        entry.exhausted_at = Some(Local::now().to_rfc3339());
    }
    if cfg.active_key_id.as_deref() == Some(id) {
        if let Some(next) = cfg.keys.iter().find(|k| !k.exhausted && k.id != id) {
            cfg.active_key_id = Some(next.id.clone());
        }
    }
    let _ = write_config(base, &cfg);
    public_keys_view(base)
}

pub fn pick_available_key(base: &Path, skip: &HashSet<String>) -> Option<KeyEntry> {
    let mut cfg = read_config(base);
    apply_monthly_refresh(base, &mut cfg);
    let available: Vec<_> = cfg
        .keys
        .iter()
        .filter(|k| !k.exhausted && !skip.contains(&k.id))
        .cloned()
        .collect();
    if available.is_empty() {
        return None;
    }
    if let Some(active_id) = &cfg.active_key_id {
        if let Some(found) = available.iter().find(|k| &k.id == active_id) {
            return Some(found.clone());
        }
    }
    available.into_iter().next()
}
