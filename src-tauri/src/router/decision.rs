//! TokenRouter 路由决策：纯函数状态机。
//!
//! 选择策略恒定：按链序取第一个「可用」候选（冷却未过 / 低配额都算不可用）。
//! 因此 fail-back 不需要独立逻辑——链头一旦恢复（冷却到期或配额回升）自然
//! 重新成为第一个可用者。全部不可用时降级到第一个未冷却者，仍比直接拒绝好。
//!
//! 所有函数以 `now` 为参数注入时钟，测试无需 sleep。

use crate::router::config::{CandidateConfig, RouterSettings};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 候选进入冷却的原因。Quota 冷却到期后还要求恢复到 failback 阈值才回用，
/// 防止在阈值下方反复 429 → 冷却 → 立即再撞的抖动。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CooldownReason {
    Quota,
    Auth,
    Error,
}

/// 单候选运行态（每路由一份，随配置对齐）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CandidateState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_reason: Option<CooldownReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// 每路由运行态。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteState {
    /// 最近一次实际承接请求的候选下标（UI 的「当前激活」）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_index: Option<usize>,
    pub candidates: Vec<CandidateState>,
}

impl RouteState {
    /// 候选数量随配置变化时就地扩缩：截断或以默认态补齐，保留前缀状态。
    /// 每次访问该路由运行态前调用一次，热更新路由表即无需专门迁移。
    pub fn align(&mut self, len: usize) {
        self.candidates.resize(len, CandidateState::default());
    }
}

/// 决策用配额视图。`remaining` 为 None 表示没有权威数据——数据缺失不拦截
/// 请求（路由器宁可多发一次上游，也不因监控缺口拒绝服务）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct QuotaView {
    pub remaining: Option<f64>,
}

impl QuotaView {
    /// 取两个视图的较小者（快照窗口 vs 手填日限）。
    pub fn combine(self, other: QuotaView) -> QuotaView {
        QuotaView {
            remaining: match (self.remaining, other.remaining) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        }
    }
}

/// 决策参数（`Settings.router` 的相关快照）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecisionPolicy {
    pub failover_threshold_percent: u8,
    pub failback_threshold_percent: u8,
    pub error_cooldown_secs: u64,
}

impl DecisionPolicy {
    pub fn from_settings(s: &RouterSettings) -> Self {
        Self {
            failover_threshold_percent: s.failover_threshold_percent,
            failback_threshold_percent: s.failback_threshold_percent,
            error_cooldown_secs: s.error_cooldown_secs,
        }
    }
}

/// 5xx / 网络错误的短冷却：防对死上游连打，又不至于像配额错那样长罚。
pub const SERVER_ERROR_COOLDOWN_SECS: u64 = 30;

/// 决策前解析好的单候选输入（快照读取是异步的，须在 pick 之前完成）。
pub struct ResolvedCandidate<'a> {
    pub config: &'a CandidateConfig,
    pub quota: QuotaView,
    /// 凭据可用（BearerKey 且非空）。false 的候选连降级都不参与。
    pub usable: bool,
}

/// 一次上游结果的分类。
#[derive(Debug, Clone, PartialEq)]
pub enum UpstreamOutcome {
    Success,
    QuotaError {
        /// 上游 Retry-After 秒数；无则用 error_cooldown_secs。
        retry_after_secs: Option<u64>,
    },
    AuthError,
    ServerError,
    /// 参数类 4xx：透传给客户端，不冷却也不切换（避免掩盖真实错误）。
    ClientError,
}

impl UpstreamOutcome {
    /// 是否应当换下一个候选重试。
    pub fn retryable(&self) -> bool {
        !matches!(self, UpstreamOutcome::Success | UpstreamOutcome::ClientError)
    }
}

/// HTTP 状态码 → 结果分类（Retry-After 由服务器层补充解析）。
pub fn classify_status(status: u16) -> UpstreamOutcome {
    match status {
        429 | 402 => UpstreamOutcome::QuotaError {
            retry_after_secs: None,
        },
        401 | 403 => UpstreamOutcome::AuthError,
        s if (500..600).contains(&s) => UpstreamOutcome::ServerError,
        _ => UpstreamOutcome::ClientError,
    }
}

fn is_cooling(state: &CandidateState, now: DateTime<Utc>) -> bool {
    state.cooldown_until.is_some_and(|until| until > now)
}

