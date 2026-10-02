//! 工具日志 watch + 增量扫描（token-monitor 借鉴点，轻量实现）。
//!
//! 每 3 秒用各工具的 **metadata 指纹**（文件 path/len/mtime，不读内容）检测日志是
//! 否变化：没变 → 该工具完全不重扫；变了 → **仅重扫那一个工具**并更新缓存，再把
//! 结果广播给前端。相比之前每 N 秒对全部工具全量扫描，无关工具不再被反复解析。
//!
//! 用轻量轮询替代 OS watcher（notify），避免额外的原生依赖，仍能满足"3-5 秒内反映
//! 新增用量"的目标。

use crate::local::{self, LocalToolReport, LocalToolsPayload, SharedLocalCache};
use crate::storage::Storage;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;

/// 启动工具日志 watch（常驻后台任务）。每 3s 轮询。
pub fn spawn_tool_watcher(app: tauri::AppHandle, local: SharedLocalCache, storage: Arc<Storage>) {
    tauri::async_runtime::spawn(async move {
        let mut last: HashMap<String, String> = HashMap::new();
        loop {
            for tool_id in local::TOOL_IDS {
                let fp = local::cache::tool_fingerprint(tool_id);
                if last.get(tool_id) == Some(&fp) {
                    continue; // 日志未变 → 此工具跳过
                }
                last.insert(tool_id.to_string(), fp);
                // jsonl 追加型工具走字节级增量续读（只解析新增行）；其余整工具重扫。
                let report = if local::delta::is_jsonl_tool(tool_id) {
                    local::delta::refresh_jsonl_tool(&local, &storage, tool_id).await
                } else {
                    local::scan_tool(tool_id, storage.as_ref())
                };
                update_cache(&local, tool_id, report).await;
                if tool_id == "minimax-code" {
                    if let Some(payload) = local.cached().await {
                        local::persist_all_tools(storage.as_ref(), &payload.tools);
                    }
                }
                let _ = app.emit("tools-updated", ());
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
}

/// 把单个工具的重扫结果合并进缓存（按 id 替换）。
async fn update_cache(
    local: &SharedLocalCache,
    tool_id: &str,
    report: Option<LocalToolReport>,
) {
    let mut payload = local.cached().await.unwrap_or_default();
    payload.tools.retain(|t| t.id != tool_id);
    if let Some(r) = report {
        payload.tools.push(r);
    }
    local.store(payload).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_fingerprint_is_stable_across_calls() {
        // 同一工具、源未变时，两次指纹应一致（供 watch 去重判断）。
        for tool in local::TOOL_IDS {
            let a = local::cache::tool_fingerprint(tool);
            let b = local::cache::tool_fingerprint(tool);
            assert_eq!(a, b, "tool {tool} fingerprint not stable");
        }
    }
}