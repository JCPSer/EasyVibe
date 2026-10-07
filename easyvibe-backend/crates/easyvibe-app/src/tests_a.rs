//! main.rs 内联测试（迁移自 god file，仅测试编译）——分片 1/4。

use crate::assets::*;
use crate::router::*;
use crate::service::agent::{llm_test_inner, LlmTestBody};
use easyvibe_map::concerns::*;
use crate::service::map::*;
use crate::test_support::*;
use tower::ServiceExt;

    /// llm_test：不存在且未填 Base URL 的服务必须直接报错（不发 HTTP、不 panic）
    #[tokio::test]
    async fn llm_test_rejects_unknown_service() {
        let st = test_state().await;
        let err = llm_test_inner(&st, LlmTestBody {
            service_id: "no-such-service".into(),
            base_url: None, model: None, api_key: None,
        }).await;
        assert!(err.is_err(), "未知服务必须报错: {:?}", err.ok());
    }

    /// 2026-10-04 打磨批三新行为回归：视图冲突 409+force 覆盖 / dev-doc 删除与越界防线 /
    /// harness restore 非法备份名拒绝（不触文件系统）
    #[tokio::test]
    async fn audit_batch_view_conflict_devdoc_delete_harness_guard() {
        let (state, repo) = chat_state("audit-batch").await;
        let root = std::env::temp_dir().join("ev-chat-test-audit-batch");
        let app = build_router(state);
        let post = |app: axum::Router, url: String, body: String| {
            async move {
                app.oneshot(
                    axum::http::Request::post(url)
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };
        // 1) 视图：首次 201，同名再存 409，force 覆盖 201
        let view_body = r#"{"name":"冲突视图","nodes":["module:exam-core"],"edges":[],"annotations":[]}"#.to_string();
        let r1 = post(app.clone(), format!("/api/repos/{repo}/views"), view_body.clone()).await;
        assert_eq!(r1.status(), axum::http::StatusCode::CREATED);
        let r2 = post(app.clone(), format!("/api/repos/{repo}/views"), view_body.clone()).await;
        assert_eq!(r2.status(), axum::http::StatusCode::CONFLICT, "同名冲突必须 409（不再静默另存后缀）");
        let r3 = post(app.clone(), format!("/api/repos/{repo}/views?force=true"), view_body).await;
        assert_eq!(r3.status(), axum::http::StatusCode::CREATED, "force 覆盖应成功");
        let views = std::fs::read_dir(root.join(".easyvibe/views")).unwrap().count();
        assert_eq!(views, 1, "覆盖后仍只有一个同名视图文件");

        // 2) dev-doc：正常删除成功 + 路径越界被拒
        let doc_dir = root.join(".easyvibe/development_docs/u/1_requirements_matrix");
        std::fs::create_dir_all(&doc_dir).unwrap();
        std::fs::write(doc_dir.join("a.md"), "hello").unwrap();
        let del = |app: axum::Router, body: String| {
            let repo = repo.clone();
            async move {
                app.oneshot(
                    axum::http::Request::delete(format!("/api/repos/{repo}/dev-doc"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };
        let d1 = del(app.clone(), r#"{"path":"../map/map.json"}"#.to_string()).await;
        assert_eq!(d1.status(), axum::http::StatusCode::NOT_FOUND, "越界路径必须拒绝（保护地图文件）");
        assert!(root.join(".easyvibe/map/map.json").exists(), "地图文件不得被越界删除");
        let d2 = del(app.clone(), r#"{"path":"u/1_requirements_matrix/a.md"}"#.to_string()).await;
        assert_eq!(d2.status(), axum::http::StatusCode::OK);
        assert!(!doc_dir.join("a.md").exists(), "正常删除应落盘");

        // 3) harness custom 槽：路径游戏在触文件系统前被拒（v2：出厂端点全删，防线收在白名单）
        let h = app
            .clone()
            .oneshot(
                axum::http::Request::delete("/api/harness/custom/file?path=../state.json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(h.status(), axum::http::StatusCode::BAD_REQUEST, "路径游戏必须 400");
    }

    /// 资产解析链兜底：env 未设 + exe 旁无 + cwd 无 → 内嵌副本（persist=false 不落盘）
    #[test]
    fn text_asset_chain_falls_back_to_embedded() {
        let (s, p) = resolve_text_asset(
            "EASYVIBE_TEST_DEFINITELY_UNSET_VAR",
            "easyvibe-test-nonexistent-asset-6f2d.md",
            "EMBEDDED_BODY",
            false,
        );
        assert_eq!(s, "EMBEDDED_BODY");
        assert!(p.contains("<embedded:"), "应标记为内嵌来源: {p}");
    }

    /// 资产解析链 env 优先：显式路径存在时直接用
    #[test]
    fn text_asset_chain_env_wins() {
        let f = std::env::temp_dir().join("easyvibe-test-asset-env-wins.md");
        std::fs::write(&f, "ENV_BODY").unwrap();
        // 安全：测试进程独占临时文件读写，无并发 set_env 竞态
        std::env::set_var("EASYVIBE_TEST_ASSET_VAR", &f);
        let (s, p) = resolve_text_asset("EASYVIBE_TEST_ASSET_VAR", "whatever.md", "EMBEDDED", false);
        std::env::remove_var("EASYVIBE_TEST_ASSET_VAR");
        assert_eq!(s, "ENV_BODY");
        assert_eq!(p, f.to_string_lossy().into_owned());
        std::fs::remove_file(&f).ok();
    }

    // c-arch-10 ΔS8：`mime_by_ext` 随静态托管外提至 assembly/static_host.rs，
    // 其唯一测试消费点亦随迁（见该文件内联 `#[cfg(test)] mod tests`）。

    #[tokio::test]
    async fn health_ok() {
        let app = build_router(test_state().await);
        let resp = app.oneshot(axum::http::Request::get("/api/health").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn map_etag_conditional_304() {
        // Y1 清债回归：ETag 命中即 304（重连重同步不再全量传输）
        let (state, repo) = chat_state("etag").await;
        let app = build_router(state);
        let get = |if_none: Option<&str>| {
            let mut req = axum::http::Request::get(format!("/api/repos/{repo}/map"))
                .body(axum::body::Body::empty()).unwrap();
            if let Some(v) = if_none {
                req.headers_mut().insert("if-none-match", v.parse().unwrap());
            }
            let app = app.clone();
            async move {
                let resp = app.oneshot(req).await.unwrap();
                (resp.status(), resp.headers().get("etag").and_then(|h| h.to_str().ok()).unwrap_or("").to_string())
            }
        };
        let (status, etag) = get(None).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(etag.starts_with('"') && etag.ends_with('"'), "ETag 应为引号包裹的哈希: {etag}");
        let (status2, _) = get(Some(&etag)).await;
        assert_eq!(status2, axum::http::StatusCode::NOT_MODIFIED, "If-None-Match 命中应 304");
    }

    #[tokio::test]
    async fn unknown_repo_404() {
        let app = build_router(test_state().await);
        let resp = app.oneshot(axum::http::Request::get("/api/repos/nope/map").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn diff_concerns_classifies_fixed_new_persisted() {
        // 2026-10-05 巡检新旧对照：id 继承 / 无 id 文本兜底 / 模块消失区分 / 首巡全 new
        let map = |concerns: serde_json::Value| {
            serde_json::json!({
                "health": { "concerns": concerns["arch"] },
                "modules": concerns["modules"],
            })
        };
        let old = vec![
            OldConcern { scope: "arch", module: None, id: Some("c-arch-1".into()), finding: "A".into() },
            OldConcern { scope: "arch", module: None, id: None, finding: "B".into() },          // 无 id：文本兜底
            OldConcern { scope: "module", module: Some("m1".into()), id: Some("c-m1-1".into()), finding: "C".into() },
            OldConcern { scope: "module", module: Some("m2".into()), id: Some("c-m2-1".into()), finding: "D".into() },
        ];
        let new_map = map(serde_json::json!({
            "arch": [
                { "id": "c-arch-1", "finding": "A 措辞改了也算同一项" },   // id 继承 → persisted
                { "id": "c-arch-2", "finding": "B" },                        // 无 id 文本兜底 → persisted
            ],
            "modules": [
                { "id": "m1", "health": { "concerns": [{ "id": "c-m1-1", "finding": "C" }] } },
                // m2 整模块消失 → moduleGone，不算 fixed
            ],
        }));
        let d = diff_concerns(&old, &new_map);
        assert_eq!(d["persisted"], 3, "id 继承 1 + 文本兜底 1 + 模块内继承 1");
        assert_eq!(d["fixed"].as_array().unwrap().len(), 0);
        assert_eq!(d["new"].as_array().unwrap().len(), 0);
        assert_eq!(d["moduleGone"].as_array().unwrap().len(), 1);

        // 新问题 + 修复并存（m1 仍在但 concern 被修 = fixed；m2 整模块消失 = moduleGone）
        let new_map2 = map(serde_json::json!({
            "arch": [ { "id": "c-arch-9", "finding": "新问题" } ],
            "modules": [ { "id": "m1", "health": { "concerns": [] } } ],
        }));
        let d2 = diff_concerns(&old, &new_map2);
        assert_eq!(d2["new"].as_array().unwrap().len(), 1);
        assert_eq!(d2["fixed"].as_array().unwrap().len(), 3, "A（id 未命中）+ B（文本未命中）+ C（m1 还在但 concern 没了）");
        assert_eq!(d2["moduleGone"].as_array().unwrap().len(), 1, "D 随 m2 整模块消失");
        // 首巡（旧为空）→ 全 new（架构 2 + m1 模块 1 = 3）
        let d3 = diff_concerns(&[], &new_map);
        assert_eq!(d3["new"].as_array().unwrap().len(), 3);
        assert_eq!(d3["persisted"], 0);
    }

    #[test]
    fn assign_concern_ids_backfills_stable_ids() {
        // 2026-10-05 实弹回归：LLM 未输出 id 时由后端兜底——
        // 缺 id 的项按 scope 顺延旧编号分配；已有 id 不动；编号不冲突。
        let old = vec![
            OldConcern { scope: "arch", module: None, id: Some("c-arch-1".into()), finding: "A".into() },
            OldConcern { scope: "module", module: Some("m1".into()), id: Some("c-m1-1".into()), finding: "C".into() },
            OldConcern { scope: "module", module: Some("m2".into()), id: Some("c-m2-1".into()), finding: "D".into() },
        ];
        let mut map = serde_json::json!({
            "health": { "concerns": [
                { "severity": "high", "finding": "A 仍在（有 id 不动）", "id": "c-arch-1" },
                { "severity": "high", "finding": "B 新冒出的架构问题" },
            ] },
            "modules": [
                { "id": "m1", "health": { "concerns": [ { "severity": "high", "finding": "C 仍在但 LLM 没给 id" } ] } },
                { "id": "m3", "health": { "concerns": [ { "severity": "high", "finding": "E 新模块新问题" } ] } },
            ],
        });
        let changed = assign_concern_ids(&old, &mut map);
        assert_eq!(changed, 3, "B/C/E 三项补 id，A 已有不动");
        assert_eq!(map["health"]["concerns"][0]["id"], "c-arch-1");
        assert_eq!(map["health"]["concerns"][1]["id"], "c-arch-2", "arch 顺延 1 之后");
        assert_eq!(map["modules"][0]["health"]["concerns"][0]["id"], "c-m1-2", "m1 顺延 c-m1-1 之后");
        assert_eq!(map["modules"][1]["health"]["concerns"][0]["id"], "c-m3-1", "m3 无旧编号从 1 起");
        // 二次运行幂等：已有 id 全部不动
        let again = assign_concern_ids(&old, &mut map);
        assert_eq!(again, 0, "幂等：不重复分配");
    }

    #[tokio::test]
    async fn progress_done_ago_gates_on_phase() {
        let dir = std::env::temp_dir().join("ev-progress-done-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        // 无文件 → None
        assert!(progress_done_ago_secs(&dir).is_none());
        // phase != done → None（归纳中不得误判）
        std::fs::write(dir.join(".easyvibe/map/progress.json"), r#"{"phase":"inducting"}"#).unwrap();
        assert!(progress_done_ago_secs(&dir).is_none());
        // phase=done → Some（文件刚写，ago 很小）
        std::fs::write(dir.join(".easyvibe/map/progress.json"), r#"{"phase":"done"}"#).unwrap();
        let ago = progress_done_ago_secs(&dir).expect("done 应有秒数");
        assert!(ago < 5, "刚落盘的文件 ago 应极小: {ago}");
    }

    /// 2026-10-04 实弹回归：收尸只认本会话写出的 100%——旧 progress.json
    /// （上次归纳的 done）对新一轮会话必须返回 None，否则新会话秒被杀。
    #[tokio::test]
    async fn grace_ignores_stale_progress_done() {
        let dir = std::env::temp_dir().join("ev-progress-stale-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        std::fs::write(dir.join(".easyvibe/map/progress.json"), r#"{"phase":"done"}"#).unwrap();
        // 会话在文件落盘之后启动 → 旧 done 不算数
        let since_later = std::fs::metadata(dir.join(".easyvibe/map/progress.json")).unwrap().modified().unwrap()
            + std::time::Duration::from_secs(1);
        assert!(progress_done_ago_secs_since(&dir, since_later).is_none(), "旧 done 不得触发收尸");
        // 会话先于文件落盘启动（模拟归纳进行中写出了 done）→ 才算本会话产物
        let since_earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let ago = progress_done_ago_secs_since(&dir, since_earlier).expect("新 done 应被承认");
        assert!(ago < 5, "刚落盘的文件 ago 应极小: {ago}");
    }

    #[tokio::test]
    async fn chat_persists_and_restores_from_db() {
        let (state, repo) = chat_state("persist").await;
        let app = build_router(state);
        let r = post_chat(&app, &repo, "谁负责评测提交？").await;
        let reply = r["data"]["reply"].as_str().unwrap_or_default();
        assert!(reply.contains("考试与评测核心"), "stub 应答应基于地图, reply={reply}");
        assert!(r["data"]["refs"].as_array().unwrap().iter().any(|x| x == "exam-core"));

        // 第二轮后再恢复：完整历史从库读（M3-5：不再只活在前端 state）
        post_chat(&app, &repo, "健康度多少分？").await;
        let resp = app
            .clone()
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/chat")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        let messages = data["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "两轮对话 = user+assistant × 2");
        assert_eq!(messages[0]["role"], "user");
        assert!(messages[0]["content"].as_str().unwrap().contains("评测提交"));
        // token 用量已记账（stub 估算为正；POST 返回累计口径）
        assert!(data["usage"]["promptTokens"].as_i64().unwrap() > 0, "记账链路应产生正用量");
    }

    #[tokio::test]
    async fn chat_module_refs_injected_but_original_persisted() {
        let (state, repo) = chat_state("mrefs").await;
        let app = build_router(state);
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/chat"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({ "message": "它健康吗？", "module_refs": ["exam-core", "ghost-module"] }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "带 module_refs 的 chat 应成功");
        // 持久化的是用户原文：聚焦块与未知 id 都不得进库（D9：聚焦只活在本轮 LLM 调用）
        let resp = app
            .clone()
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/chat")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let messages = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["messages"]
            .as_array()
            .unwrap()
            .clone();
        let last_user = messages.iter().rev().find(|m| m["role"] == "user").unwrap();
        let content = last_user["content"].as_str().unwrap();
        assert_eq!(content, "它健康吗？");
        assert!(!content.contains("聚焦模块"), "聚焦块不得落库");
        assert!(!content.contains("ghost-module"), "未知模块 id 不得落库");
    }

    #[tokio::test]
    async fn conversations_multi_create_chat_scope() {
        let (state, repo) = chat_state("multiconv").await;
        let app = build_router(state);
        let get = |path: String| {
            let app = app.clone();
            async move {
                app.oneshot(axum::http::Request::get(path).body(axum::body::Body::empty()).unwrap()).await.unwrap()
            }
        };
        // 初始：每仓库一个默认会话
        let resp = get(format!("/api/repos/{repo}/conversations")).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(list["data"].as_array().unwrap().len(), 0, "会话懒创建：初始应为 0");

        // 新建第二个会话
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/conversations"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"title":"重构专项"}"#.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let created = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        let cid = created["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(created["data"]["title"], "重构专项");
        assert_eq!(created["data"]["runtime"]["state"], "idle");

        // 向新会话发消息：不影响默认会话
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/chat"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({ "message": "只在新会话里", "conv": cid }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let resp = get(format!("/api/repos/{repo}/chat?conv={cid}")).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let d = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(d["data"]["messages"].as_array().unwrap().len(), 2, "新会话应有问答两条");
        assert_eq!(d["data"]["conversation"]["title"], "重构专项");

        // 缺省会话 = 最近活跃（M4-2 语义）：不带 conv 的 /chat 应回到刚聊过的会话
        let resp = get(format!("/api/repos/{repo}/chat")).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let d = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(d["data"]["conversation"]["id"], cid, "缺省应回落到最近活跃的会话");
        assert_eq!(d["data"]["messages"].as_array().unwrap().len(), 2);

        // 重命名生效
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::put(format!("/api/repos/{repo}/conversations/{cid}"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"title":"改名了"}"#.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let resp = get(format!("/api/repos/{repo}/conversations")).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        // 缺省回落不新建会话：列表仍只有手工创建的那一个（懒创建纪律）
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        assert!(list["data"].as_array().unwrap().iter().any(|c| c["title"] == "改名了"));
    }

    #[tokio::test]
    async fn y6_blocks_cross_site_but_allows_tauri_origin() {
        let (state, repo) = chat_state("y6").await;
        let app = build_router(state);
        // evil 页面：cross-site 且无白名单 Origin → 403
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/api/repos")
                    .header("sec-fetch-site", "cross-site")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN, "跨站请求应被 Y6 拦截");
        // 桌面壳：Origin 白名单 → 放行（evil 页面无法伪造 Origin）
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/api/repos")
                    .header("sec-fetch-site", "cross-site")
                    .header("origin", TAURI_ORIGIN)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "桌面壳 Origin 应豁免 Y6");
        let _ = repo;
    }

    #[tokio::test]
    async fn chat_manual_compact_leaves_trace_and_keeps_window() {
        let (state, repo) = chat_state("compact").await;
        // 让会话超过近期窗口（8 轮 = 16 条 > KEEP_RECENT 6）
        let app = build_router(state);
        for i in 0..8 {
            post_chat(&app, &repo, &format!("第 {i} 个问题：模块职责是什么？")).await;
        }
        // 手动压缩（force，绕过阈值）
        let resp = app
            .clone()
            .oneshot(axum::http::Request::post(format!("/api/repos/{repo}/chat/compact")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        assert_eq!(data["compacted"], true);
        let trace = data["trace"].as_str().unwrap().to_string();
        assert!(trace.contains("上下文已压缩"), "留痕消息: {trace}");

        // 恢复：系统留痕消息在列；水位前消息标 compacted（原文保留可回放）；近期窗口未压缩
        let resp = app
            .clone()
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/chat")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        let messages = data["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 17, "16 条对话 + 1 条系统留痕");
        assert_eq!(messages[16]["role"], "system");
        assert!(messages[16]["content"].as_str().unwrap().contains("上下文已压缩"));
        assert!(messages.iter().take(10).all(|m| m["compacted"] == true), "水位前 10 条已折叠进摘要");
        assert!(messages[10..16].iter().all(|m| m["compacted"] == false), "近期窗口原文保留");
        // 摘要非空且 stub 诚实标注
        let summary = data["summary"].as_str().unwrap().to_string();
        assert!(summary.contains("stub"), "stub 模式压缩应诚实标注: {summary}");
    }
