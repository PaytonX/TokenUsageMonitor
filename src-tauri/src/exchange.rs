//! 汇率服务（B.6）：USD→各币种汇率的抓取、缓存与覆盖合并。
//!
//! 来源优先级（高 → 低）：
//! 1. `settings.rate_overrides` 用户手动覆盖 —— 永远生效，不被拉取值冲掉；
//! 2. SQLite `exchange_rates` 缓存表 —— 最近一次成功网络拉取的值；
//! 3. 编译期内置兜底值 —— 离线首启时的最后防线（与前端 currency.ts 一致）。
//!
//! 网络失败绝不致命（风险约束 E）：失败原因写进快照 `warning`，其余来源照常
//! 合并，本机监控不受影响。缓存超过 [`RATE_MAX_AGE_HOURS`] 才重新打网络。

use crate::settings::Settings;
use crate::storage::Storage;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;

/// 缓存超过该时长（小时）才视为陈旧、重新拉取。
pub const RATE_MAX_AGE_HOURS: i64 = 24;

/// 支持的币种全集（与前端 `Currency` 联合类型保持一致）。
pub const SUPPORTED_CODES: &[&str] = &["USD", "CNY", "TWD", "HKD", "JPY", "EUR", "GBP"];

/// 编译期内置兜底汇率（每 1 USD 兑该币种），与前端 currency.ts 的 USD_RATES 一致。
pub const FALLBACK_USD_RATES: &[(&str, f64)] = &[
    ("USD", 1.0),
    ("CNY", 7.13),
    ("TWD", 31.9),
    ("HKD", 7.79),
    ("JPY", 142.3),
    ("EUR", 0.92),
    ("GBP", 0.79),
];

/// 免费公开源（无需 key）。
const RATE_API_URL: &str = "https://open.er-api.com/v6/latest/USD";

/// 单行汇率快照。`source` ∈ "override" | "live" | "default"。
#[derive(Debug, Clone, Serialize)]
pub struct RateRow {
    pub code: String,
    pub rate: f64,
    /// 该值的抓取时间（ISO8601）；覆盖行为空串（前端显示"手动"）。
    pub updated_at: String,
    pub source: String,
}

/// 提供给前端的完整汇率快照。
#[derive(Debug, Clone, Serialize)]
pub struct RatesSnapshot {
    pub rates: Vec<RateRow>,
    /// 最近一次成功网络拉取的时间（ISO8601）；从未拉取过则为空串。
    pub fetched_at: String,
    /// 网络失败等说明；成功时为 None。
    pub warning: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenErApiResponse {
    rates: HashMap<String, f64>,
}

/// 从免费公开源抓取最新汇率。仅保留 [`SUPPORTED_CODES`] 中的币种，USD 恒为 1。
/// 源缺失的币种直接跳过，由 [`effective_rows`] 回落到兜底值。
pub async fn fetch_latest_rates(http: &reqwest::Client) -> Result<Vec<(String, f64)>> {
    let resp: OpenErApiResponse = http
        .get(RATE_API_URL)
        // 10s 超时：离线/黑洞网络下避免挂起设置保存与手动刷新
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("requesting exchange rates")?
        .json()
        .await
        .context("decoding exchange rates")?;
    let mut out = Vec::with_capacity(SUPPORTED_CODES.len());
    for &code in SUPPORTED_CODES {
        let rate = if code == "USD" {
            1.0
        } else {
            match resp.rates.get(code) {
                Some(&r) if r.is_finite() && r > 0.0 => r,
                _ => continue,
            }
        };
        out.push((code.to_string(), rate));
    }
    Ok(out)
}

/// 缓存时间戳是否已陈旧（超过 [`RATE_MAX_AGE_HOURS`]）。解析失败视为陈旧。
pub fn is_stale(updated_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(updated_at) {
        Ok(t) => {
            chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc)).num_hours()
                >= RATE_MAX_AGE_HOURS
        }
        Err(_) => true,
    }
}

/// 合并三层来源生成快照行（纯函数，便于单测）。`cached` 形如
/// `storage::list_exchange_rates()` 的返回 `(code, rate, updated_at)`。
pub fn effective_rows(
    cached: &[(String, f64, String)],
    overrides: &HashMap<String, f64>,
) -> Vec<RateRow> {
    let mut rows = Vec::with_capacity(SUPPORTED_CODES.len());
    for &code in SUPPORTED_CODES {
        // 1) 用户覆盖优先；非法值（<=0/NaN）忽略，走正常链路。
        if let Some(&r) = overrides.get(code) {
            if r.is_finite() && r > 0.0 {
                rows.push(RateRow {
                    code: code.to_string(),
                    rate: r,
                    updated_at: String::new(),
                    source: "override".to_string(),
                });
                continue;
            }
        }
        // 2) 最近一次网络拉取的缓存。
        if let Some((_, rate, updated_at)) = cached.iter().find(|(c, _, _)| c == code) {
            rows.push(RateRow {
                code: code.to_string(),
                rate: *rate,
                updated_at: updated_at.clone(),
                source: "live".to_string(),
            });
            continue;
        }
        // 3) 编译期兜底。USD 恒为 1，永远走这里（不在缓存表里）。
        let fallback = FALLBACK_USD_RATES
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, r)| *r)
            .unwrap_or(1.0);
        rows.push(RateRow {
            code: code.to_string(),
            rate: fallback,
            updated_at: String::new(),
            source: "default".to_string(),
        });
    }
    rows
}

