//! What actually changed in a settings save.
//!
//! Before this module every save_settings pinged every polling loop with a
//! bare unit value, and each loop answered by fetching immediately - so
//! toggling the display currency, the edge-snap flag or a window position
//! still cost one HTTP request per configured provider. The payload here lets
//! the scheduler answer "does this edit change *when or whether* I poll?"
//! instead of assuming yes.
//!
//! Classification is per-field and additive: a field nobody has classified yet
//! stays inert, which is the safe direction for a poll that was merely
//! skipped (the next regular tick still runs). A field that should have
//! triggered a fetch but is unclassified only costs freshness, never
//! correctness.
//!
//! The config.toml schema is untouched - this is purely an in-process signal.

use crate::settings::Settings;
use std::collections::HashSet;

/// The subset of a settings save that a poll loop cares about.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsDelta {
    /// A poll interval, account membership, enable flag or proxy changed.
    /// True means "an immediate fetch is worth it".
    pub poll_relevant: bool,
    /// A field the UI reads changed (currency, ring window, thresholds, hub
    /// role, window behaviour). Consumers re-render; poll loops ignore it.
    pub display_relevant: bool,
    /// Accounts were added or removed, as opposed to toggled.
    pub membership_changed: bool,
    /// Instance ids that flipped false -> true. A newly enabled account has
    /// no snapshot yet, so it must fetch even if nothing else changed.
    pub newly_enabled: Vec<String>,
    /// Instance ids that flipped true -> false. Polling for these stops; the
    /// registry reconcile in save_settings drops their state separately.
    pub newly_disabled: Vec<String>,
    /// TokenRouter 配置（开关/端口/路由表）发生了变化。端口变化时由
    /// save_settings 触发代理服务重启；其余字段每请求现读，无需重启。
    pub router_changed: bool,
}

