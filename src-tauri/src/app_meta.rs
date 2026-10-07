//! 应用元信息与版本检查。
//!
//! - `get_app_meta`：版本号 + GitHub 仓库地址（前端"关于"页渲染用，独立于
//!   `get_device_report`，避免把仓库地址混进 hub 同步的数据模型）。
//! - `open_url`：受控地在系统浏览器打开仓库页面。Tauri 前端 `<a>` 不会走
//!   系统浏览器，而引入 opener 插件只为一个链接不值得，故用 explorer 直开。
//! - `check_app_update`：查 GitHub Releases 最新版并与当前版本比较。
//!   不做自动下载/安装——签名分发超出本项目的发布形态，提示到 Release 页即可。

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// GitHub 仓库 slug（owner/repo）。仓库直达链接与更新检查的唯一来源：
/// 建仓后只需改这一处（README、Cargo.toml repository 字段另行同步）。
pub const GITHUB_REPO: &str = "PaytonX/TokenUsageMonitor";

/// `open_url` 允许打开的 host 白名单。release 元数据里携带的 html_url 只有
/// 命中名单才会交给系统浏览器，防止被构造的 URL 注入非预期地址。
const ALLOWED_OPEN_HOSTS: [&str; 2] = ["github.com", "www.github.com"];

/// 更新检查的固定 API 端点。host 校验在发送前做一次，代码虽只此一处调用，
/// 但保持"先校验后请求"的书写习惯，避免后续改动时绕过检查。
const RELEASES_API_HOST: &str = "api.github.com";

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn repo_url() -> String {
    format!("https://github.com/{GITHUB_REPO}")
}

#[derive(Serialize)]
pub struct AppMeta {
    pub version: String,
    pub repo_url: String,
}

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub current: String,
    /// 远端最新 tag（原样保留 "v" 前缀）；仓库无 release 时为 None。
    pub latest: Option<String>,
    pub has_update: bool,
    pub url: Option<String>,
    pub published_at: Option<String>,
}

/// https + 白名单 host 校验。拒绝非 https、userinfo 混淆（user@evil.com）、
/// 端口后缀等其他形态——host 提取自 scheme 后第一个 `/`/`?`/`#` 之前的部分。
pub fn is_allowed_open_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host_authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // 去掉可选的 user@ 前缀，统一小写比较。
    let host = host_authority.rsplit('@').next().unwrap_or(host_authority);
    // 显式端口一律拒绝：GitHub 页面只在默认 443 上提供，带端口的 URL
    // 不属于仓库直达的合法形态（IPv6 字面量同样被拒，本名单只收域名）。
    if host.contains(':') {
        return false;
    }
    ALLOWED_OPEN_HOSTS.contains(&host.to_ascii_lowercase().as_str())
}

/// 比较远端 tag 与当前版本。tag 允许带 v/V 前缀；任一侧无法按 semver 解析时
/// 返回 None（调用方视为"无更新"——宁可不提示也不误报）。
pub fn tag_is_newer(current: &str, tag: &str) -> Option<bool> {
    let raw = tag.trim().trim_start_matches(['v', 'V']);
    let cur = semver::Version::parse(current.trim()).ok()?;
    let latest = semver::Version::parse(raw).ok()?;
    Some(latest > cur)
}

#[tauri::command]
pub fn get_app_meta() -> AppMeta {
    AppMeta {
        version: current_version(),
        repo_url: repo_url(),
    }
}

/// 在系统浏览器打开一个白名单内的 https 链接。仅支持仓库相关页面，
/// 所以只实现了 Windows（本项目唯一目标平台）。
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !is_allowed_open_url(&url) {
        return Err(format!("url is not on the open allowlist: {url}"));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("opening browser failed: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("open_url is only implemented on Windows".to_string())
    }
}

#[derive(Deserialize)]
struct ReleaseResp {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
}