/// 按需刷新缓存并返回合并快照。`force` 跳过陈旧判断直接拉取。
/// 网络失败不报错：原因写进快照 `warning`，其余来源照常合并。
pub async fn refresh_rates(
    storage: &Storage,
    settings: &Settings,
    http: &reqwest::Client,
    force: bool,
) -> RatesSnapshot {
    let mut warning: Option<String> = None;
    let cached = storage.list_exchange_rates().unwrap_or_default();
    let need_fetch =
        force || cached.is_empty() || cached.iter().any(|(_, _, u)| is_stale(u));
    if need_fetch {
        match fetch_latest_rates(http).await {
            Ok(rates) => {
                if let Err(e) = storage.save_exchange_rates(&rates) {
                    warning = Some(format!("汇率缓存写入失败：{e}"));
                }
            }
            Err(e) => {
                warning = Some(format!("汇率拉取失败，已回退缓存/默认值：{e}"));
            }
        }
    }
    let cached = storage.list_exchange_rates().unwrap_or_default();
    // RFC3339 UTC 时间戳字典序 == 时间序，max 即最近一次拉取。
    let fetched_at = cached
        .iter()
        .map(|(_, _, u)| u.clone())
        .max()
        .unwrap_or_default();
    let rates = effective_rows(&cached, &settings.rate_overrides);
    RatesSnapshot {
        rates,
        fetched_at,
        warning,
    }
}

#[cfg(test)]
mod exchange_tests {
    use super::*;

    fn temp_storage() -> Storage {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("pulse_fx_unit_test_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Storage::open(&path).unwrap()
    }

    fn cached(pairs: &[(&str, f64, &str)]) -> Vec<(String, f64, String)> {
        pairs
            .iter()
            .map(|(c, r, u)| (c.to_string(), *r, u.to_string()))
            .collect()
    }

    #[test]
    fn is_stale_respects_window_and_garbage() {
        let fresh = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let old = (chrono::Utc::now() - chrono::Duration::hours(RATE_MAX_AGE_HOURS + 1))
            .to_rfc3339();
        assert!(!is_stale(&fresh));
        assert!(is_stale(&old));
        assert!(is_stale("not-a-timestamp"));
        assert!(is_stale(""));
    }

    #[test]
    fn effective_rows_precedence_override_live_default() {
        let rows = effective_rows(
            &cached(&[("CNY", 7.20, "2026-01-01T00:00:00+00:00")]),
            &HashMap::from([("HKD".to_string(), 7.80)]),
        );
        let get = |code: &str| rows.iter().find(|r| r.code == code).unwrap();
        // 覆盖 > 缓存。
        let hkd = get("HKD");
        assert_eq!(hkd.source, "override");
        assert!((hkd.rate - 7.80).abs() < 1e-9);
        let cny = get("CNY");
        assert_eq!(cny.source, "live");
        assert!((cny.rate - 7.20).abs() < 1e-9);
        assert_eq!(cny.updated_at, "2026-01-01T00:00:00+00:00");
        // 无缓存无覆盖 → 兜底。
        let eur = get("EUR");
        assert_eq!(eur.source, "default");
        assert!((eur.rate - 0.92).abs() < 1e-9);
        // USD 恒为 1。
        let usd = get("USD");
        assert!((usd.rate - 1.0).abs() < 1e-9);
        // 行数覆盖全部支持币种。
        assert_eq!(rows.len(), SUPPORTED_CODES.len());
    }

    #[test]
    fn effective_rows_ignores_invalid_overrides() {
        let rows = effective_rows(
            &cached(&[]),
            &HashMap::from([
                ("CNY".to_string(), 0.0),
                ("JPY".to_string(), f64::NAN),
            ]),
        );
        let get = |code: &str| rows.iter().find(|r| r.code == code).unwrap();
        assert_eq!(get("CNY").source, "default");
        assert_eq!(get("JPY").source, "default");
    }

    #[test]
    fn refresh_rates_uses_cache_when_fresh_and_falls_back_offline() {
        let s = temp_storage();
        // 写入一条"新鲜"缓存（刚拉取过）。
        s.save_exchange_rates(&[("CNY".to_string(), 7.15)]).unwrap();
        let http = reqwest::Client::new();
        let settings = Settings::default();
        // force=false 且缓存新鲜：不应打网络（离线也不报错），返回缓存值。
        let snap = futures_block_on(refresh_rates(&s, &settings, &http, false));
        assert!(snap.warning.is_none());
        let cny = snap.rates.iter().find(|r| r.code == "CNY").unwrap();
        assert!((cny.rate - 7.15).abs() < 1e-9);
        assert_eq!(cny.source, "live");
        assert!(!snap.fetched_at.is_empty());
    }

    #[test]
    fn refresh_rates_offline_force_returns_warning_with_fallback() {
        let s = temp_storage();
        let http = reqwest::Client::new();
        let settings = Settings::default();
        // 空缓存 + force：若环境可联网则拉取成功；否则回退默认值并带 warning。
        // 两种结果都必须返回完整快照（币种全集），且不 panic。
        let snap = futures_block_on(refresh_rates(&s, &settings, &http, true));
        assert_eq!(snap.rates.len(), SUPPORTED_CODES.len());
        if snap.warning.is_some() {
            let cny = snap.rates.iter().find(|r| r.code == "CNY").unwrap();
            assert_eq!(cny.source, "default");
        }
    }

    /// 极简 block_on：current-thread runtime，让 reqwest 的 IO 驱动可用。
    fn futures_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("building test tokio runtime")
            .block_on(fut)
    }
}
