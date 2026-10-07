//! Rust 侧面向用户文案（托盘菜单、窗口标题、系统通知）的多语言支持。
//!
//! 前端有更完整的 i18n（`src/lib/i18n/`）；这里只覆盖少量 Rust 侧文案。
//! 语言取值与前端约定一致：`"auto"` | `"zh-CN"` | `"en"`，写在
//! `Settings.language`；`"auto"` 与无法识别的值回退系统语言，系统语言不是
//! 英文时回退中文（项目默认语言）。

/// 受支持的语言。文案直接写在消息函数里（总量个位数，不值得建表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    ZhCn,
    En,
}

/// 解析设置里的语言值。`"en"` / `"zh-CN"`（兼容 `"zh"`）显式生效，
/// 其余（含 `"auto"`、空串、旧配置缺省）跟随系统语言。
pub fn resolve(setting: &str) -> Lang {
    match setting.trim().to_ascii_lowercase().as_str() {
        "en" => Lang::En,
        "zh-cn" | "zh" => Lang::ZhCn,
        _ => system_lang(),
    }
}

fn system_lang() -> Lang {
    // sys_locale 读当前用户区域（如 "en-US" / "zh-CN"）；取不到按中文处理。
    sys_locale::get_locale()
        .map(|l| {
            if l.to_ascii_lowercase().starts_with("en") {
                Lang::En
            } else {
                Lang::ZhCn
            }
        })
        .unwrap_or(Lang::ZhCn)
}

// --- 托盘菜单 ---

pub fn tray_show(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "显示面板",
        Lang::En => "Show Panel",
    }
}

pub fn tray_quit(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "退出",
        Lang::En => "Quit",
    }
}

// --- 窗口标题后缀（前缀恒为品牌名 "TokenUsageMonitor · "） ---

#[derive(Debug, Clone, Copy)]
pub enum WindowPage {
    PeekHandle,
    Settings,
    Trend,
    Tools,
}

pub fn window_suffix(lang: Lang, page: WindowPage) -> &'static str {
    match (lang, page) {
        (Lang::ZhCn, WindowPage::PeekHandle) => "贴边把手",
        (Lang::ZhCn, WindowPage::Settings) => "设置",
        (Lang::ZhCn, WindowPage::Trend) => "用量趋势",
        (Lang::ZhCn, WindowPage::Tools) => "工具用量",
        (Lang::En, WindowPage::PeekHandle) => "Dock Handle",
        (Lang::En, WindowPage::Settings) => "Settings",
        (Lang::En, WindowPage::Trend) => "Usage Trend",
        (Lang::En, WindowPage::Tools) => "Tool Usage",
    }
}

/// 完整窗口标题。语言切换不影响已开窗口的标题（重开时生效）。
pub fn window_title(lang: Lang, page: WindowPage) -> String {
    format!("TokenUsageMonitor · {}", window_suffix(lang, page))
}

// --- 阈值通知（notify.rs 状态机） ---

pub fn usage_title(lang: Lang, crit: bool, provider: &str, pct: u8) -> String {
    match (lang, crit) {
        (Lang::ZhCn, false) => format!("{provider} 用量提醒（{pct}%）"),
        (Lang::ZhCn, true) => format!("{provider} 用量告急（{pct}%）"),
        (Lang::En, false) => format!("{provider} usage warning ({pct}%)"),
        (Lang::En, true) => format!("{provider} usage critical ({pct}%)"),
    }
}

/// 正文：`{provider} 本窗口已用 {pct}%`，燃烧率可估时追加「约 … 后耗尽」。
pub fn usage_body(lang: Lang, provider: &str, pct: u8, eta_secs: Option<u64>) -> String {
    let mut body = match lang {
        Lang::ZhCn => format!("{provider} 本窗口已用 {pct}%"),
        Lang::En => format!("{provider} used {pct}% of this window"),
    };
    if let Some(secs) = eta_secs {
        let eta = eta_text(lang, secs);
        if !eta.is_empty() {
            body.push_str(match lang {
                Lang::ZhCn => "，",
                Lang::En => ", ",
            });
            body.push_str(&eta);
        }
    }
    body
}

/// 燃烧率 ETA 文案。不足 1 分钟统一为「约 1 分钟内」。
pub fn eta_text(lang: Lang, secs: u64) -> String {
    let mins = secs / 60;
    match lang {
        Lang::ZhCn => {
            if mins >= 60 {
                format!("约 {} 小时 {} 分钟后耗尽", mins / 60, mins % 60)
            } else if mins >= 1 {
                format!("约 {mins} 分钟后耗尽")
            } else {
                "约 1 分钟内耗尽".to_string()
            }
        }
        Lang::En => {
            if mins >= 60 {
                format!("about {} h {} m until exhausted", mins / 60, mins % 60)
            } else if mins >= 1 {
                format!("about {mins} min until exhausted")
            } else {
                "exhausted within ~1 min".to_string()
            }
        }
    }
}

// --- 更新通知（lib.rs 启动检查） ---

pub fn update_notify(lang: Lang, version: &str, url: Option<&str>) -> (String, String) {
    let body = match (lang, url) {
        (Lang::ZhCn, Some(u)) => format!("最新版本 {version}，点击查看：{u}"),
        (Lang::ZhCn, None) => {
            format!("最新版本 {version}，可在设置 → 关于与诊断 中检查。")
        }
        (Lang::En, Some(u)) => format!("Version {version} is available. See: {u}"),
        (Lang::En, None) => {
            format!("Version {version} is available — check it under Settings → About & Diagnostics.")
        }
    };
    let title = match lang {
        Lang::ZhCn => "TokenUsageMonitor 有新版本".to_string(),
        Lang::En => "TokenUsageMonitor update available".to_string(),
    };
    (title, body)
}

