//! 字节级 offset 续读增量扫描（jsonl 追加型工具的 watch 加速）。
//!
//! claude/codex 的日志是**按会话的 jsonl 文件**，写入方会不断**往文件尾部追加**完整
//! JSON 行。watch 判定某工具日志变化后，以往会整工具全量重扫（每个文件整读、逐行走）。
//! 这里改为：对一个文件比对 `len` 与存储的续读指针 `offset`——
//!
//! - `len == offset` 且 mtime 未变 → 该文件没有新字节，跳过；
//! - `len >  offset`（尾部追加）→ 从 `offset` 处 seek 续读**新增完整行**，只解析新增；
//! - `len <= offset` 或 mtime 变但长度未增（轮转/原地改写）→ 无法增量，回退整工具全量。
//!
//! 新增行只影响"最近一天"的用量：把增量合并进缓存中该工具的今日 `LocalDay`（以及对应
//! 模型的今日 `LocalDay`），历史由全量缓存兜底。这样秒级轮询时不再反复解析几百个历史文件。

use super::{LocalModelUsage, LocalToolReport, SharedLocalCache, UNCLASSIFIED_MODEL};
use crate::storage::Storage;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 当前支持字节级续读的 jsonl 追加型工具。
pub fn is_jsonl_tool(tool_id: &str) -> bool {
    matches!(tool_id, "claude-code" | "codex")
}

/// 返回某 jsonl 工具的所有 session `*.jsonl` 文件（与 claude/codex 扫描同源发现）。
fn jsonl_files(tool_id: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match tool_id {
        "claude-code" => {
            for root in super::wsl::existing_dotdirs(".claude/projects") {
                walk_jsonl(&root, &mut out);
            }
        }
        "codex" => {
            for root in super::wsl::existing_dotdirs(".codex/sessions") {
                walk_jsonl(&root, &mut out);
            }
        }
        _ => {}
    }
    out
}

fn walk_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_jsonl(&p, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
}

/// 按工具选择行解析器（与各自模块同一份逻辑，避免口径分叉）。
fn parse_line(tool_id: &str, line: &str) -> Option<(f64, f64, f64, String, String)> {
    match tool_id {
        "claude-code" => super::claude::parse_assistant(line),
        "codex" => super::codex::parse_line(line),
        _ => None,
    }
}

/// 把一行增量并入 `by_day` / `by_model` 桶（空日期、零用量由解析器自行筛掉）。
fn ingest_line(
    tool_id: &str,
    line: &str,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_model: &mut BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
) {
    let Some((input, cache_read, output, date, model)) = parse_line(tool_id, line) else {
        return;
    };
    if date.is_empty() {
        return;
    }
    let e = by_day.entry(date.clone()).or_insert((0.0, 0.0, 0.0));
    e.0 += input;
    e.1 += cache_read;
    e.2 += output;
    // 无模型的行归入"未标记模型"桶，保证按日总量与按模型汇总一致。
    let model = if model.is_empty() {
        UNCLASSIFIED_MODEL.to_string()
    } else {
        model
    };
    let m = by_model.entry(model).or_default();
    let de = m.entry(date).or_insert((0.0, 0.0, 0.0));
    de.0 += input;
    de.1 += cache_read;
    de.2 += output;
}

/// 从 `offset` 处 seek 续读一个 jsonl 文件的**新增**字节，解析其中所有完整行，并入桶。
/// 返回新的续读指针：停在最后一个完整行末尾（尾部不完整的一行不计数、留待下次补齐），
/// 避免读到半行导致 JSON 解析失败或重复计数。
fn read_appended(
    path: &Path,
    offset: u64,
    tool_id: &str,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_model: &mut BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
) -> u64 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return offset;
    };
    let Ok(cur_len) = file.metadata().map(|m| m.len()) else {
        return offset;
    };
    if cur_len <= offset {
        return offset;
    }
    let _ = file.seek(SeekFrom::Start(offset));
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return offset;
    }
    let last_nl = buf.iter().rposition(|&b| b == b'\n');
    let complete = last_nl.map(|i| i + 1).unwrap_or(0);
    let new_offset = offset + complete as u64;
    for line in buf[..complete].split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(s) = std::str::from_utf8(line) {
            ingest_line(tool_id, s, by_day, by_model);
        }
    }
    new_offset
}

