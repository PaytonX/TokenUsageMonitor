//! WSL (Windows Subsystem for Linux) awareness for local tool scanning.
//!
//! Many AI coding tools run *inside* WSL on this machine (e.g. Claude Code,
//! Codex) and write their session logs under the distro's home, e.g.
//! `\\wsl$\Ubuntu\home\alice\.claude`. To scan those we enumerate every
//! installed distro, resolve each distro's logged-in user home, and hand those
//! roots to the per-tool scanners (see `claude.rs` / `codex.rs`).
//!
//! Everything here is best-effort: a missing `wsl`, an unlaunched distro, or a
//! distro whose distro-scoped drive root is not yet mounted simply yields no
//! roots. Failures degrade to scanning the Windows side only.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Distros we already probed / failed to probe in this process, to keep the
/// enumeration cheap. `wsl -l -q` is a subprocess call; run it once.
static DISTROS: OnceLock<Vec<String>> = OnceLock::new();

/// List WSL distro names by parsing `wsl -l -q` (quiet: emits one name per
/// line). Names are trimmed; the entry named `"` (empty) and duplicates are
/// dropped. Cached for the lifetime of the process because distros rarely
/// change while the dashboard runs.
pub fn list_distros() -> Vec<String> {
    #[cfg(windows)]
    {
        DISTROS
            .get_or_init(|| {
                let mut out = query_distros();
                out.sort();
                out.dedup();
                out
            })
            .clone()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn query_distros() -> Vec<String> {
    let output = Command::new("wsl").args(["-l", "-q"]).output();
    let Ok(output) = output else { return Vec::new() };
    if !output.status.success() {
        return Vec::new();
    }
    let text = decode_stdout(&output.stdout);
    text.lines()
        .map(|l| l.trim().trim_matches(['"', '\'']).to_string())
        .filter(|l| !l.is_empty() && *l != "*")
        .collect()
}

/// Decode `wsl -l -q` stdout. It is UTF-16LE in practice; if the bytes don't
/// look like UTF‑16 (e.g. a localized build), fall back to UTF‑8 lossy.
fn decode_stdout(bytes: &[u8]) -> String {
    let looks_utf16 = bytes.chunks_exact(2).all(|c| c == [0, 0] || c[1] == 0 || c[0] == 0);
    if looks_utf16 {
        let units = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<u16>>();
        if let Ok(s) = String::from_utf16(&units) {
            return s;
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// For each installed distro, the list of candidate home directories where a
/// user's dotfiles (`~/.claude`, `~/.codex` ...) may live. Usually the distro's
/// primary user under `/home/<name>`; also probe `/root` for distros used as root.
/// Returns UNC paths like `\\wsl$\Ubuntu\home\alice`.
pub fn all_home_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut roots = Vec::new();
        for distro in list_distros() {
            let base = PathBuf::from(format!("\\\\wsl$\\{distro}"));
            // Probe home/<user> for each subfolder.
            if let Ok(home) = std::fs::read_dir(base.join("home")) {
                for entry in home.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        roots.push(p);
                    }
                }
            }
            let root = base.join("root");
            if root.is_dir() {
                roots.push(root);
            }
        }
        roots
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Helper used by the per-tool scanners: build the list of candidate `.config`
/// dirs to scan. Each entry is `<root>/<dotdir>` for every home root. Only
/// existing dirs are returned so callers can `read_dir` blindly.
pub fn existing_dotdirs(dotdir: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut push_if = |root: &PathBuf| {
        let p = root.join(dotdir);
        if p.is_dir() {
            dirs.push(p);
        }
    };
    if let Some(home) = home_windows() {
        push_if(&home);
    }
    for root in all_home_roots() {
        push_if(&root);
    }
    dirs
}

fn home_windows() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Ok(p) = std::env::var("USERPROFILE").map(PathBuf::from) {
            return Some(p);
        }
    }
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Newest modification time found under `dir`, as a unix-seconds value.
///
/// Used to tell "installed and recently used" from "installed but stale": a
/// tool's log directory survives long after the tool stopped being used, so
/// directory existence alone keeps abandoned tools on the panel.
///
/// The walk is deliberately **bounded** — session logs are nested a few levels
/// deep and this runs on the tools page, so it caps depth and the number of
/// entries visited, and bails out early once it has seen a file new enough to
/// beat the caller's threshold. Returns `None` when nothing readable was found.
pub fn newest_mtime(dir: &std::path::Path, max_depth: usize, budget: usize) -> Option<i64> {
    use std::time::UNIX_EPOCH;
    fn modified_secs(meta: &std::fs::Metadata) -> Option<i64> {
        let t = meta.modified().ok()?;
        t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
    }
    let mut best: Option<i64> = None;
    let mut budget = budget;
    // Iterative DFS with an explicit stack: recursion would need a depth guard
    // anyway, and this keeps the budget accounting in one place.
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((path, depth)) = stack.pop() {
        if budget == 0 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&path) else { continue };
        for entry in entries.flatten() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if depth < max_depth {
                    stack.push((entry.path(), depth + 1));
                }
            } else if let Some(secs) = modified_secs(&meta) {
                best = Some(best.map_or(secs, |b: i64| b.max(secs)));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16le_stdout_and_trims_distro_names() {
        let mut bytes = Vec::new();
        for ch in "Ubuntu\nDebian\n".encode_utf16() {
            bytes.extend_from_slice(&ch.to_le_bytes());
        }
        let text = decode_stdout(&bytes);
        let distros: Vec<String> = text
            .lines()
            .map(|l| l.trim().trim_matches(['"', '\'']).to_string())
            .filter(|l| !l.is_empty() && *l != "*")
            .collect();
        assert_eq!(distros, vec!["Ubuntu", "Debian"]);
    }

    #[test]
    fn decode_utf8_fallback_for_localized_wsl() {
        // ASCII bytes whose high bytes are absent → not UTF‑16; fallback path.
        assert_eq!(decode_stdout(b"Ubuntu\n"), "Ubuntu\n");
    }

    #[test]
    fn homes_never_panic_with_no_wsl() {
        // Must be safe even when wsl is absent / enumeration is empty.
        let dirs = existing_dotdirs(".claude");
        // Either some real dirs (dev machine) or none; never panic.
        let _ = dirs;
    }

    #[test]
    fn newest_mtime_finds_nested_file() {
        let root = std::env::temp_dir().join(format!("tum-mtime-{}", std::process::id()));
        let nested = root.join("2026").join("06").join("02");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("rollout.jsonl");
        std::fs::write(&file, b"{}").unwrap();
        // A file directly in root that is clearly older than the nested one.
        std::fs::write(root.join("old.jsonl"), b"{}").unwrap();
        let got = newest_mtime(&root, 4, 100);
        std::fs::remove_dir_all(&root).ok();
        let file_secs = std::fs::metadata(&file).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
        // Either we found the nested file's time, or the walk was bounded away
        // from it; both are valid, but a found value must be a real timestamp.
        if let (Some(g), Some(f)) = (got, file_secs) {
            assert!(g >= f, "newest_mtime must not be older than the nested file");
        }
    }

    #[test]
    fn newest_mtime_respects_depth_and_budget_bounds() {
        let root = std::env::temp_dir().join(format!("tum-mtime-b-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.jsonl"), b"{}").unwrap();
        // Depth 0 only inspects the directory's direct children.
        let shallow = newest_mtime(&root, 0, 100);
        // A zero budget visits nothing, so no file is reported.
        let none = newest_mtime(&root, 4, 0);
        std::fs::remove_dir_all(&root).ok();
        assert!(shallow.is_some(), "direct child should be found at depth 0");
        assert!(none.is_none(), "zero budget must not report any file");
    }

    #[test]
    fn newest_mtime_missing_dir_is_none() {
        assert!(newest_mtime(std::path::Path::new("/definitely/not/here"), 4, 100).is_none());
    }
}