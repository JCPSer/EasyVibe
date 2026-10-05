//! 输出流解析与结果捕获：
//! - `parse_stream_event`：claude stream-json assistant 事件 → 可读文本（跳过 result 防双份）
//! - `parse_stream_meta`：system init → 模型名 / result → usage
//! - `append_capture`：stdout 归档缓冲（1MB 封顶，保头 + RESULT 行替换式保底）
//! 拆自 lib.rs（2026-10-05 防膨胀）。

use crate::SessionMetaUpdate;

/// claude `--output-format stream-json` 事件 → 可读文本。
/// 只取 assistant 事件的正文/思考块（进度可见）；result 事件是全量回放，跳过防双份。
/// 返回 None = 该事件无可展示内容（system/user/result 或解析失败）。
pub(crate) fn parse_stream_event(line: &str) -> Option<String> {
    let ev: serde_json::Value = serde_json::from_str(line).ok()?;
    if ev["type"].as_str() != Some("assistant") {
        return None;
    }
    let content = &ev["message"]["content"];
    let blocks = content.as_array().cloned().unwrap_or_default();
    let mut out = String::new();
    if blocks.is_empty() {
        // 兼容 content 为纯字符串的形态
        if let Some(t) = content.as_str() {
            out.push_str(t);
        }
    }
    for b in blocks {
        match b["type"].as_str() {
            Some("text") => {
                if let Some(t) = b["text"].as_str() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
            Some("thinking") => {
                if let Some(t) = b["thinking"].as_str() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str("[思考] ");
                    out.push_str(t);
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// M1/U1：stream-json 事件 → 会话元更新（system init 抽模型名 / result 抽 usage）。
/// 与 parse_stream_event 互补：那边跳过的 system/result，在这里被分派去落盘——
/// 文本归直播，元数据归 agent_sessions，互不干扰。
pub(crate) fn parse_stream_meta(line: &str) -> Option<SessionMetaUpdate> {
    let ev: serde_json::Value = serde_json::from_str(line).ok()?;
    match ev["type"].as_str()? {
        "system" if ev["subtype"].as_str() == Some("init") => {
            ev["model"].as_str().map(|m| SessionMetaUpdate::Model(m.to_string()))
        }
        "result" => {
            let usage = &ev["usage"];
            // cost_usd 为必填门槛：连自报成本都没有的事件不更新 usage 列（保持 NULL 语义）
            let cost_usd = ev["total_cost_usd"].as_f64()?;
            Some(SessionMetaUpdate::Usage {
                input_tokens: usage["input_tokens"].as_i64()?,
                output_tokens: usage["output_tokens"].as_i64()?,
                cache_read_tokens: usage["cache_read_input_tokens"].as_i64().unwrap_or(0),
                cache_write_tokens: usage["cache_creation_input_tokens"].as_i64().unwrap_or(0),
                cost_usd,
                duration_ms: ev["duration_ms"].as_i64().unwrap_or(0),
                turns: ev["num_turns"].as_i64().unwrap_or(0),
            })
        }
        _ => None,
    }
}
/// stdout 归档缓冲写入（M4-1）：1MB 封顶保头丢尾——RESULT 行在末尾，超帽时替换式保底。
pub(crate) fn append_capture(buf: &std::sync::Mutex<String>, payload: &str) {
    if let Ok(mut buf) = buf.lock() {
        if buf.len() < 1_048_576 {
            buf.push_str(payload);
            if !payload.ends_with('\n') {
                buf.push('\n');
            }
        } else if payload.contains("[EASYVIBE-RESULT]") {
            // 超帽时仍保留 RESULT 归档行（短行，替换式保底）
            let trimmed: String = payload.chars().take(4096).collect();
            buf.push_str(&trimmed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_meta_extracts_model_and_usage() {
        // system init → Model（不等终态）
        let init = r#"{"type":"system","subtype":"init","model":"claude-sonnet-4-6"}"#;
        match parse_stream_meta(init) {
            Some(SessionMetaUpdate::Model(m)) => assert_eq!(m, "claude-sonnet-4-6"),
            other => panic!("init 事件应解析出模型名: {other:?}"),
        }
        // result → Usage（claude 自报口径：cost/tokens/时长/轮数）
        let result = r#"{"type":"result","model":"claude-sonnet-4-6","total_cost_usd":0.0421,"duration_ms":52000,"num_turns":3,
            "usage":{"input_tokens":12000,"output_tokens":3000,"cache_read_input_tokens":8000,"cache_creation_input_tokens":4000}}"#;
        match parse_stream_meta(result) {
            Some(SessionMetaUpdate::Usage { input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd, duration_ms, turns }) => {
                assert_eq!(input_tokens, 12000);
                assert_eq!(output_tokens, 3000);
                assert_eq!(cache_read_tokens, 8000);
                assert_eq!(cache_write_tokens, 4000);
                assert!((cost_usd - 0.0421).abs() < 1e-9);
                assert_eq!(duration_ms, 52000);
                assert_eq!(turns, 3);
            }
            other => panic!("result 事件应解析出 usage: {other:?}"),
        }
        // 无 total_cost_usd → 不更新（usage 列保持 NULL 语义）
        let no_cost = r#"{"type":"result","usage":{"input_tokens":1,"output_tokens":1}}"#;
        assert!(parse_stream_meta(no_cost).is_none());
        // assistant/垃圾行不归元分派
        assert!(parse_stream_meta(r#"{"type":"assistant"}"#).is_none());
        assert!(parse_stream_meta("not json").is_none());
    }

    #[test]
    fn stream_event_extracts_assistant_text_and_skips_result() {
        // stream-json → 文本还原：assistant 正文/思考块提取；result/system 跳过（防双份）
        let text_ev = r###"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"先读文件"},{"type":"text","text":"## 交付摘要\n改动完成"}]}}"###;
        let out = parse_stream_event(text_ev).expect("assistant 事件必须有文本");
        assert!(out.contains("[思考] 先读文件"), "思考块保留: {out}");
        assert!(out.contains("## 交付摘要"), "正文保留: {out}");

        // 协议行在文本块内保持纯文本形态——take_output 的 RESULT/REVIEW 解析零改动
        let proto_ev = r###"{"type":"assistant","message":{"content":[{"type":"text","text":"完成\n[EASYVIBE-RESULT] {\"summary\":\"ok\"}"}]}}"###;
        let out = parse_stream_event(proto_ev).unwrap();
        assert!(out.lines().any(|l| l.starts_with("[EASYVIBE-RESULT]")), "协议行必须是独立纯文本行: {out}");

        // result 事件 = 全量回放，必须跳过（否则双份）
        let result_ev = r#"{"type":"result","result":"全部内容回放"}"#;
        assert!(parse_stream_event(result_ev).is_none());
        // system 事件跳过
        let sys_ev = r#"{"type":"system","subtype":"init"}"#;
        assert!(parse_stream_event(sys_ev).is_none());
        // 非 JSON 行（异常输出）→ None（不广播原始垃圾）
        assert!(parse_stream_event("not json at all").is_none());
    }}