/// 全量解析一个 jsonl 文件（用于"新出现的文件"或回退），并入桶。
fn scan_file_into(
    path: &Path,
    tool_id: &str,
    by_day: &mut BTreeMap<String, (f64, f64, f64)>,
    by_model: &mut BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
) {
    let Ok(bytes) = std::fs::read(path) else { return };
    for line in bytes.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(s) = std::str::from_utf8(line) {
            ingest_line(tool_id, s, by_day, by_model);
        }
    }
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn last_write_time(path: &Path) -> i64 {
    std::fs::metadata(path)
        .map(|m| {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                m.last_write_time() as i64
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                m.mtime()
            }
        })
        .unwrap_or(0)
}

/// 在 `report.daily` 中按日期累加（不存在则新增一行）。
fn add_to_daily(daily: &mut Vec<super::LocalDay>, date: &str, input: f64, cache_read: f64, output: f64) {
    if let Some(d) = daily.iter_mut().find(|d| d.date == *date) {
        d.add(input, cache_read, output);
    } else {
        let mut nd = super::LocalDay {
            date: date.to_string(),
            ..Default::default()
        };
        nd.add(input, cache_read, output);
        daily.push(nd);
    }
}

/// 把增量桶并入工具报告（每日 + 每模型），并对做成本估算的模型补算成本。
fn merge_into_report(
    report: &mut LocalToolReport,
    by_day: &BTreeMap<String, (f64, f64, f64)>,
    by_model: &BTreeMap<String, BTreeMap<String, (f64, f64, f64)>>,
) {
    for (date, (i, cr, o)) in by_day {
        add_to_daily(&mut report.daily, date, *i, *cr, *o);
    }
    report.daily.sort_by(|a, b| a.date.cmp(&b.date));
    report.total_tokens = report.daily.iter().map(|d| d.total).sum();

    for (model, days) in by_model {
        let usage = if let Some(u) = report.models.iter_mut().find(|u| u.model == *model) {
            u
        } else {
            report.models.push(LocalModelUsage {
                model: model.clone(),
                ..Default::default()
            });
            report.models.last_mut().unwrap()
        };
        for (date, (i, cr, o)) in days {
            add_to_daily(&mut usage.daily, date, *i, *cr, *o);
        }
        usage.daily.sort_by(|a, b| a.date.cmp(&b.date));
        usage.total_tokens = usage.daily.iter().map(|d| d.total).sum();
    }
    report
        .models
        .sort_by(|a, b| b.total_tokens.partial_cmp(&a.total_tokens).unwrap_or(std::cmp::Ordering::Equal));

    estimate_model_costs(&mut report.models);
}

/// 对已知价目表、当前成本估算为 0 的模型补算成本（与 claude/codex 全量扫描同口径）。
fn estimate_model_costs(models: &mut [LocalModelUsage]) {
    for m in models {
        if m.cost > 0.0 {
            continue;
        }
        let input: f64 = m.daily.iter().map(|d| d.input + d.cache_read).sum();
        let output: f64 = m.daily.iter().map(|d| d.output).sum();
        if let Some(cost) = crate::pricing::compute_cost(&crate::providers::TokenBreakdown {
            input,
            cache_read: 0.0,
            output,
            model_id: Some(m.model.clone()),
        }) {
            if cost > 0.0 {
                m.cost = cost;
                m.currency = "USD".to_string();
                m.cost_estimated = true;
            }
        }
    }
}

/// 为某工具的所有 jsonl 文件重建续读指针（offset=当前长度，视日志为追加型）。
/// 在全量重扫（watch 回退 / scan_all）之后调用，保证下一次 watch 增量从新增处读，不重复计数。
pub fn record_tool_offsets(storage: &Storage, tool_id: &str) {
    if !is_jsonl_tool(tool_id) {
        return;
    }
    for path in jsonl_files(tool_id) {
        let len = file_len(&path);
        let mt = last_write_time(&path);
        let _ = storage.upsert_tool_file_offset(
            tool_id,
            &path.to_string_lossy().into_owned(),
            len,
            len,
            mt,
        );
    }
}

