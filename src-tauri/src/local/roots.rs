//! 本机工具数据目录的定位与重定位解析。
//!
//! 各 AI 工具默认把数据写在 `~/<点目录>`（`.claude` / `.zcode` / `.minimax` …），
//! 但用户常把点目录整体搬到别处（例如统一放到 `D:\Lab\.agentdata\`）。搬迁后
//! 旧位置往往**残留一个僵留副本**，而新位置仍在写入——扫描器若只认默认位置，
//! 就会读到停更的历史数据，表现为"最近几天用量为 0"。
//!
//! ## 为什么不做无配置自动发现
//!
//! 实测这些工具**都不自报数据目录**：没有环境变量、没有配置项、没有注册表键、
//! 快捷方式里也没有启动参数。任意路径的搬迁在程序看来不可知，因此发现必须
//! 有一条用户可声明的入口。
//!
//! ## 解析规则
//!
//! - **额外数据根目录**（设置项）：`D:\Lab\.agentdata` 一处即可覆盖其下所有
//!   点目录（`.minimax` / `.zcode` / `.cherrystudio` …），无需逐个工具填。
//! - **每工具精确覆盖**：直接指定该工具的目录，作为兜底。
//! - **新鲜度优先**：候选目录可能同时存在（默认位置 + 重定位位置），此时按
//!   目录内最新文件时间取**最近活跃**的那一个，而不是并集——并集会把僵留副本
//!   与活跃目录的同一天重复计数。
//! - **WSL 例外**：WSL 发行版里的 home 与 Windows home 是**不同机器**的数据，
//!   始终并集扫描，不做新鲜度互斥（见 [`crate::local::wsl`]）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// 走目录内文件时间比较时的遍历上限（深度 / 条目数）。仅用于挑选活跃目录，
/// 不需要看全树；与 `wsl::newest_mtime` 的边界一致。
const FRESHNESS_DEPTH: usize = 4;
const FRESHNESS_BUDGET: usize = 2_000;

#[derive(Debug, Default, Clone)]
pub struct ToolRootsConfig {
    /// 用户声明的额外数据根目录（其下的点目录会被自动命中）。
    pub extra: Vec<PathBuf>,
    /// 每工具精确目录覆盖，key 为工具标识（点目录名或 `cherry-studio`）。
    pub overrides: BTreeMap<String, PathBuf>,
}

static CONFIG: RwLock<Option<ToolRootsConfig>> = RwLock::new(None);

/// 进程内生效的数据根目录配置。启动与设置保存时写入，扫描器只读。
///
/// 用进程级快照而非逐层传递，是为了让所有扫描器的签名保持不变——它们分布
/// 在 `local/*` 与 `cache.rs`，其中若干没有 `AppState` 可用。设置极少变动，
/// 单进程内读取无竞争。
pub fn set_config(extra: Vec<PathBuf>, overrides: BTreeMap<String, PathBuf>) {
    if let Ok(mut guard) = CONFIG.write() {
        *guard = Some(ToolRootsConfig { extra, overrides });
    }
}

fn config() -> ToolRootsConfig {
    CONFIG
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

/// 已声明的额外数据根目录。
pub fn extra_roots() -> Vec<PathBuf> {
    config().extra
}

/// 某工具的精确目录覆盖。
pub fn override_for(key: &str) -> Option<PathBuf> {
    config().overrides.get(key).cloned()
}

/// 在 `candidates` 中挑出**最近活跃**的已存在目录；都不存在则 `None`。
///
/// 活跃度 = 目录内最新文件的修改时间（`None` 视为最不活跃，排在最后但仍可被
/// 选中——总比完全不扫好）。并列时取列表靠前者，保证结果确定。
pub fn pick_freshest(candidates: &[PathBuf]) -> Option<PathBuf> {
    let mut best: Option<(i64, PathBuf)> = None;
    for dir in candidates.iter().filter(|d| d.is_dir()) {
        let stamp = newest_mtime(dir).unwrap_or(i64::MIN);
        match &best {
            Some((b, _)) if *b >= stamp => {}
            _ => best = Some((stamp, dir.clone())),
        }
    }
    best.map(|(_, d)| d)
}

/// 目录内最新文件的修改时间（unix 秒）。
fn newest_mtime(dir: &Path) -> Option<i64> {
    super::wsl::newest_mtime(dir, FRESHNESS_DEPTH, FRESHNESS_BUDGET)
}

/// Windows 侧的候选目录集合：默认 home、每个额外根目录下的同名点目录、
/// 以及该工具的精确覆盖。去重后交由 [`pick_freshest`] 裁决。
pub fn windows_candidates(dotdir: &str, tool_key: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Ok(home) = std::env::var("USERPROFILE").map(PathBuf::from) {
        push(home.join(dotdir));
    }
    for root in extra_roots() {
        push(root.join(dotdir));
    }
    if let Some(o) = override_for(tool_key) {
        push(o);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("tum-roots-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 核心行为：默认位置与重定位位置并存时，取最近活跃的那个——
    /// 否则僵留副本会遮蔽活跃目录（近几日显示 0 的根因）。
    #[test]
    fn picks_the_more_recently_active_candidate() {
        let stale = tmp("stale");
        let live = tmp("live");
        std::fs::write(stale.join("a.jsonl"), b"{}").unwrap();
        // live 写一个明确更新的文件时间（+2h），避免依赖写入顺序。
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(7200);
        std::fs::File::create(live.join("a.jsonl"))
            .unwrap()
            .set_modified(future)
            .unwrap();
        let got = pick_freshest(&[stale.clone(), live.clone()]);
        assert_eq!(got, Some(live.clone()));
        let _ = std::fs::remove_dir_all(&stale);
        let _ = std::fs::remove_dir_all(&live);
    }

    #[test]
    fn missing_candidates_yield_none() {
        assert!(pick_freshest(&[PathBuf::from("/definitely/not/here/a")]).is_none());
        assert!(pick_freshest(&[]).is_none());
    }

    #[test]
    fn extra_roots_are_joined_with_the_dotdir() {
        let root = tmp("root");
        set_config(vec![root.clone()], BTreeMap::new());
        let cands = windows_candidates(".minimax", "minimax");
        assert!(cands.contains(&root.join(".minimax")));
        set_config(Vec::new(), BTreeMap::new());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn per_tool_override_is_included_and_deduped() {
        let root = tmp("oroot");
        let exact = tmp("oexact");
        let mut ov = BTreeMap::new();
        ov.insert(".minimax".to_string(), exact.clone());
        set_config(vec![root.clone()], ov);
        let cands = windows_candidates(".minimax", ".minimax");
        assert!(cands.contains(&exact), "exact override must be a candidate");
        // 覆盖项与根目录推导项相同时不重复。
        set_config(vec![exact.clone()], {
            let mut m = BTreeMap::new();
            m.insert(".minimax".to_string(), exact.join(".minimax"));
            m
        });
        let cands2 = windows_candidates(".minimax", ".minimax");
        let mut seen = cands2.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), cands2.len(), "candidates must be deduped");
        set_config(Vec::new(), BTreeMap::new());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&exact);
    }

    /// 并发读写不得 panic（设置保存与扫描会并发）。
    #[test]
    fn concurrent_set_and_read_is_safe() {
        let handles: Vec<_> = (0..4)
            .map(|i| {
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        set_config(vec![PathBuf::from(format!("D:/r{i}"))], BTreeMap::new());
                        let _ = extra_roots();
                        let _ = windows_candidates(".zcode", ".zcode");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        set_config(Vec::new(), BTreeMap::new());
    }
}
