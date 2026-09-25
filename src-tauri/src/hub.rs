//! 多端同步 hub（B8，本地优先）。
//!
//! 一个轻量的 HTTP 服务，作为多台 TokenUsageMonitor 实例共享的设备用量汇聚点：
//! - `POST /ingest`：设备上报本机用量摘要（`HubDevice`）→ 持久化到 SQLite。
//! - `GET  /devices`：返回所有设备的用量摘要 JSON 数组。
//!
//! 通过标准库 `TcpListener` 手写最小 HTTP/1.1（仅这两个端点），不引入额外依赖，
//! 契合"轻量"取向。同一实例可同时是 hub（监听端口）与上报方（agent），由设置决定。

use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// 单日 token 用量（设备逐日序列的一项）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubDay {
    pub date: String,
    pub total: f64,
}

/// 单台设备上报的用量摘要。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubDevice {
    pub device_id: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub version: String,
    /// ISO8601 上报时间。
    pub reported_at: String,
    /// 近 90 天本地工具 token 合计（该设备的"工具消耗"）。
    pub tool_tokens: f64,
    pub provider_count: u64,
    pub tool_count: u64,
    /// 近 90 天逐日 token 序列（升序，缺量日补 0）。旧报告无此字段时回退为空。
    #[serde(default)]
    pub daily: Vec<HubDay>,
}

/// 启动一个阻塞式的 hub HTTP 服务线程。`storage` 用 clone 传入；服务会在独立线程
/// 运行并常驻，直到进程退出。绑定失败（端口占用）仅记录并返回 false。
/// `token` 非空时 `/ingest` 与 `/devices` 需携带 `Authorization: Bearer <token>`。
pub fn spawn_hub_server(storage: Arc<Storage>, port: u16, token: String) -> bool {
    let addr = format!("0.0.0.0:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[hub] bind {addr} failed: {e}");
            return false;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let storage = storage.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                let _ = handle_conn(stream, &storage, &token);
            });
        }
    });
    true
}

fn handle_conn(
    mut stream: TcpStream,
    storage: &Arc<Storage>,
    token: &str,
) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.trim().split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();

    // 读取请求头直到空行，拿到 Content-Length 与 Authorization。
    let mut content_length: usize = 0;
    let mut authorization: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = lower.strip_prefix("authorization:") {
            authorization = Some(trimmed[("authorization:".len())..].trim().to_string());
        }
    }

    // 鉴权：配置了 token 时，请求须带匹配的 Bearer token，否则 401。
    if !authorized(authorization.as_deref(), token) {
        let resp = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes())?;
        stream.flush()?;
        return Ok(());
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    let (status, payload) = if method == "GET" && (target == "/devices" || target == "/api/devices") {
        match list_payload(storage) {
            Ok(s) => ("200 OK", s),
            Err(e) => ("500 Internal Server Error", format!("{{\"error\":{}}}", json_escape(&e.to_string()))),
        }
    } else if method == "POST" && (target == "/ingest" || target == "/api/ingest") {
        match ingest(storage, &body) {
            Ok(_) => ("200 OK", "{\"ok\":true}".to_string()),
            Err(e) => ("400 Bad Request", format!("{{\"error\":{}}}", json_escape(&e.to_string()))),
        }
    } else {
        ("404 Not Found", "{\"error\":\"not found\"}".to_string())
    };

    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
        payload.len(),
        payload
    );
    stream.write_all(resp.as_bytes())?;
    stream.flush()
}

fn ingest(storage: &Arc<Storage>, body: &[u8]) -> Result<(), String> {
    let device: HubDevice = serde_json::from_slice(body).map_err(|e| format!("bad json: {e}"))?;
    if device.device_id.trim().is_empty() {
        return Err("device_id empty".into());
    }
    storage
        .upsert_hub_device(&device)
        .map_err(|e| e.to_string())
}

fn list_payload(storage: &Arc<Storage>) -> Result<String, String> {
    let devices = storage.list_hub_devices().map_err(|e| e.to_string())?;
    serde_json::to_string(&devices).map_err(|e| e.to_string())
}

