//! TokenRouter 路由决策：四层判定的纯函数状态机。
//!
//! ① 主动预警（可设阈值，0 = 关闭）：以**最小（最短周期）在报窗口**的剩余 %
//!    为准——月总量剩 5% 但 5h 窗剩 98% 时不会误切；手填日限/月上限视为
//!    对应周期的窗口参与比较。
//! ② 硬墙兜底（恒开）：任一约束打满（100% / over）→ 立即不可用。
//! ③ 被动观测（恒开）：限流/配额错误立即冷却并换线路；连接异常原地重试、
//!    连续达到熔断次数才标记线路。永不基于推测切走。
//! ④ 切回探视：主线路「无窗口打满 + 最小窗口有余量」即具备资格；探针就是
//!    下一个真实请求；失败按 ×2 指数退避，达到最大次数后等窗口重置。
//!
//! 所有函数以 `now` 为参数注入时钟，测试无需 sleep。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 线路进入冷却的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CooldownReason {
    Quota,
    Auth,
    Error,
}

/// 单线路运行态（每路由一份，随配置对齐）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CandidateState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_reason: Option<CooldownReason>,
    /// 连接异常/5xx 的连续失败计数（成功清零；达到熔断阈值才进冷却）。
    #[serde(default)]
    pub consecutive_conn_errors: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// 切回探视运行态：主线路资格满足但实测仍失败时的退避记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeState {
    /// 已失败的探视次数。
    pub attempts: u32,
    /// 下一次允许探视主线路的时间（×2 退避；放弃后 = reset_at 或低频）。
    pub next_probe_at: DateTime<Utc>,
}

/// 每路由运行态。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteState {
    /// 最近一次实际承接请求的线路下标（UI 的「当前走 X」）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_index: Option<usize>,
    /// 主线路切回探视；None = 不在退避中（正常按链序选择）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<ProbeState>,
    pub candidates: Vec<CandidateState>,
}

impl RouteState {
    /// 线路数量随配置变化时就地扩缩：截断或以默认态补齐，保留前缀状态。
    /// 每次访问该路由运行态前调用一次，热更新配置即无需专门迁移。
    pub fn align(&mut self, len: usize) {
        self.candidates.resize(len, CandidateState::default());
    }
}

/// 一道「墙」：一个可能拦住请求的约束窗口。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraint {
    /// 周期序：0 = 最短（five_hour）< 1 daily < 2 weekly < 3 monthly。
    /// 主动预警只看周期最短者的剩余 %。
    pub period_rank: u8,
    /// 剩余比例 [0,1]；None = 该约束没有配额刻度（如 used-only 月窗）。
    pub remaining_percent: Option<f64>,
    /// 是否已打满（100% / over / used >= 手填上限）。
    pub full: bool,
}

/// 配额判定结果（由 [`assess`] 从约束集得出）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct QuotaVerdict {
    /// 最小（最短周期）约束的剩余%；同周期取更小者。None = 无任何刻度。
    pub min_window_remaining: Option<f64>,
    /// 硬墙：任一约束打满。
    pub hard_full: bool,
    /// 切回资格：没有打满的墙，且最小窗口有余量（>0 或无刻度）。
    pub can_probe: bool,
}

/// 汇总约束集 → 判定结果（② 硬墙 + ① 主动预警的输入）。
pub fn assess(constraints: &[Constraint]) -> QuotaVerdict {
    let mut min_rank = u8::MAX;
    let mut min_rem: Option<f64> = None;
    let mut hard_full = false;
    for c in constraints {
        if c.full {
            hard_full = true;
        }
        if c.period_rank < min_rank {
            min_rank = c.period_rank;
            min_rem = c.remaining_percent;
        } else if c.period_rank == min_rank {
            if let Some(r) = c.remaining_percent {
                min_rem = Some(match min_rem {
                    Some(m) => m.min(r),
                    None => r,
                });
            }
        }
    }
    let can_probe = !hard_full && !min_rem.is_some_and(|r| r <= 0.0);
    QuotaVerdict {
        min_window_remaining: min_rem,
        hard_full,
        can_probe,
    }
}

/// 决策参数（`Settings.router` 的相关快照）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecisionPolicy {
    /// 主动预警阈值（最小窗口剩余 %，0 = 关闭主动预警）。
    pub proactive_threshold_percent: u8,
    /// 限流/配额错误的冷却秒数（上游 Retry-After 优先，由服务器层填入）。
    pub error_cooldown_secs: u64,
    /// 连接异常熔断阈值（连续 N 次）。
    pub conn_breaker_count: u32,
    /// 切回探视起始间隔（秒）。
    pub probe_start_secs: u64,
    /// 切回探视最大次数。
    pub probe_max_attempts: u32,
}

