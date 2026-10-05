//! 问题项对照域（2026-10-05 巡检新旧对照）：concerns 快照提取、两阶段匹配、
//! 修复/新增/持续对照、稳定 id 兜底注入。纯函数集合——从 routes/map 独立出来，
//! 既给 god file 守卫留出空间，也让对照逻辑可单测。
//!
//! 实弹背景：LLM 连续两轮未遵守巡检提示词的 id 继承规则，对照退化为洗牌——
//! id 稳定性改由本模块在后端兜底保证（assign_concern_ids），不依赖模型自觉。

/// 问题项快照条目（巡检新旧对照的旧侧）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OldConcern {
    pub(crate) scope: &'static str, // "arch" | "module"
    pub(crate) module: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) finding: String,
}

/// 从地图 JSON 提取全部 concerns（架构级 + 各模块级）——巡检开始前快照用。
pub(crate) fn extract_concerns(map: &serde_json::Value) -> Vec<OldConcern> {
    let mut out = Vec::new();
    let mut push = |scope: &'static str, module: Option<String>, c: &serde_json::Value| {
        out.push(OldConcern {
            scope,
            module,
            id: c["id"].as_str().map(Into::into),
            finding: c["finding"].as_str().unwrap_or_default().into(),
        });
    };
    for c in map["health"]["concerns"].as_array().into_iter().flatten() {
        push("arch", None, c);
    }
    for m in map["modules"].as_array().into_iter().flatten() {
        let mid = m["id"].as_str().map(Into::into);
        for c in m["health"]["concerns"].as_array().into_iter().flatten() {
            push("module", mid.clone(), c);
        }
    }
    out
}

/// 两阶段匹配（评审#S3）——先 (scope, module, id) 精确匹配（双侧都有 id），
/// 剩余项按 (scope, module, finding) 文本精确匹配兜底（旧图无 id 的过渡期数据；
/// 新侧不限制 id——LLM 会给继承来的旧问题也分配新 id，但旧侧无 id 时文本是唯一锚点）。
/// 返回 (旧侧命中, 新侧命中)——diff_concerns 与 assign_concern_ids 共用。
fn match_concerns(old: &[OldConcern], new: &[OldConcern]) -> (Vec<bool>, Vec<bool>) {
    let mut used_old = vec![false; old.len()];
    let mut used_new = vec![false; new.len()];
    for (oi, o) in old.iter().enumerate() {
        let Some(oid) = &o.id else { continue };
        for (ni, n) in new.iter().enumerate() {
            if used_new[ni] || n.id.as_deref() != Some(oid.as_str()) {
                continue;
            }
            if o.scope == n.scope && o.module == n.module {
                used_old[oi] = true;
                used_new[ni] = true;
                break;
            }
        }
    }
    for (oi, o) in old.iter().enumerate() {
        if used_old[oi] || o.id.is_some() {
            continue;
        }
        for (ni, n) in new.iter().enumerate() {
            if used_new[ni] || n.finding != o.finding {
                continue;
            }
            if o.scope == n.scope && o.module == n.module {
                used_old[oi] = true;
                used_new[ni] = true;
                break;
            }
        }
    }
    (used_old, used_new)
}

/// 巡检新旧对照（纯函数；测试全覆盖）：fixed = 旧有今无 / new = 旧无今有 /
/// persisted = 两边都有。仍孤立的旧项计 fixed——但其模块已从新图消失的计
/// moduleGone（评审#S3：模块没了 ≠ 修好了）。
pub(crate) fn diff_concerns(old: &[OldConcern], new_map: &serde_json::Value) -> serde_json::Value {
    let new = extract_concerns(new_map);
    let (used_old, used_new) = match_concerns(old, &new);

    let alive_modules: std::collections::HashSet<&str> = new_map["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str())
        .collect();
    let item = |c: &OldConcern| serde_json::json!({ "id": c.id, "finding": c.finding });
    let mut fixed = Vec::new();
    let mut module_gone = Vec::new();
    for (oi, o) in old.iter().enumerate() {
        if used_old[oi] {
            continue;
        }
        match o.scope {
            "module" if o.module.as_deref().is_some_and(|m| !alive_modules.contains(m)) => {
                module_gone.push(serde_json::json!({ "module": o.module, "finding": o.finding }));
            }
            _ => fixed.push(item(o)),
        }
    }
    let fresh: Vec<_> = new.iter().zip(used_new.iter()).filter(|(_, u)| !**u).map(|(n, _)| item(n)).collect();
    serde_json::json!({
        "fixed": fixed,
        "new": fresh,
        "persisted": used_old.iter().filter(|b| **b).count(),
        "moduleGone": module_gone,
    })
}

/// id 兜底注入：巡检成功后由后端不靠 LLM 保证 id 稳定——已有 id 不动；
/// 缺 id 的（持续项或新增项）在该 scope 顺延最大编号分配。
/// 返回变动数（>0 时调用方应把 map 原子写回）——当轮注入后下轮对照即可信。
pub(crate) fn assign_concern_ids(old: &[OldConcern], map: &mut serde_json::Value) -> usize {
    // 与 extract_concerns 完全同序提取新侧，并记录可写位置（JSON pointer 段）
    enum Slot {
        Arch(usize),
        Module(usize, usize),
    }
    let mut slots: Vec<(Slot, OldConcern)> = Vec::new();
    for (i, c) in map["health"]["concerns"].as_array().into_iter().flatten().enumerate() {
        slots.push((Slot::Arch(i), OldConcern {
            scope: "arch", module: None,
            id: c["id"].as_str().map(Into::into),
            finding: c["finding"].as_str().unwrap_or_default().into(),
        }));
    }
    for (mi, m) in map["modules"].as_array().into_iter().flatten().enumerate() {
        let mid = m["id"].as_str().map(Into::into);
        for (ci, c) in m["health"]["concerns"].as_array().into_iter().flatten().enumerate() {
            slots.push((Slot::Module(mi, ci), OldConcern {
                scope: "module", module: mid.clone(),
                id: c["id"].as_str().map(Into::into),
                finding: c["finding"].as_str().unwrap_or_default().into(),
            }));
        }
    }
    // 每 scope 下一编号：旧 id 数字后缀最大值起步（moduleGone 的编号自然释放）
    let scope_key = |c: &OldConcern| c.module.clone().unwrap_or_else(|| "arch".into());
    let mut next: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for o in old {
        if let Some(id) = &o.id {
            if let Some((_, tail)) = id.rsplit_once('-') {
                if let Ok(n) = tail.parse::<usize>() {
                    let e = next.entry(scope_key(o)).or_insert(0);
                    *e = (*e).max(n);
                }
            }
        }
    }
    let mut changed = 0usize;
    for (slot, n) in &slots {
        if n.id.is_some() {
            continue;
        }
        let (key, ptr) = match slot {
            Slot::Arch(i) => ("arch".to_string(), format!("/health/concerns/{i}")),
            Slot::Module(mi, ci) => {
                let mid = map["modules"].get(*mi).and_then(|m| m["id"].as_str()).unwrap_or_default().to_string();
                (mid.clone(), format!("/modules/{mi}/health/concerns/{ci}"))
            }
        };
        let e = next.entry(key.clone()).or_insert(0);
        *e += 1;
        let id = if key == "arch" { format!("c-arch-{e}") } else { format!("c-{key}-{e}") };
        if let Some(c) = map.pointer_mut(&ptr) {
            c["id"] = serde_json::Value::String(id);
            changed += 1;
        }
    }
    changed
}