fn json_escape(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// Bearer 鉴权判定：`token` 为空时不鉴权（放行）；否则请求须携带匹配的
/// `Authorization: Bearer <token>`。
fn authorized(authorization: Option<&str>, token: &str) -> bool {
    if token.is_empty() {
        return true;
    }
    authorization
        .is_some_and(|a| a.strip_prefix("Bearer ").is_some_and(|t| t.trim() == token))
}

/// 作为 agent 把本机用量上报到远端 hub。`token` 非空时附 `Authorization: Bearer`。
pub async fn report_to_hub(base: &str, device: &HubDevice, token: &str) -> Result<(), String> {
    let url = format!("{}/ingest", base.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let body = serde_json::to_vec(device).map_err(|e| e.to_string())?;
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body)
        .timeout(std::time::Duration::from_secs(8));
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("report failed: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("hub responded {}", resp.status()))
    }
}

/// 作为 client 拉取 hub 的设备列表。`token` 非空时附 `Authorization: Bearer`。
pub async fn fetch_devices(base: &str, token: &str) -> Result<Vec<HubDevice>, String> {
    let url = format!("{}/devices", base.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut req = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(8));
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("hub unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("hub responded {}", resp.status()));
    }
    resp.json::<Vec<HubDevice>>()
        .await
        .map_err(|e| format!("bad hub payload: {e}"))
}

/// 用轻量 marshal 构建设备摘要（避免依赖 reqwest/chrono 类型在此反复）。
pub fn build_device_usage(
    device_id: &str,
    hostname: &str,
    os: &str,
    arch: &str,
    version: &str,
    tool_tokens: f64,
    provider_count: u64,
    tool_count: u64,
    daily: Vec<HubDay>,
) -> HubDevice {
    HubDevice {
        device_id: device_id.to_string(),
        hostname: hostname.to_string(),
        os: os.to_string(),
        arch: arch.to_string(),
        version: version.to_string(),
        reported_at: chrono::Utc::now().to_rfc3339(),
        tool_tokens,
        provider_count,
        tool_count,
        daily,
    }
}

/// 本机设备标识元信息（id, hostname, os, arch, version）。供上报与 `get_device`
/// 复用，避免两处各自读环境变量。
pub fn machine_info() -> (String, String, String, String, String) {
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    (
        hostname.to_lowercase(),
        hostname,
        std::env::consts::OS.to_string(),
        std::env::consts::ARCH.to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    )
}

/// 从本地工具扫描缓存聚合近 90 天逐日 token 序列（跨工具按日求和，升序）。
/// 缓存未就绪时返回空序列。
pub async fn tool_daily_from_cache(
    local: &crate::local::SharedLocalCache,
) -> Vec<HubDay> {
    let Some(payload) = local.cached().await else {
        return Vec::new();
    };
    let mut by = std::collections::BTreeMap::<String, f64>::new();
    for tool in &payload.tools {
        for day in &tool.daily {
            *by.entry(day.date.clone()).or_default() += day.total;
        }
    }
    let mut keys: Vec<String> = by.keys().cloned().collect();
    keys.sort();
    if keys.len() > 90 {
        keys = keys[keys.len() - 90..].to_vec();
    }
    keys.into_iter()
        .map(|date| {
            let total = by[&date];
            HubDay { date, total }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_allows_when_no_token_configured() {
        assert!(authorized(None, ""));
        assert!(authorized(Some("Bearer anything"), ""));
    }

    #[test]
    fn auth_rejects_missing_or_wrong_token() {
        assert_eq!(authorized(None, "s3cret"), false);
        assert_eq!(authorized(Some("Bearer wrong"), "s3cret"), false);
        assert_eq!(authorized(Some("s3cret"), "s3cret"), false); // 缺少 Bearer 前缀
    }

    #[test]
    fn auth_accepts_matching_bearer_token() {
        assert_eq!(authorized(Some("Bearer s3cret"), "s3cret"), true);
    }

    #[test]
    fn report_roundtrips_in_storage() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("tum_hub_test_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let storage = crate::storage::Storage::open(&path).unwrap();
        let s = Arc::new(storage);

        let d1 = build_device_usage("alpha", "PC-A", "windows", "x86_64", "0.1.0", 123.0, 2, 3, Vec::new());
        let d2 = build_device_usage("beta", "PC-B", "windows", "arm64", "0.1.0", 456.0, 1, 2, Vec::new());
        s.upsert_hub_device(&d1).unwrap();
        s.upsert_hub_device(&d2).unwrap();

        let mut ids: Vec<String> =
            s.list_hub_devices().unwrap().into_iter().map(|d| d.device_id).collect();
        ids.sort();
        assert_eq!(ids, vec!["alpha", "beta"]);

        // 再次上报 alpha 覆盖而非重复。
        let d1b = build_device_usage("alpha", "PC-A", "windows", "x86_64", "0.1.1", 999.0, 2, 3, Vec::new());
        s.upsert_hub_device(&d1b).unwrap();
        let devices = s.list_hub_devices().unwrap();
        assert_eq!(devices.len(), 2);
        assert!(devices.iter().any(|d| d.device_id == "alpha" && d.tool_tokens == 999.0));
        let _ = std::fs::remove_file(&path);
    }
}