impl SettingsDelta {
    /// Diff old against new and classify what moved.
    ///
    /// Callers must pass the settings as they were *before* the write; the
    /// SettingsStore cache is overwritten by save(), so the "before" value has
    /// to be captured up front.
    pub fn compute(old: &Settings, new: &Settings) -> Self {
        // ---- poll relevance --------------------------------------------
        // Interval, membership, enable flags and the proxy all change *when*
        // or *whether* a loop fetches, so they are the only poll triggers.
        let mut poll_relevant = old.poll_interval_seconds != new.poll_interval_seconds
            || old.proxy_url != new.proxy_url;

        // enabled_providers is the pre-multi-account list, already migrated
        // into accounts at load time. Editing it later cannot reach a poll
        // loop, so it is deliberately not poll-relevant.
        let old_enabled: HashSet<&str> = old
            .accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.instance_id.as_str())
            .collect();
        let new_enabled: HashSet<&str> = new
            .accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.instance_id.as_str())
            .collect();

        let mut newly_enabled: Vec<String> = new_enabled
            .difference(&old_enabled)
            .map(|s| (*s).to_string())
            .collect();
        let mut newly_disabled: Vec<String> = old_enabled
            .difference(&new_enabled)
            .map(|s| (*s).to_string())
            .collect();
        // Sorted so the payload is comparable in tests and stable in logs.
        newly_enabled.sort();
        newly_disabled.sort();

        if !newly_enabled.is_empty() || !newly_disabled.is_empty() {
            poll_relevant = true;
        }

        let old_ids: HashSet<&str> = old
            .accounts
            .iter()
            .map(|a| a.instance_id.as_str())
            .collect();
        let new_ids: HashSet<&str> = new
            .accounts
            .iter()
            .map(|a| a.instance_id.as_str())
            .collect();
        let membership_changed = old_ids != new_ids;

        if membership_changed {
            poll_relevant = true;
        }

        // ---- display relevance -----------------------------------------
        // Read by the frontend on settings-changed, or by the notify state
        // machine; none of these should cost a network request.
        let display_relevant = old.display_currency != new.display_currency
            || old.rate_overrides != new.rate_overrides
            || old.ring_window != new.ring_window
            || old.countdown_mode != new.countdown_mode
            || old.notify_enabled != new.notify_enabled
            || old.notify_warn_percent != new.notify_warn_percent
            || old.notify_crit_percent != new.notify_crit_percent
            || old.hub_mode != new.hub_mode
            || old.hub_port != new.hub_port
            || old.hub_base != new.hub_base
            || old.report_on != new.report_on
            || old.compact_mode != new.compact_mode
            || old.close_to_tray != new.close_to_tray
            || old.edge_snap != new.edge_snap
            || old.autostart != new.autostart
            || old.dashboard_x != new.dashboard_x
            || old.dashboard_y != new.dashboard_y
            // 数据目录变了 → 本地工具面板必须重扫（save_settings 已清缓存并
            // 重建 roots 快照；此处保证前端也收到刷新信号）。
            || old.tool_data_roots != new.tool_data_roots
            || old.tool_data_dirs != new.tool_data_dirs
            // 路由配置：设置页与主界面快速开关都需要立即反映。
            || old.router != new.router;

        Self {
            poll_relevant,
            display_relevant,
            membership_changed,
            newly_enabled,
            newly_disabled,
            router_changed: old.router != new.router,
        }
    }

    /// Whether this account's own enable state flipped in this save.
    pub fn toggled(&self, instance_id: &str) -> bool {
        self.newly_enabled.iter().any(|id| id == instance_id)
            || self.newly_disabled.iter().any(|id| id == instance_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::AccountMeta;

    fn account(id: &str, enabled: bool) -> AccountMeta {
        AccountMeta {
            instance_id: id.to_string(),
            provider_kind: "deepseek".to_string(),
            label: id.to_string(),
            accent_color: "#29b6f6".to_string(),
            enabled,
            note: None,
        }
    }

    fn base() -> Settings {
        let mut s = Settings::default();
        s.accounts = vec![account("deepseek", true), account("minimax", true)];
        s
    }

    #[test]
    fn identical_settings_produce_an_empty_delta() {
        let s = base();
        let d = SettingsDelta::compute(&s, &s.clone());
        assert!(!d.poll_relevant);
        assert!(!d.display_relevant);
        assert!(!d.membership_changed);
        assert!(d.newly_enabled.is_empty());
        assert!(d.newly_disabled.is_empty());
    }

    #[test]
    fn display_only_edit_is_not_poll_relevant() {
        let old = base();
        let mut new = old.clone();
        new.display_currency = "CNY".to_string();
        new.ring_window = "daily".to_string();
        new.edge_snap = false;
        let d = SettingsDelta::compute(&old, &new);
        assert!(d.display_relevant, "UI-visible fields changed");
        assert!(!d.poll_relevant, "must not spend a request per provider");
    }

    #[test]
    fn interval_change_is_poll_relevant_without_membership_change() {
        let old = base();
        let mut new = old.clone();
        new.poll_interval_seconds = 120;
        let d = SettingsDelta::compute(&old, &new);
        assert!(d.poll_relevant);
        assert!(!d.membership_changed);
        assert!(d.newly_enabled.is_empty() && d.newly_disabled.is_empty());
    }

    #[test]
    fn disabling_an_account_reports_it_as_newly_disabled() {
        let old = base();
        let mut new = old.clone();
        new.accounts[1].enabled = false;
        let d = SettingsDelta::compute(&old, &new);
        assert_eq!(d.newly_disabled, vec!["minimax".to_string()]);
        assert!(d.newly_enabled.is_empty());
        assert!(!d.membership_changed, "both accounts still exist");
        assert!(d.poll_relevant);
        assert!(d.toggled("minimax"));
        assert!(!d.toggled("deepseek"));
    }

    #[test]
    fn adding_an_account_is_a_membership_change() {
        let old = base();
        let mut new = old.clone();
        new.accounts.push(account("codex", true));
        let d = SettingsDelta::compute(&old, &new);
        assert!(d.membership_changed);
        assert!(d.poll_relevant);
        assert_eq!(d.newly_enabled, vec!["codex".to_string()]);
        assert!(d.newly_disabled.is_empty());
    }

    #[test]
    fn proxy_change_is_poll_relevant() {
        // A new proxy rebuilds every provider, so stale data would mislead.
        let old = base();
        let mut new = old.clone();
        new.proxy_url = Some("socks5://127.0.0.1:1080".to_string());
        let d = SettingsDelta::compute(&old, &new);
        assert!(d.poll_relevant);
        assert!(!d.membership_changed);
    }

    #[test]
    fn account_metadata_edit_alone_does_not_poll() {
        // Renaming or recolouring a card is presentation; the fetched usage is
        // identical, so spending a request would be waste.
        let old = base();
        let mut new = old.clone();
        new.accounts[0].label = "main account".to_string();
        new.accounts[0].accent_color = "#ff5252".to_string();
        let d = SettingsDelta::compute(&old, &new);
        assert!(!d.poll_relevant);
        assert!(!d.membership_changed);
    }

    #[test]
    fn router_edits_are_display_relevant_only() {
        let old = base();
        let mut new = old.clone();
        new.router.enabled = true;
        new.router.port = 43212;
        let d = SettingsDelta::compute(&old, &new);
        assert!(d.display_relevant);
        assert!(d.router_changed);
        assert!(!d.poll_relevant, "路由器不参与轮询，不该白白打一轮请求");
    }

    #[test]
    fn accounts_disabled_in_both_versions_produce_nothing() {
        let old = base();
        let mut new = old.clone();
        new.accounts[0].enabled = false;
        let d = SettingsDelta::compute(&old, &new);
        assert_eq!(d.newly_disabled, vec!["deepseek".to_string()]);

        // A save where both accounts stay off changes nothing.
        let mut both_off = base();
        for a in &mut both_off.accounts {
            a.enabled = false;
        }
        let d2 = SettingsDelta::compute(&both_off, &both_off);
        assert!(!d2.poll_relevant);
    }
}
