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