//! Data export: the current snapshot and the local-tool ledger as a file the
//! user owns.
//!
//! Two shapes on purpose:
//! - JSON is a faithful dump (serde, pretty) and can hold every scope at once;
//! - CSV is a spreadsheet view, one row per (entity, bucket) so it pivots in
//!   Excel/Sheets without any post-processing.
//!
//! CSV is deliberately *not* offered for the `all` scope. The two datasets
//! share no columns (provider windows are quota/limit pairs; tool days are
//! token counts), and a merged table would either drop half the data or invent
//! empty columns the reader has to ignore. One honest table beats one wide
//! one; `all` + `csv` is refused with that reason.

use crate::local::LocalToolsPayload;
use crate::providers::UsageSnapshot;
use std::fmt::Write as _;

/// Escape one CSV field per RFC 4180: wrap in quotes when the value contains a
/// delimiter, quote, or newline, and double any embedded quote.
///
/// A leading `=`, `+`, `-` or `@` is additionally prefixed with a single
/// quote. Provider display names come from third-party APIs and end up in a
/// file the user is likely to open in Excel, where an unescaped leading `=`
/// is a formula. Losing a literal quote here is a far better outcome than
/// losing a file, or executing something on open.
pub fn csv_field(s: &str) -> String {
    // Control characters would survive the quoting below but confuse the
    // spreadsheet; collapse them so one cell stays one line.
    let cleaned: String = s
        .chars()
        .filter(|c| *c != '\r' && *c != '\n' && *c != '\t')
        .collect();
    let neutralized = if cleaned.starts_with(['=', '+', '-', '@']) {
        format!("'{cleaned}")
    } else {
        cleaned
    };
    if neutralized.contains([',', '"', '\n']) {
        format!("\"{}\"", neutralized.replace('"', "\"\""))
    } else {
        neutralized
    }
}

/// Render the provider snapshots as one row per (provider, window).
///
/// Long rather than wide on purpose: a wide table needs four parallel column
/// groups (5h/daily/weekly/monthly) and a reader has to know which group is
/// populated. Long keeps the window name in a cell, so pivoting and filtering
/// both work.
pub fn render_providers_csv(snaps: &[UsageSnapshot]) -> String {
    let mut out = String::from(
        "provider_id,provider_name,window,used,quota,unit,reset_at,overage,snapshot_at\n",
    );
    for s in snaps {
        let w = &s.windows;
        let timestamp = s.timestamp.to_rfc3339();
        let mut push = |window: &str, u: &crate::providers::WindowUsage| {
            let _ = writeln!(
                out,
                "{},{},{},{},{},{},{},{},{}",
                csv_field(&s.provider_id),
                csv_field(&s.provider_display_name),
                window,
                u.used,
                u.quota,
                csv_field(u.unit.label()),
                u.reset_at
                    .map(|r| r.to_rfc3339())
                    .unwrap_or_default(),
                u.over_quota,
                csv_field(&timestamp),
            );
        };
        if let Some(x) = &w.five_hour {
            push("5h", x);
        }
        if let Some(x) = &w.daily {
            push("daily", x);
        }
        if let Some(x) = &w.weekly {
            push("weekly", x);
        }
        if let Some(x) = &w.monthly {
            push("monthly", x);
        }
        if let Some(b) = &w.balance {
            let _ = writeln!(
                out,
                "{},{},{},{},{},{},{},{},{}",
                csv_field(&s.provider_id),
                csv_field(&s.provider_display_name),
                "balance",
                b.total,
                "",
                csv_field(&b.currency),
                "",
                false,
                csv_field(&timestamp),
            );
        }
    }
    out
}

/// Render the local-tool ledger as one row per (tool, day).
pub fn render_tools_csv(payload: &LocalToolsPayload) -> String {
    let mut out = String::from("tool_id,tool_name,date,input,cache_read,output,total\n");
    for t in &payload.tools {
        for d in &t.daily {
            let _ = writeln!(
                out,
                "{},{},{},{},{},{},{}",
                csv_field(&t.id),
                csv_field(&t.name),
                csv_field(&d.date),
                d.input,
                d.cache_read,
                d.output,
                d.total,
            );
        }
    }
    out
}

/// Number of data rows `render_providers_csv` would emit (header excluded).
pub fn provider_row_count(snaps: &[UsageSnapshot]) -> usize {
    snaps
        .iter()
        .map(|s| {
            let w = &s.windows;
            usize::from(w.five_hour.is_some())
                + usize::from(w.daily.is_some())
                + usize::from(w.weekly.is_some())
                + usize::from(w.monthly.is_some())
                + usize::from(w.balance.is_some())
        })
        .sum()
}

