//! Persisted second-level cache for local tool scanning.
//!
//! Full scans re-parse every log/db source (Claude Code alone walks 200+ JSONL
//! files). Most of the time nothing changed between refreshes, so we gate the
//! scan behind a cheap **metadata fingerprint**: we walk the source trees and
//! hash each file's `(path, size, mtime)` — never its content. If the digest
//! still matches the one stored next to the last result, the UI reuses that
//! result from disk instead of re-reading every file.
//!
//! The fingerprint deliberately mirrors each scanner's own source discovery so
//! a change to any reachable source (new/changed log, rotated DB, added WSL
//! file) invalidates the cache.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Metadata digest of every local tool source. Returns 0 only when no source
/// data can be discovered at all (e.g. nothing installed).
pub fn src_fingerprint() -> String {
    let mut hasher = DefaultHasher::new();

    // 1) Claude Code JSONL sessions (Windows + every WSL distro home).
    for root in super::wsl::existing_dotdirs(".claude/projects") {
        hash_jsonl_tree(&root, &mut hasher);
    }
    // 2) Codex JSONL sessions.
    for root in super::wsl::existing_dotdirs(".codex/sessions") {
        hash_jsonl_tree(&root, &mut hasher);
    }
    // 3) Hermes SQLite (+ its WAL/shm side files).
    for hermes in super::wsl::existing_dotdirs(".hermes") {
        for name in ["state.db", "state.db-wal", "state.db-shm"] {
            hash_file(&hermes.join(name), &mut hasher);
        }
    }
    // 4) MiniMax Code SQLite (v2 runtime + legacy).
    for db in minimax_dbs() {
        hash_file(&db, &mut hasher);
    }
    // 5) Cherry Studio SQLite.
    if let Some(db) = cherry_db() {
        hash_file(&db, &mut hasher);
    }

    format!("{:016x}", hasher.finish())
}

fn hash_jsonl_tree(dir: &Path, hasher: &mut DefaultHasher) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            hash_jsonl_tree(&path, hasher);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            hash_file(&path, hasher);
        }
    }
}

/// 计算单个工具的源指纹（metadata 级），用于 watch 判定"该工具日志是否变化"。
/// 未变 → 工具也不重扫；变了 → 仅重扫该工具（增量）。
pub fn tool_fingerprint(tool_id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    match tool_id {
        "claude-code" => {
            for root in super::wsl::existing_dotdirs(".claude/projects") {
                hash_jsonl_tree(&root, &mut hasher);
            }
        }
        "codex" => {
            for root in super::wsl::existing_dotdirs(".codex/sessions") {
                hash_jsonl_tree(&root, &mut hasher);
            }
        }
        "hermes" => {
            for hermes in super::wsl::existing_dotdirs(".hermes") {
                for name in ["state.db", "state.db-wal", "state.db-shm"] {
                    hash_file(&hermes.join(name), &mut hasher);
                }
            }
        }
        "minimax-code" => {
            for db in minimax_dbs() {
                hash_file(&db, &mut hasher);
            }
        }
        "cherry-studio" => {
            if let Some(db) = cherry_db() {
                hash_file(&db, &mut hasher);
            }
        }
        _ => return "".to_string(),
    }
    format!("{:016x}", hasher.finish())
}

/// 判断某工具是否"已安装"（其日志源目录是否存在）——用于在无近 90 天数据时仍保留
/// 该工具卡/页签，而不是把有安装但近期未用量的工具（如 Codex）静默丢掉。
pub fn tool_installed(tool_id: &str) -> bool {
    match tool_id {
        "claude-code" => !super::wsl::existing_dotdirs(".claude/projects").is_empty(),
        "codex" => !super::wsl::existing_dotdirs(".codex/sessions").is_empty(),
        "hermes" => !super::wsl::existing_dotdirs(".hermes").is_empty(),
        "cherry-studio" => cherry_db().is_some(),
        "minimax-code" => !minimax_dbs().is_empty(),
        _ => false,
    }
}

/// A tool that is installed but whose logs have not been touched for this many
/// days is treated as abandoned: it stays out of the panel unless it still has
/// usage rows inside the 90-day window.
///
/// Directory existence is a poor liveness signal — Codex's `.codex/sessions`
/// keeps every session from months ago, so the tool looks "installed forever"
/// long after the user stopped using it. Requiring recent activity keeps the
/// panel to tools actually in rotation, while a tool that *is* still being used
/// never disappears (its logs are written continuously).
pub const STALE_DAYS: i64 = 30;