/// 查询 GitHub Releases 最新发布并与当前版本比较，复用设置里的共享 HTTP
/// client（带用户的代理配置）。GitHub API 要求请求带 User-Agent。
pub async fn fetch_latest_release(client: &reqwest::Client) -> Result<UpdateInfo, String> {
    let api_url = format!("https://{RELEASES_API_HOST}/repos/{GITHUB_REPO}/releases/latest");
    if !is_releases_api_url(&api_url) {
        return Err("releases api url failed host validation".to_string());
    }
    let resp = client
        .get(&api_url)
        .header(
            reqwest::header::USER_AGENT,
            format!("TokenUsageMonitor/{}", current_version()),
        )
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("requesting latest release failed: {e}"))?;

    // 仓库还没有任何 release（404）不算错误，静默视为"无更新"。
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateInfo {
            current: current_version(),
            latest: None,
            has_update: false,
            url: None,
            published_at: None,
        });
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }
    let rel: ReleaseResp = resp
        .json()
        .await
        .map_err(|e| format!("decoding release failed: {e}"))?;
    let has_update = tag_is_newer(&current_version(), &rel.tag_name).unwrap_or(false);
    let url = is_allowed_open_url(&rel.html_url).then_some(rel.html_url);
    Ok(UpdateInfo {
        current: current_version(),
        latest: Some(rel.tag_name),
        has_update,
        url,
        published_at: rel.published_at,
    })
}

/// 校验一个 URL 是否为本模块允许的 releases API 端点（https + 指定 host）。
fn is_releases_api_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    host.eq_ignore_ascii_case(RELEASES_API_HOST)
}

#[tauri::command]
pub async fn check_app_update(state: tauri::State<'_, crate::AppState>) -> Result<UpdateInfo, String> {
    let client = state.http.read().await.clone();
    fetch_latest_release(&client).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_url_allows_github_https_only() {
        assert!(is_allowed_open_url("https://github.com/PaytonX/TokenUsageMonitor/releases"));
        // 通用夹具：只验 www. 主机变体被放行，仓库名本身不参与判定。
        assert!(is_allowed_open_url("https://www.github.com/anyone/anything"));
        assert!(is_allowed_open_url("https://GITHUB.com/x"));
    }

    #[test]
    fn open_url_rejects_non_https_and_foreign_hosts() {
        assert!(!is_allowed_open_url("http://github.com/x"));
        assert!(!is_allowed_open_url("https://evil.com/github.com"));
        assert!(!is_allowed_open_url("https://user@evil.com"));
        assert!(!is_allowed_open_url("https://github.com:8443/x"));
        assert!(!is_allowed_open_url("https://"));
        assert!(!is_allowed_open_url("file:///C:/Windows/System32"));
        assert!(!is_allowed_open_url(""));
    }

    #[test]
    fn tag_comparison_handles_v_prefix_and_prerelease() {
        assert_eq!(tag_is_newer("0.1.0", "v0.2.0"), Some(true));
        assert_eq!(tag_is_newer("0.1.0", "0.1.0"), Some(false));
        assert_eq!(tag_is_newer("0.2.0", "v0.1.9"), Some(false));
        // semver 规则：pre-release 低于正式版，正式版 0.1.0 > 0.1.0-rc.1
        assert_eq!(tag_is_newer("0.1.0", "v0.1.0-rc.1"), Some(false));
        assert_eq!(tag_is_newer("0.1.0-rc.1", "v0.1.0"), Some(true));
    }

    #[test]
    fn unparseable_tags_never_report_update() {
        assert_eq!(tag_is_newer("0.1.0", "not-a-version"), None);
        assert_eq!(tag_is_newer("", "v0.2.0"), None);
    }

    #[test]
    fn releases_api_url_validation() {
        assert!(is_releases_api_url("https://api.github.com/repos/x/y/releases/latest"));
        assert!(!is_releases_api_url("http://api.github.com/repos/x/y"));
        assert!(!is_releases_api_url("https://evil.com/repos/x/y"));
    }
}