/// 候选此刻是否可承接新请求。
pub fn available(
    state: &CandidateState,
    quota: QuotaView,
    policy: DecisionPolicy,
    now: DateTime<Utc>,
) -> bool {
    if is_cooling(state, now) {
        return false;
    }
    let remaining_pct = |r: f64| r * 100.0;
    if let Some(r) = quota.remaining {
        // 主动切换：剩余不足阈值的新请求绕开该候选。
        if remaining_pct(r) < policy.failover_threshold_percent as f64 {
            return false;
        }
    }
    // 配额冷却刚到期：恢复不足 failback 阈值则继续回避（防抖）。
    // 无配额数据时放行——不因监控缺口永久封锁一个候选。
    if state.cooldown_reason == Some(CooldownReason::Quota) && state.cooldown_until.is_some() {
        if let Some(r) = quota.remaining {
            if remaining_pct(r) < policy.failback_threshold_percent as f64 {
                return false;
            }
        }
    }
    true
}

/// 选出本次请求的候选下标：链序第一个可用者；无则第一个未冷却且凭据可用者
/// （低配额的候选在降级时仍可用——发一次撞墙好过直接拒绝，撞了会进冷却）。
pub fn pick_candidate(
    candidates: &[ResolvedCandidate<'_>],
    states: &[CandidateState],
    policy: DecisionPolicy,
    now: DateTime<Utc>,
) -> Option<usize> {
    let state_of = |i: usize| states.get(i).cloned().unwrap_or_default();
    let mut fallback = None;
    for (i, c) in candidates.iter().enumerate() {
        if !c.usable {
            continue;
        }
        if available(&state_of(i), c.quota, policy, now) {
            return Some(i);
        }
        if fallback.is_none() && !is_cooling(&state_of(i), now) {
            fallback = Some(i);
        }
    }
    fallback
}

/// 把一次上游结果写回候选运行态。返回是否进入了值得关注的冷却（新事件）。
pub fn on_result(
    states: &mut [CandidateState],
    index: usize,
    outcome: &UpstreamOutcome,
    policy: DecisionPolicy,
    now: DateTime<Utc>,
    message: Option<&str>,
) -> bool {
    let Some(st) = states.get_mut(index) else {
        return false;
    };
    match outcome {
        UpstreamOutcome::Success => {
            st.last_error = None;
            // Error 冷却（网络抖动）可以被成功洗白；Quota/Auth 冷却不行——
            // 一次成功不代表配额恢复，那个由快照/冷却到期判定。
            if st.cooldown_reason == Some(CooldownReason::Error) {
                st.cooldown_until = None;
                st.cooldown_reason = None;
            }
            false
        }
        UpstreamOutcome::QuotaError { retry_after_secs } => {
            let secs = retry_after_secs.unwrap_or(policy.error_cooldown_secs).max(1);
            st.cooldown_until = Some(now + chrono::Duration::seconds(secs as i64));
            st.cooldown_reason = Some(CooldownReason::Quota);
            if let Some(m) = message {
                st.last_error = Some(m.to_string());
            }
            true
        }
        UpstreamOutcome::AuthError => {
            st.cooldown_until =
                Some(now + chrono::Duration::seconds(policy.error_cooldown_secs as i64));
            st.cooldown_reason = Some(CooldownReason::Auth);
            if let Some(m) = message {
                st.last_error = Some(m.to_string());
            }
            true
        }
        UpstreamOutcome::ServerError => {
            st.cooldown_until = Some(
                now + chrono::Duration::seconds(SERVER_ERROR_COOLDOWN_SECS as i64),
            );
            st.cooldown_reason = Some(CooldownReason::Error);
            if let Some(m) = message {
                st.last_error = Some(m.to_string());
            }
            true
        }
        // 参数类 4xx 是请求自己的问题，与候选健康无关。
        UpstreamOutcome::ClientError => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> DecisionPolicy {
        DecisionPolicy {
            failover_threshold_percent: 20,
            failback_threshold_percent: 50,
            error_cooldown_secs: 300,
        }
    }

    fn cand(account: &str) -> CandidateConfig {
        CandidateConfig {
            account: account.to_string(),
            model: format!("model-{account}"),
            base_url: "https://upstream.test".to_string(),
            plan_limit_tokens_daily: None,
        }
    }

    fn resolved<'a>(config: &'a CandidateConfig, remaining: Option<f64>) -> ResolvedCandidate<'a> {
        ResolvedCandidate {
            config,
            quota: QuotaView { remaining },
            usable: true,
        }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc::now() + chrono::Duration::seconds(secs)
    }

    #[test]
    fn picks_chain_head_when_all_available() {
        let configs = [cand("a"), cand("b"), cand("c")];
        let cands = [
            resolved(&configs[0], Some(0.9)),
            resolved(&configs[1], Some(0.8)),
            resolved(&configs[2], Some(0.7)),
        ];
        assert_eq!(pick_candidate(&cands, &[], policy(), Utc::now()), Some(0));
    }

    #[test]
    fn low_quota_head_is_skipped_and_failure_back_is_automatic() {
        let configs = [cand("a"), cand("b")];
        // 链头剩 15%（< failover 20%）→ 不可用；候选 2 接管。
        let cands = [
            resolved(&configs[0], Some(0.15)),
            resolved(&configs[1], Some(0.9)),
        ];
        assert_eq!(pick_candidate(&cands, &[], policy(), Utc::now()), Some(1));
        // 链头恢复到 failback 阈值以上 → 自动切回，无需额外状态。
        let cands = [
            resolved(&configs[0], Some(0.6)),
            resolved(&configs[1], Some(0.9)),
        ];
        assert_eq!(pick_candidate(&cands, &[], policy(), Utc::now()), Some(0));
    }

    #[test]
    fn missing_quota_data_does_not_block() {
        // 无快照 ≠ 不可用：监控缺口不拦截请求。
        let configs = [cand("a")];
        let cands = [resolved(&configs[0], None)];
        assert_eq!(pick_candidate(&cands, &[], policy(), Utc::now()), Some(0));
    }

    #[test]
    fn cooldown_skips_candidate_until_it_expires() {
        let configs = [cand("a"), cand("b")];
        let mut states = vec![CandidateState::default(), CandidateState::default()];
        states[0].cooldown_until = Some(at(60));
        states[0].cooldown_reason = Some(CooldownReason::Quota);
        let cands = [
            resolved(&configs[0], None),
            resolved(&configs[1], Some(0.9)),
        ];
        assert_eq!(pick_candidate(&cands, &states, policy(), Utc::now()), Some(1));
        // 冷却过期后回到链头。
        assert_eq!(
            pick_candidate(&cands, &states, policy(), at(120)),
            Some(0)
        );
    }

    #[test]
    fn quota_cooldown_expiry_requires_failback_threshold() {
        let configs = [cand("a"), cand("b")];
        let mut states = vec![CandidateState::default(), CandidateState::default()];
        states[0].cooldown_until = Some(at(60));
        states[0].cooldown_reason = Some(CooldownReason::Quota);
        // 冷却到期但快照显示剩 30%（< failback 50%）→ 继续回避。
        let cands = [
            resolved(&configs[0], Some(0.3)),
            resolved(&configs[1], Some(0.9)),
        ];
        assert_eq!(
            pick_candidate(&cands, &states, policy(), at(120)),
            Some(1)
        );
        // Auth 冷却到期不受 failback 阈值约束（凭据修复即恢复）。
        states[0].cooldown_reason = Some(CooldownReason::Auth);
        assert_eq!(
            pick_candidate(&cands, &states, policy(), at(120)),
            Some(0)
        );
    }

    #[test]
    fn all_cooling_falls_back_to_low_quota_candidate() {
        // 两个都在配额冷却 → None。
        let configs = [cand("a"), cand("b")];
        let cooling = |until| CandidateState {
            cooldown_until: Some(until),
            cooldown_reason: Some(CooldownReason::Quota),
            last_error: None,
        };
        let states = vec![cooling(at(60)), cooling(at(60))];
        let cands = [
            resolved(&configs[0], Some(0.1)),
            resolved(&configs[1], Some(0.1)),
        ];
        assert_eq!(pick_candidate(&cands, &states, policy(), Utc::now()), None);
        // 只有候选 2 冷却 → 降级用未冷却的低配额链头。
        let states = vec![CandidateState::default(), cooling(at(60))];
        assert_eq!(pick_candidate(&cands, &states, policy(), Utc::now()), Some(0));
    }

    #[test]
    fn unusable_candidates_are_skipped_entirely() {
        let configs = [cand("a"), cand("b")];
        let mut c = resolved(&configs[0], Some(1.0));
        c.usable = false;
        let cands = [c, resolved(&configs[1], Some(0.9))];
        assert_eq!(pick_candidate(&cands, &[], policy(), Utc::now()), Some(1));
    }

    #[test]
    fn quota_error_sets_cooldown_and_retry_after_wins() {
        let mut states = vec![CandidateState::default()];
        let p = policy();
        assert!(on_result(
            &mut states,
            0,
            &UpstreamOutcome::QuotaError { retry_after_secs: Some(10) },
            p,
            Utc::now(),
            Some("429")
        ));
        assert!(states[0].cooldown_until.is_some());
        assert_eq!(states[0].cooldown_reason, Some(CooldownReason::Quota));
        // Retry-After 10s < 默认 300s：取上游指示。
        let until = states[0].cooldown_until.unwrap();
        assert!(until <= at(15), "retry_after must beat default cooldown");
    }

    #[test]
    fn success_clears_error_cooldown_but_not_quota_cooldown() {
        let mut states = vec![CandidateState {
            cooldown_until: Some(at(60)),
            cooldown_reason: Some(CooldownReason::Error),
            last_error: Some("boom".into()),
        }];
        on_result(&mut states, 0, &UpstreamOutcome::Success, policy(), Utc::now(), None);
        assert!(states[0].cooldown_until.is_none());
        assert!(states[0].last_error.is_none());

        let mut states = vec![CandidateState {
            cooldown_until: Some(at(60)),
            cooldown_reason: Some(CooldownReason::Quota),
            last_error: None,
        }];
        on_result(&mut states, 0, &UpstreamOutcome::Success, policy(), Utc::now(), None);
        assert!(states[0].cooldown_until.is_some(), "quota cooldown survives");
    }

    #[test]
    fn server_error_cools_shortly() {
        let mut states = vec![CandidateState::default()];
        let now = Utc::now();
        on_result(&mut states, 0, &UpstreamOutcome::ServerError, policy(), now, Some("502"));
        let until = states[0].cooldown_until.unwrap();
        assert!(
            until - now <= chrono::Duration::seconds(SERVER_ERROR_COOLDOWN_SECS as i64)
        );
        assert!(until > now);
    }

    #[test]
    fn classify_status_maps_quotas_auth_and_server_errors() {
        assert!(classify_status(429).retryable());
        assert!(classify_status(402).retryable());
        assert!(classify_status(401).retryable());
        assert!(classify_status(403).retryable());
        assert!(classify_status(500).retryable());
        assert!(classify_status(529).retryable(), "anthropic overloaded");
        assert!(!classify_status(400).retryable());
        assert!(!classify_status(404).retryable());
        assert!(matches!(
            classify_status(429),
            UpstreamOutcome::QuotaError { retry_after_secs: None }
        ));
    }

    #[test]
    fn align_preserves_prefix_and_grows_with_defaults() {
        let mut rs = RouteState {
            last_used_index: Some(2),
            candidates: vec![
                CandidateState { cooldown_until: Some(at(60)), cooldown_reason: Some(CooldownReason::Quota), last_error: None },
                CandidateState::default(),
                CandidateState::default(),
            ],
        };
        rs.align(2);
        assert_eq!(rs.candidates.len(), 2);
        assert_eq!(rs.last_used_index, Some(2));
        rs.align(4);
        assert_eq!(rs.candidates.len(), 4);
        assert_eq!(rs.candidates[0].cooldown_reason, Some(CooldownReason::Quota));
        assert_eq!(rs.candidates[3], CandidateState::default());
    }

    #[test]
    fn combine_takes_the_tighter_view() {
        assert_eq!(
            QuotaView { remaining: Some(0.9) }.combine(QuotaView { remaining: Some(0.1) }),
            QuotaView { remaining: Some(0.1) }
        );
        assert_eq!(
            QuotaView { remaining: None }.combine(QuotaView { remaining: Some(0.4) }),
            QuotaView { remaining: Some(0.4) }
        );
        assert_eq!(
            QuotaView { remaining: None }.combine(QuotaView { remaining: None }),
            QuotaView { remaining: None }
        );
    }
}