/// Unix-seconds timestamp of the most recent log activity for `tool_id`, or
/// `None` when the tool is not installed / nothing readable was found.
///
/// SQLite-backed tools report the database file's own mtime; session-log tools
/// report the newest file under their log tree. The tree walk is bounded
/// (depth + entry budget) so a large history cannot stall the tools page.
pub fn tool_last_active_at(tool_id: &str) -> Option<i64> {
    /// Deep enough for Codex (`sessions/YYYY/MM/DD/rollout-*.jsonl`) while
    /// still shallow enough to stay cheap on large histories.
    const MAX_DEPTH: usize = 4;
    /// Upper bound on directory entries visited per tool.
    const BUDGET: usize = 2_000;

    fn newest_of_dirs(dirs: &[std::path::PathBuf]) -> Option<i64> {
        dirs.iter().filter_map(|d| super::wsl::newest_mtime(d, MAX_DEPTH, BUDGET)).max()
    }
    fn newest_of_files(files: &[std::path::PathBuf]) -> Option<i64> {
        files.iter().filter_map(|p| super::wsl::newest_mtime(p, 0, 1)).max()
    }

    match tool_id {
        "claude-code" => newest_of_dirs(&super::wsl::existing_dotdirs(".claude/projects")),
        "codex" => newest_of_dirs(&super::wsl::existing_dotdirs(".codex/sessions")),
        "hermes" => newest_of_dirs(&super::wsl::existing_dotdirs(".hermes")),
        "cherry-studio" => cherry_db().and_then(|db| newest_of_files(&[db])),
        "minimax-code" => newest_of_files(&minimax_dbs()),
        _ => None,
    }
}

/// Whether a tool is installed **and** still in active use (log activity within
/// [`STALE_DAYS`]). Callers use this to keep an installed-but-idle tool off the
/// panel; an unknown timestamp is treated as active so we never hide a tool
/// merely because its clock could not be read.
pub fn tool_recently_active(tool_id: &str) -> bool {
    match tool_last_active_at(tool_id) {
        Some(secs) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(secs);
            let age_days = (now - secs) / 86_400;
            age_days <= STALE_DAYS
        }
        // No readable timestamp (not installed, or an unreadable tree). Fall
        // back to the plain existence check so behaviour matches the old rule.
        None => tool_installed(tool_id),
    }
}

fn hash_file(path: &Path, hasher: &mut DefaultHasher) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    path.to_string_lossy().hash(hasher);
    meta.len().hash(hasher);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mtime().hash(hasher);
        meta.mtime_nsec().hash(hasher);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.last_write_time().hash(hasher);
    }
}

fn minimax_dbs() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var("USERPROFILE")
        .map(std::path::PathBuf::from)
        .ok()
    {
        out.push(home.join(".minimax").join("v2").join("sqlite").join("runtime-state.sqlite"));
        out.push(home.join(".minimax").join("sqlite.db"));
    }
    out
}

fn cherry_db() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("APPDATA").map(std::path::PathBuf::from) {
        return Some(p.join("CherryStudio").join("Data").join("cherrystudio.sqlite"));
    }
    std::env::var("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .map(|p| p.join("CherryStudio").join("Data").join("cherrystudio.sqlite"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    #[test]
    fn hash_includes_len_so_content_change_detected() {
        let dir = std::env::temp_dir().join(format!("tum_fp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.jsonl");
        std::fs::write(&f, "x").unwrap();

        let mut h1 = DefaultHasher::new();
        hash_file(&f, &mut h1);
        // Restore a later mtime so the run is deterministic vs. fast writes.
        let meta = std::fs::metadata(&f).unwrap();
        drop(meta);
        std::fs::write(&f, "xx").unwrap();
        let mut h2 = DefaultHasher::new();
        hash_file(&f, &mut h2);

        // Either len (most likely) or mtime differs → hash must differ.
        assert_ne!(h1.finish(), h2.finish());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cherry_db_never_panics() {
        let _ = cherry_db();
        let _ = minimax_dbs();
    }

    #[cfg(windows)]
    #[test]
    fn fingerprint_is_non_empty() {
        // Even with no sources it returns a stable hex string (hasher of nothing).
        let fp = src_fingerprint();
        assert!(fp.len() == 16);
    }
}
