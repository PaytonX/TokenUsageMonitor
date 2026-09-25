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
}