impl DecisionPolicy {
    pub fn from_settings(s: &crate::router::config::RouterSettings) -> Self {
        Self {
            proactive_threshold_percent: s.proactive_threshold_percent,
            error_cooldown_secs: s.error_cooldown_secs,
            conn_breaker_count: s.conn_breaker_count,
            probe_start_secs: s.probe_start_secs,
            probe_max_attempts: s.probe_max_attempts,
        }
    }
}

/// 连接类熔断进入的短冷却时长（远短于配额冷却）。
pub const CONN_BREAKER_COOLDOWN_SECS: u64 = 60;
/// 拿不到 reset_at 时的低频探视频率。
pub const PROBE_FALLBACK_INTERVAL_SECS: u64 = 900;

/// 单线路解析后的决策输入（快照读取是异步的，须在 pick 之前完成）。
pub struct ResolvedCandidate<'a> {
    pub config: &'a crate::router::config::CandidateConfig,
    pub quota: QuotaVerdict,
    /// 凭据可用（BearerKey 且非空）。false 的线路连降级都不参与。
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
    /// 是否应当换下一条线路重试（连接类在服务器层还有一次原地重试机会）。
    pub fn retryable(&self) -> bool {
        !matches!(self, UpstreamOutcome::Success | UpstreamOutcome::ClientError)
    }
}

/// HTTP 状态码 → 结果分类（Retry-After 由服务器层补充解析）。
///
/// 404 归为连接类（ServerError）：模型/路径在某条线路上不存在是该**线路自己
/// 的配置问题**（base_url 版本段拼错、线路模型名上游没有），换下一条线路正是
/// 正确处理；走熔断计数而非配额长冷却——用户修好配置后 60 秒内即可恢复，
/// 而不是白等 5 分钟的配额冷却。
pub fn classify_status(status: u16) -> UpstreamOutcome {
    match status {
        429 | 402 => UpstreamOutcome::QuotaError {
            retry_after_secs: None,
        },
        401 | 403 => UpstreamOutcome::AuthError,
        404 => UpstreamOutcome::ServerError,
        s if (500..600).contains(&s) => UpstreamOutcome::ServerError,
        _ => UpstreamOutcome::ClientError,
    }
}

fn is_cooling(state: &CandidateState, now: DateTime<Utc>) -> bool {
    state.cooldown_until.is_some_and(|until| until > now)
}

/// ③+①+②：线路此刻是否可承接新请求（探视放行由 pick 对 index 0 特判）。
pub fn available(
    state: &CandidateState,
    quota: &QuotaVerdict,
    policy: &DecisionPolicy,
    now: DateTime<Utc>,
) -> bool {
    if is_cooling(state, now) {
        return false;
    }
    // ② 硬墙兜底：任一窗口打满 → 立即不可用。
    if quota.hard_full {
        return false;
    }
    // ① 主动预警：最小窗口剩余 % 低于阈值 → 新请求绕开。无刻度数据不拦截。
    if policy.proactive_threshold_percent > 0 {
        if let Some(r) = quota.min_window_remaining {
            if r * 100.0 < policy.proactive_threshold_percent as f64 {
                return false;
            }
        }
    }
    true
}

/// 选出本次请求的线路下标：链序第一个可用者；无则第一个未冷却且凭据可用者。
///
/// `index 0` 特判（④ 切回探视）：主线路处于探视退避中（`probe` 存在）时，
/// 只有到期（`now >= next_probe_at`）才允许下一个真实请求作为探针访问它；
/// 未到期则连降级路径也跳过主线路——退避的意义就是别再撞。
pub fn pick_candidate(
    candidates: &[ResolvedCandidate<'_>],
    states: &[CandidateState],
    route_state: Option<&RouteState>,
    policy: &DecisionPolicy,
    now: DateTime<Utc>,
) -> Option<usize> {
    let state_of = |i: usize| states.get(i).cloned().unwrap_or_default();
    let probe = route_state.and_then(|r| r.probe.as_ref());
    let mut fallback = None;
    for (i, c) in candidates.iter().enumerate() {
        if !c.usable {
            continue;
        }
        let st = state_of(i);
        // 探视退避中的主线路：到期才放行（作为探针），未到期连降级都不参与。
        if i == 0 {
            if let Some(p) = probe {
                if now < p.next_probe_at {
                    continue;
                }
                // 到期：只要没打满就放行一次（探针；资格不满足时服务器层会清探视态）。
                if !c.quota.hard_full {
                    return Some(0);
                }
                continue;
            }
        }
        if available(&st, &c.quota, policy, now) {
            return Some(i);
        }
        if fallback.is_none() && !is_cooling(&st, now) {
            fallback = Some(i);
        }
    }
    fallback
}

/// 一次上游结果对运行态的影响。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeEffect {
    None,
    /// 该线路进入了冷却（值得关注的状态变化）。
    EnteredCooldown,
    /// Error 冷却被成功清除。
    ClearedCooldown,
}