// --- TokenRouter 切换通知（router/server.rs） ---
// reason 取值与 RouterSwitchEvent 一致："failover" | "failback"。

pub fn router_switch_notify(
    lang: Lang,
    route: &str,
    from: &str,
    to: &str,
    reason: &str,
) -> (String, String) {
    let reason_text = match (lang, reason) {
        (Lang::ZhCn, "failover") => "主线路配额受限，已自动切换",
        (Lang::ZhCn, _) => "主线路已恢复，切回主线路",
        (Lang::En, "failover") => "primary route quota limited, switched automatically",
        (Lang::En, _) => "primary route recovered, switched back",
    };
    let body = match lang {
        Lang::ZhCn => format!("{route}：{from} → {to}（{reason_text}）"),
        Lang::En => format!("{route}: {from} → {to} ({reason_text})"),
    };
    let title = match lang {
        Lang::ZhCn => "TokenRouter 已切换".to_string(),
        Lang::En => "TokenRouter switched".to_string(),
    };
    (title, body)
}

#[cfg(test)]
mod i18n_tests {
    use super::*;

    #[test]
    fn explicit_languages_resolve_directly() {
        assert_eq!(resolve("en"), Lang::En);
        assert_eq!(resolve("zh-CN"), Lang::ZhCn);
        assert_eq!(resolve("zh"), Lang::ZhCn);
        assert_eq!(resolve(" EN "), Lang::En);
    }

    #[test]
    fn unknown_values_fall_back_to_a_supported_language() {
        // auto / 空串 / 旧配置缺省 / 乱值都回退系统语言——系统语言本身受
        // 测试机影响，只断言结果合法。
        for setting in ["auto", "", "fr-FR", "zh-TW"] {
            assert!(
                matches!(resolve(setting), Lang::ZhCn | Lang::En),
                "resolve({setting:?}) must yield a supported lang"
            );
        }
    }

    #[test]
    fn tray_strings_differ_per_language() {
        assert_eq!(tray_show(Lang::ZhCn), "显示面板");
        assert_eq!(tray_show(Lang::En), "Show Panel");
        assert_eq!(tray_quit(Lang::ZhCn), "退出");
        assert_eq!(tray_quit(Lang::En), "Quit");
    }

    #[test]
    fn window_titles_keep_the_brand_prefix() {
        assert_eq!(window_title(Lang::ZhCn, WindowPage::Settings), "TokenUsageMonitor · 设置");
        assert_eq!(window_title(Lang::En, WindowPage::Trend), "TokenUsageMonitor · Usage Trend");
        assert_eq!(window_title(Lang::En, WindowPage::Tools), "TokenUsageMonitor · Tool Usage");
        assert_eq!(window_title(Lang::En, WindowPage::PeekHandle), "TokenUsageMonitor · Dock Handle");
    }

    #[test]
    fn usage_notification_warn_and_crit_titles() {
        assert_eq!(
            usage_title(Lang::ZhCn, false, "方舟", 82),
            "方舟 用量提醒（82%）"
        );
        assert_eq!(
            usage_title(Lang::ZhCn, true, "方舟", 96),
            "方舟 用量告急（96%）"
        );
        assert_eq!(
            usage_title(Lang::En, false, "Ark", 82),
            "Ark usage warning (82%)"
        );
        assert_eq!(
            usage_title(Lang::En, true, "Ark", 96),
            "Ark usage critical (96%)"
        );
    }

    #[test]
    fn usage_body_appends_eta_only_when_known() {
        assert_eq!(
            usage_body(Lang::ZhCn, "方舟", 82, None),
            "方舟 本窗口已用 82%"
        );
        // 25800s = 430 min = 7 h 10 m。
        assert_eq!(
            usage_body(Lang::ZhCn, "方舟", 82, Some(5 * 3600 + 130 * 60)),
            "方舟 本窗口已用 82%，约 7 小时 10 分钟后耗尽"
        );
        assert_eq!(
            usage_body(Lang::En, "Ark", 82, Some(3 * 60)),
            "Ark used 82% of this window, about 3 min until exhausted"
        );
        // 不足 1 分钟统一为「约 1 分钟内」。
        assert_eq!(
            usage_body(Lang::ZhCn, "方舟", 82, Some(30)),
            "方舟 本窗口已用 82%，约 1 分钟内耗尽"
        );
    }

    #[test]
    fn update_notification_covers_both_url_variants() {
        let (zh_title, zh_body) =
            update_notify(Lang::ZhCn, "0.3.0", Some("https://github.com/x/r"));
        assert_eq!(zh_title, "TokenUsageMonitor 有新版本");
        assert!(zh_body.contains("0.3.0") && zh_body.contains("github.com"));

        let (en_title, en_body) = update_notify(Lang::En, "0.3.0", None);
        assert_eq!(en_title, "TokenUsageMonitor update available");
        assert!(en_body.contains("0.3.0") && en_body.contains("Settings"));
    }

    #[test]
    fn router_switch_notification_maps_reasons() {
        let (zh_title, zh_body) =
            router_switch_notify(Lang::ZhCn, "主力", "glm-4.7", "kimi-k2", "failover");
        assert_eq!(zh_title, "TokenRouter 已切换");
        assert!(zh_body.contains("主线路配额受限") && zh_body.contains("glm-4.7 → kimi-k2"));

        let (_, en_body) =
            router_switch_notify(Lang::En, "main", "glm-4.7", "kimi-k2", "failback");
        assert!(en_body.contains("recovered") && en_body.contains("kimi-k2"));
    }
}