/// Number of data rows `render_tools_csv` would emit (header excluded).
pub fn tool_row_count(payload: &LocalToolsPayload) -> usize {
    payload.tools.iter().map(|t| t.daily.len()).sum()
}

/// The JSON document for the `all` scope.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportAllJson<'a> {
    pub providers: &'a [UsageSnapshot],
    pub tools: &'a LocalToolsPayload,
}

#[cfg(test)]
mod tests {
    use super::{csv_field, render_providers_csv, render_tools_csv};
    use crate::local::{LocalDay, LocalToolReport, LocalToolsPayload};
    use crate::providers::{CostSource, UsageSnapshot, UsageUnit, UsageWindows, WindowUsage};
    use chrono::{TimeZone, Utc};

    fn snap(name: &str, display: &str) -> UsageSnapshot {
        let w = WindowUsage {
            used: 12.5,
            quota: 100.0,
            unit: UsageUnit::Tokens,
            reset_at: None,
            over_quota: false,
            cost_source: CostSource::ProviderReported,
            tokens: None,
        };
        UsageSnapshot {
            provider_id: name.to_string(),
            provider_display_name: display.to_string(),
            plan_tier: None,
            timestamp: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
            windows: UsageWindows {
                five_hour: Some(w),
                daily: None,
                weekly: None,
                monthly: None,
                balance: None,
            },
            heatmap: None,
        }
    }

    #[test]
    fn plain_fields_are_not_quoted() {
        assert_eq!(csv_field("deepseek"), "deepseek");
        assert_eq!(csv_field(""), "");
    }

    #[test]
    fn delimiters_quotes_and_newlines_are_escaped() {
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        // A newline inside a cell is illegal in RFC 4180: collapse rather than
        // emit a row the parser will split in the wrong place.
        assert_eq!(csv_field("a\nb"), "ab");
        assert_eq!(csv_field("a\tb"), "ab");
    }

    #[test]
    fn leading_formula_characters_are_neutralized() {
        // Excel/Calc treat these as a formula on open.
        assert_eq!(csv_field("=1+1"), "'=1+1");
        assert_eq!(csv_field("+1"), "'+1");
        assert_eq!(csv_field("-1"), "'-1");
        assert_eq!(csv_field("@SUM(A1)"), "'@SUM(A1)");
        // Only a *leading* character is dangerous; interior ones are inert.
        assert_eq!(csv_field("a=b"), "a=b");
    }

    #[test]
    fn provider_csv_is_one_row_per_window() {
        let csv = render_providers_csv(&[snap("ds", "DeepSeek")]);
        let lines: Vec<&str> = csv.trim_end().split('\n').collect();
        assert_eq!(lines[0], "provider_id,provider_name,window,used,quota,unit,reset_at,overage,snapshot_at");
        assert_eq!(lines.len(), 2, "header + one populated window");
        assert!(lines[1].starts_with("ds,DeepSeek,5h,12.5,100,tokens,,"));
    }

    #[test]
    fn provider_csv_skips_absent_windows() {
        // daily/weekly/monthly are None in the fixture, so no empty rows.
        let csv = render_providers_csv(&[snap("ds", "DeepSeek")]);
        assert!(!csv.contains(",daily,"));
        assert!(!csv.contains(",balance,"));
    }

    #[test]
    fn provider_csv_quotes_a_name_with_a_comma() {
        let csv = render_providers_csv(&[snap("x", "Acme, Inc")]);
        assert!(csv.contains("\"Acme, Inc\""));
    }

    #[test]
    fn tools_csv_is_one_row_per_day() {
        let payload = LocalToolsPayload {
            tools: vec![LocalToolReport {
                id: "claude-code".to_string(),
                name: "Claude Code".to_string(),
                daily: vec![
                    LocalDay {
                        date: "2026-01-02".to_string(),
                        input: 10.0,
                        cache_read: 5.0,
                        output: 2.0,
                        total: 17.0,
                    },
                    LocalDay {
                        date: "2026-01-03".to_string(),
                        input: 1.0,
                        cache_read: 0.0,
                        output: 1.0,
                        total: 2.0,
                    },
                ],
                ..Default::default()
            }],
            sessions_parsed: 2,
        };
        let csv = render_tools_csv(&payload);
        let lines: Vec<&str> = csv.trim_end().split('\n').collect();
        assert_eq!(lines.len(), 3, "header + two days");
        assert_eq!(
            lines[1],
            "claude-code,Claude Code,2026-01-02,10,5,2,17"
        );
    }

    #[test]
    fn empty_input_yields_a_header_only_file() {
        assert_eq!(
            render_providers_csv(&[]).trim_end().split('\n').count(),
            1
        );
        assert_eq!(
            render_tools_csv(&LocalToolsPayload::default()),
            "tool_id,tool_name,date,input,cache_read,output,total\n"
        );
    }
}