/// 把一次上游结果写回线路运行态（③ 被动观测）。
pub fn on_result(
    states: &mut [CandidateState],
    index: usize,
    outcome: &UpstreamOutcome,
    policy: &DecisionPolicy,
    now: DateTime<Utc>,
    message: Option<&str>,
) -> OutcomeEffect {
    let Some(st) = states.get_mut(index) else {
        return OutcomeEffect::None;
    };
    match outcome {
        UpstreamOutcome::Success => {
            st.last_error = None;
            st.consecutive_conn_errors = 0;
            if st.cooldown_reason == Some(CooldownReason::Error) {
                st.cooldown_until = None;
                st.cooldown_reason = None;
                return OutcomeEffect::ClearedCooldown;
            }
            OutcomeEffect::None
        }
        UpstreamOutcome::QuotaError { retry_after_secs } => {
            // 限流/配额不会毫秒级恢复：立即冷却，不原地重试。
            let secs = retry_after_secs.unwrap_or(policy.error_cooldown_secs).max(1);
            st.cooldown_until = Some(now + chrono::Duration::seconds(secs as i64));
            st.cooldown_reason = Some(CooldownReason::Quota);
            if let Some(m) = message {
                st.last_error = Some(m.to_string());
            }
            OutcomeEffect::EnteredCooldown
        }
        UpstreamOutcome::AuthError => {
            st.cooldown_until =
                Some(now + chrono::Duration::seconds(policy.error_cooldown_secs as i64));
            st.cooldown_reason = Some(CooldownReason::Auth);
            if let Some(m) = message {
                st.last_error = Some(m.to_string());
            }
            OutcomeEffect::EnteredCooldown
        }
        UpstreamOutcome::ServerError => {
            // 连接类：原地重试由服务器层处理；这里只做跨请求熔断计数。
            st.consecutive_conn_errors = st.consecutive_conn_errors.saturating_add(1);
            if let Some(m) = message {
                st.last_error = Some(m.to_string());
            }
            if st.consecutive_conn_errors >= policy.conn_breaker_count.max(1) {
                st.cooldown_until = Some(
                    now + chrono::Duration::seconds(CONN_BREAKER_COOLDOWN_SECS as i64),
                );
                st.cooldown_reason = Some(CooldownReason::Error);
                OutcomeEffect::EnteredCooldown
            } else {
                OutcomeEffect::None
            }
        }
        // 参数类 4xx 是请求自己的问题，与线路健康无关。
        UpstreamOutcome::ClientError => OutcomeEffect::None,
    }
}