/// watch 路径入口：对 jsonl 工具做增量续读并合并进缓存里该工具的今日用量。
///
/// - 缓存无该工具报告 → 全量扫描一次并重建 offset；
/// - 逐个文件增量续读，出现轮转/缩容/原地改写 → 回退整工具全量并重建 offset。
pub async fn refresh_jsonl_tool(
    local: &SharedLocalCache,
    storage: &Storage,
    tool_id: &str,
) -> Option<LocalToolReport> {
    if !is_jsonl_tool(tool_id) {
        return super::scan_tool(tool_id, storage);
    }
    let cached = local.cached().await;
    let base = cached.and_then(|p| p.tools.into_iter().find(|t| t.id == tool_id));
    let Some(mut report) = base else {
        record_tool_offsets(storage, tool_id);
        return super::scan_tool(tool_id, storage);
    };

    let files = jsonl_files(tool_id);
    let mut by_day: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_model: BTreeMap<String, BTreeMap<String, (f64, f64, f64)>> = BTreeMap::new();
    let mut fallback_full = false;

    for path in &files {
        let cur_len = file_len(path);
        let cur_mtime = last_write_time(path);
        let path_str = path.to_string_lossy().into_owned();
        let stored = storage.get_tool_file_offset(tool_id, &path_str).ok().flatten();

        match stored {
            None => {
                // 新出现的文件：全量解析（其用量基本都是当次会话的今日新增）。
                scan_file_into(path, tool_id, &mut by_day, &mut by_model);
                let _ = storage.upsert_tool_file_offset(
                    tool_id, &path_str, cur_len, cur_len, cur_mtime,
                );
            }
            Some((offset, _len, rec_mtime)) => {
                if cur_len < offset {
                    // 文件被缩短（轮转/重建）→ 无法增量，整工具回退全量。
                    fallback_full = true;
                    break;
                }
                if cur_len == offset {
                    // 长度未变：mtime 也一致则纯粹未变；mtime 变了说明原地改写 → 回退。
                    if cur_mtime != rec_mtime {
                        fallback_full = true;
                        break;
                    }
                    continue;
                }
                // cur_len > offset：尾部追加 → 只续读新增行。
                let new_offset = read_appended(path, offset, tool_id, &mut by_day, &mut by_model);
                let _ = storage.upsert_tool_file_offset(
                    tool_id, &path_str, new_offset, cur_len, cur_mtime,
                );
            }
        }
    }

    if fallback_full {
        record_tool_offsets(storage, tool_id);
        return super::scan_tool(tool_id, storage);
    }

    merge_into_report(&mut report, &by_day, &by_model);
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const BASE: &str = r#"{"timestamp":"2026-01-02T00:00:00Z","type":"assistant","message":{"model":"m","usage":{"input_tokens":10,"cache_read_input_tokens":5,"output_tokens":3}}}"#;
    const ADDED: &str = r#"{"timestamp":"2026-01-03T00:00:00Z","type":"assistant","message":{"model":"m","usage":{"input_tokens":100,"cache_read_input_tokens":0,"output_tokens":20}}}"#;

    #[test]
    fn read_appended_only_sees_new_complete_lines() {
        let dir = std::env::temp_dir().join(format!("tum_delta_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.jsonl");
        std::fs::write(&f, format!("{BASE}\n")).unwrap();
        let len = std::fs::metadata(&f).unwrap().len();

        let (mut d, mut dm) = (BTreeMap::new(), BTreeMap::new());
        // offset=0：读到第一条完整行并推进指针至文件末尾。
        let off1 = read_appended(&f, 0, "claude-code", &mut d, &mut dm);
        assert_eq!(off1, len);
        assert_eq!(d.len(), 1);

        // 追加一行；从 off1 续读应只看到追加行，不含 base。
        std::fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(format!("{ADDED}\n").as_bytes())
            .unwrap();
        let (mut d2, mut dm2) = (BTreeMap::new(), BTreeMap::new());
        let off2 = read_appended(&f, off1, "claude-code", &mut d2, &mut dm2);
        assert_eq!(d2.len(), 1);
        assert_eq!(d2.iter().next().unwrap().1 .0, 100.0); // 只有 input=100 的追加行
        assert!(off2 > off1);

        // 从 off2 无新增时，不推进指针。
        let (mut d3, mut dm3) = (BTreeMap::new(), BTreeMap::new());
        let off3 = read_appended(&f, off2, "claude-code", &mut d3, &mut dm3);
        assert_eq!(d3.len(), 0);
        assert_eq!(off3, off2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_into_report_adds_to_existing_day_and_model() {
        let mut report = super::LocalToolReport::default();
        add_to_daily(&mut report.daily, "2026-01-03", 5.0, 0.0, 0.0);
        report.daily[0].total = 5.0;
        report.total_tokens = 5.0;

        let mut by_day = BTreeMap::new();
        by_day.insert("2026-01-03".to_string(), (100.0, 20.0, 30.0));
        let mut by_model = BTreeMap::new();
        let mut days = BTreeMap::new();
        days.insert("2026-01-03".to_string(), (100.0, 20.0, 30.0));
        by_model.insert("m".to_string(), days);

        merge_into_report(&mut report, &by_day, &by_model);
        assert!(report.daily.iter().any(|d| d.date == "2026-01-03"));
        let usage = report.models.iter().find(|u| u.model == "m").unwrap();
        assert!(usage.daily.iter().any(|d| d.date == "2026-01-03"));
        assert_eq!(usage.total_tokens, 150.0);
    }
}