/// 探视失败后的退避推进（④）：next = now + start × 2^(attempts-1)；
/// 达到最大次数后由服务器层改写为 reset_at（或低频兜底间隔）。
pub fn advance_probe(probe: &mut ProbeState, policy: &DecisionPolicy, now: DateTime<Utc>) {
    probe.attempts = probe.attempts.saturating_add(1);
    let shift = probe.attempts.saturating_sub(1).min(16);
    let secs = policy
        .probe_start_secs
        .max(1)
        .saturating_mul(1u64 << shift);
    probe.next_probe_at = now + chrono::Duration::seconds(secs as i64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::config::{CandidateConfig, RouterSettings};

    fn policy() -> DecisionPolicy {
        DecisionPolicy {
            proactive_threshold_percent: 20,
            error_cooldown_secs: 300,
            conn_breaker_count: 3,
            probe_start_secs: 60,
            probe_max_attempts: 5,
        }
    }

    fn cand(account: &str) -> CandidateConfig {
        CandidateConfig {
            account: account.to_string(),
            model: format!("model-{account}"),
            base_url: "https://upstream.test".to_string(),
            plan_limit_tokens_daily: None,
            monthly_cost_limit: None,
        }
    }

    fn resolved<'a>(config: &'a CandidateConfig, remaining: Option<f64>) -> ResolvedCandidate<'a> {
        ResolvedCandidate {
            config,
            quota: QuotaVerdict {
                min_window_remaining: remaining,
                hard_full: false,
                can_probe: true,
            },
            usable: true,
        }
    }

    /// 判定用时把策略与运行态按引用传入（与服务器层一致）。
    fn pick<'a>(
        cands: &[ResolvedCandidate<'a>],
        states: &[CandidateState],
        rs: Option<&RouteState>,
        policy: &DecisionPolicy,
        now: DateTime<Utc>,
    ) -> Option<usize> {
        pick_candidate(cands, states, rs, policy, now)
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc::now() + chrono::Duration::seconds(secs)
    }

    // ---- assess ----

    fn c(rank: u8, remaining: Option<f64>, full: bool) -> Constraint {
        Constraint {
            period_rank: rank,
            remaining_percent: remaining,
            full,
        }
    }

    #[test]
    fn assess_picks_shortest_period_window() {
        // 月窗剩 5%（rank 3），5h 窗剩 98%（rank 0）→ 最小窗口是 5h 窗 → 98%。
        let v = assess(&[c(3, Some(0.05), false), c(0, Some(0.98), false)]);
        assert_eq!(v.min_window_remaining, Some(0.98));
        assert!(!v.hard_full);
        assert!(v.can_probe);
    }

    #[test]
    fn assess_hard_full_when_any_window_full_even_if_short_window_is_fine() {
        let v = assess(&[c(3, Some(0.0), true), c(0, Some(0.98), false)]);
        assert!(v.hard_full);
        assert!(!v.can_probe, "有墙打满即无切回资格");
    }

    #[test]
    fn assess_same_period_takes_the_tighter() {
        // 同为 daily：手填日限剩 10% 与 provider 日窗剩 30% → 取 10%。
        let v = assess(&[c(1, Some(0.30), false), c(1, Some(0.10), false)]);
        assert_eq!(v.min_window_remaining, Some(0.10));
    }

    #[test]
    fn assess_no_scale_data_yields_none_but_not_full() {
        // used-only 月窗（quota=0）：无刻度、不打满。
        let v = assess(&[c(3, None, false)]);
        assert_eq!(v.min_window_remaining, None);
        assert!(!v.hard_full);
        assert!(v.can_probe, "无刻度视为有余量");
    }

    #[test]
    fn assess_min_window_at_zero_loses_probe_eligibility() {
        let v = assess(&[c(0, Some(0.0), false)]);
        assert!(!v.can_probe, "最小窗口余量为 0 → 无切回资格");
        assert!(!v.hard_full, "余量 0 但未 over → 不是硬墙");
    }

    // ---- pick ----

    #[test]
    fn picks_chain_head_when_all_available() {
        let configs = [cand("a"), cand("b"), cand("c")];
        let cands = [
            resolved(&configs[0], Some(0.9)),
            resolved(&configs[1], Some(0.8)),
            resolved(&configs[2], Some(0.7)),
        ];
        assert_eq!(pick(&cands, &[], None, &policy(), Utc::now()), Some(0));
    }

    #[test]
    fn proactive_threshold_skips_low_min_window() {
        let configs = [cand("a"), cand("b")];
        let cands = [
            resolved(&configs[0], Some(0.15)),
            resolved(&configs[1], Some(0.9)),
        ];
        assert_eq!(pick(&cands, &[], None, &policy(), Utc::now()), Some(1));
        // 阈值设 0 = 关闭主动预警：低余量主线路仍然承接。
        let mut p = policy();
        p.proactive_threshold_percent = 0;
        assert_eq!(pick(&cands, &[], None, &p, Utc::now()), Some(0));
    }

    #[test]
    fn hard_full_blocks_even_when_threshold_disabled() {
        let configs = [cand("a"), cand("b")];
        let mut c0 = resolved(&configs[0], None);
        c0.quota.hard_full = true;
        let cands = [c0, resolved(&configs[1], Some(0.9))];
        let mut p = policy();
        p.proactive_threshold_percent = 0;
        assert_eq!(pick(&cands, &[], None, &p, Utc::now()), Some(1));
    }

    #[test]
    fn missing_quota_data_does_not_block() {
        let configs = [cand("a")];
        let cands = [resolved(&configs[0], None)];
        assert_eq!(pick(&cands, &[], None, &policy(), Utc::now()), Some(0));
    }

    #[test]
    fn unusable_candidates_are_skipped_entirely() {
        let configs = [cand("a"), cand("b")];
        let mut c = resolved(&configs[0], Some(1.0));
        c.usable = false;
        let cands = [c, resolved(&configs[1], Some(0.9))];
        assert_eq!(pick(&cands, &[], None, &policy(), Utc::now()), Some(1));
    }

    #[test]
    fn all_cooling_falls_back_to_non_cooling_candidate() {
        let configs = [cand("a"), cand("b")];
        let cooling = |until| CandidateState {
            cooldown_until: Some(until),
            cooldown_reason: Some(CooldownReason::Quota),
            consecutive_conn_errors: 0,
            last_error: None,
        };
        let states = vec![cooling(at(60)), cooling(at(60))];
        let cands = [
            resolved(&configs[0], Some(0.5)),
            resolved(&configs[1], Some(0.5)),
        ];
        assert_eq!(pick(&cands, &states, None, &policy(), Utc::now()), None);
        let states = vec![CandidateState::default(), cooling(at(60))];
        assert_eq!(pick(&cands, &states, None, &policy(), Utc::now()), Some(0));
    }

    // ---- probe（④）----

    fn probe_wait(secs: i64) -> RouteState {
        RouteState {
            last_used_index: Some(1),
            probe: Some(ProbeState {
                attempts: 1,
                next_probe_at: at(secs),
            }),
            candidates: vec![CandidateState::default(); 2],
        }
    }

    #[test]
    fn probe_backoff_blocks_main_line_until_due() {
        let configs = [cand("a"), cand("b")];
        let cands = [
            resolved(&configs[0], Some(0.9)),
            resolved(&configs[1], Some(0.5)),
        ];
        let rs = probe_wait(60);
        // 未到期：主线路被退避跳过（连降级都不撞它）。
        assert_eq!(pick(&cands, &rs.candidates, Some(&rs), &policy(), Utc::now()), Some(1));
        // 到期：主线路作为探针放行。
        assert_eq!(pick(&cands, &rs.candidates, Some(&rs), &policy(), at(120)), Some(0));
    }

    #[test]
    fn probe_is_cleared_when_eligibility_lost() {
        // 资格丢失（打满）→ 服务器层会清探视态；这里验证清掉后正常判定。
        let configs = [cand("a"), cand("b")];
        let mut c0 = resolved(&configs[0], Some(0.9));
        c0.quota.hard_full = true;
        let cands = [c0, resolved(&configs[1], Some(0.5))];
        let rs = probe_wait(60);
        assert_eq!(
            pick(&cands, &rs.candidates, Some(&rs), &policy(), at(120)),
            Some(1),
            "打满的主线路即使探视到期也不放行"
        );
    }

    #[test]
    fn advance_probe_doubles_the_interval() {
        let mut p = ProbeState {
            attempts: 0,
            next_probe_at: Utc::now(),
        };
        let now = Utc::now();
        let pol = policy();
        advance_probe(&mut p, &pol, now);
        assert_eq!(p.attempts, 1);
        assert_eq!((p.next_probe_at - now).num_seconds(), 60, "第 1 次失败 → 起始间隔");
        let pol = policy();
        advance_probe(&mut p, &pol, now);
        assert_eq!((p.next_probe_at - now).num_seconds(), 120, "×2");
        advance_probe(&mut p, &pol, now);
        assert_eq!((p.next_probe_at - now).num_seconds(), 240, "×4");
    }

    // ---- on_result ----

    #[test]
    fn quota_error_cools_with_retry_after() {
        let mut states = vec![CandidateState::default()];
        let p = policy();
        let eff = on_result(
            &mut states,
            0,
            &UpstreamOutcome::QuotaError {
                retry_after_secs: Some(10),
            },
            &p,
            Utc::now(),
            Some("429"),
        );
        assert_eq!(eff, OutcomeEffect::EnteredCooldown);
        assert_eq!(states[0].cooldown_reason, Some(CooldownReason::Quota));
        let until = states[0].cooldown_until.unwrap();
        assert!(until <= at(15), "Retry-After 优先于默认冷却");
    }

    #[test]
    fn server_error_requires_breaker_count_before_cooldown() {
        let mut states = vec![CandidateState::default(); 1];
        let p = policy();
        let now = Utc::now();
        // 前 2 次：只计数，不冷却（单次/偶发失败不杀线路）。
        for _ in 0..2 {
            let eff = on_result(&mut states, 0, &UpstreamOutcome::ServerError, &p, now, Some("err"));
            assert_eq!(eff, OutcomeEffect::None);
            assert!(states[0].cooldown_until.is_none());
        }
        // 第 3 次（达到熔断阈值）：短冷却。
        let eff = on_result(&mut states, 0, &UpstreamOutcome::ServerError, &p, now, Some("err"));
        assert_eq!(eff, OutcomeEffect::EnteredCooldown);
        assert_eq!(states[0].cooldown_reason, Some(CooldownReason::Error));
        let until = states[0].cooldown_until.unwrap();
        assert_eq!(
            (until - now).num_seconds(),
            CONN_BREAKER_COOLDOWN_SECS as i64
        );
    }

    #[test]
    fn success_resets_conn_errors_and_clears_error_cooldown() {
        let mut states = vec![CandidateState {
            cooldown_until: Some(at(60)),
            cooldown_reason: Some(CooldownReason::Error),
            consecutive_conn_errors: 2,
            last_error: Some("boom".into()),
        }];
        on_result(&mut states, 0, &UpstreamOutcome::Success, &policy(), Utc::now(), None);
        assert!(states[0].cooldown_until.is_none());
        assert_eq!(states[0].consecutive_conn_errors, 0);
        assert!(states[0].last_error.is_none());
        // Quota 冷却不被成功清除（恢复由资格判定/冷却到期决定）。
        let mut states = vec![CandidateState {
            cooldown_until: Some(at(60)),
            cooldown_reason: Some(CooldownReason::Quota),
            consecutive_conn_errors: 0,
            last_error: None,
        }];
        on_result(&mut states, 0, &UpstreamOutcome::Success, &policy(), Utc::now(), None);
        assert!(states[0].cooldown_until.is_some());
    }

    #[test]
    fn auth_error_cools_down() {
        let mut states = vec![CandidateState::default()];
        let eff = on_result(
            &mut states,
            0,
            &UpstreamOutcome::AuthError,
            &policy(),
            Utc::now(),
            Some("401"),
        );
        assert_eq!(eff, OutcomeEffect::EnteredCooldown);
        assert_eq!(states[0].cooldown_reason, Some(CooldownReason::Auth));
    }

    #[test]
    fn classify_status_maps_error_classes() {
        assert!(classify_status(429).retryable());
        assert!(classify_status(402).retryable());
        assert!(classify_status(401).retryable());
        assert!(classify_status(403).retryable());
        assert!(classify_status(500).retryable());
        assert!(classify_status(529).retryable(), "anthropic overloaded");
        assert!(!classify_status(400).retryable());
        // 404 = 路径/模型在该线路上不存在 → 线路配置问题，应换线而非透传。
        assert!(classify_status(404).retryable());
        assert!(matches!(classify_status(404), UpstreamOutcome::ServerError));
        assert!(matches!(
            classify_status(429),
            UpstreamOutcome::QuotaError { retry_after_secs: None }
        ));
    }

    #[test]
    fn align_preserves_prefix_and_grows_with_defaults() {
        let mut rs = RouteState {
            last_used_index: Some(2),
            probe: None,
            candidates: vec![
                CandidateState {
                    cooldown_until: Some(at(60)),
                    cooldown_reason: Some(CooldownReason::Quota),
                    consecutive_conn_errors: 0,
                    last_error: None,
                },
                CandidateState::default(),
                CandidateState::default(),
            ],
        };
        rs.align(2);
        assert_eq!(rs.candidates.len(), 2);
        rs.align(4);
        assert_eq!(rs.candidates.len(), 4);
        assert_eq!(rs.candidates[0].cooldown_reason, Some(CooldownReason::Quota));
        assert_eq!(rs.candidates[3], CandidateState::default());
    }

    #[test]
    fn decision_policy_reads_settings() {
        let mut s = crate::router::config::RouterSettings::default();
        s.proactive_threshold_percent = 30;
        let p = DecisionPolicy::from_settings(&s);
        assert_eq!(p.proactive_threshold_percent, 30);
        assert_eq!(p.error_cooldown_secs, 300);
        assert_eq!(p.conn_breaker_count, 3);
    }
}
