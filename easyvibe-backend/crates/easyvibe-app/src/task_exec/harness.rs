//! task_exec::harness —— harness 出厂底账部署、装载与透明模式中和（文件 IO + 纯函数）。

use super::*;

/// 出厂 harness 只读底账：编译期内嵌（§12c 决策——reference/ 只是构建源，运行时唯一
/// 生效副本是数据目录的用户可编辑层；"改哪份才生效"的二义就此消灭）
/// 注意：内嵌的仍是 reference/ 原稿（含 .claude 路径）——换姓发生在写盘时
/// （adapt_builtin_content），原稿保持用户参考材料原样不动
pub const BUILTIN_HARNESS: &[(&str, &str)] = &[
    ("manifest.json", include_str!("../../../../../reference/manifest.json")),
    ("inject-prompt.md", include_str!("../../../../../reference/inject-prompt.md")),
    ("rule_development.md", include_str!("../../../../../reference/rule_development.md")),
    ("rule_bugfix.md", include_str!("../../../../../reference/rule_bugfix.md")),
    ("skills/grill-me/SKILL.md", include_str!("../../../../../reference/grill-me/SKILL.md")),
];

const TRANSPARENT_MODE_LINE: &str = "（透明执行模式：禁止向用户提问或要求确认；需求有歧义时按最合理假设直接执行，并在 [EASYVIBE-RESULT] 的 summary 中说明你做出的假设。）";

/// Harness manifest（§12c 边界定稿：控制面声明，装配层唯一需要解析的文件）
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessManifest {
    pub id: String,
    pub version: String,
    #[serde(default)] pub builtin: bool,
    #[serde(default)] pub route_rules: Vec<String>,
    #[serde(default)] pub skills: HarnessSkills,
    /// 透明装配时中和的指令模式（实弹#3 防线的配置化——从硬编码 grep 升级为 manifest 声明）
    #[serde(default)] pub transparent_neutralize: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSkills {
    /// user_entry 插槽：仅用户入口对话注入（§9 #4）——grill-me 挂在这里
    #[serde(default)] pub user_entry: Vec<String>,
    /// transparent 插槽：透明 agent（任务/归纳/巡检）——缺省空，不是文本删除
    #[serde(default)] pub transparent: Vec<String>,
}

/// 装载完成的 harness：三种装配产物同源不同形（同一个 manifest，两种装配产物）
#[derive(Clone)]
pub struct Harness {
    pub dir: PathBuf,
    pub manifest: HarnessManifest,
    /// 透明执行装配框架：路径换姓 + 按 manifest 中和拷问类指令（供任务 prompt）
    pub framework_transparent: String,
    /// user_entry 插槽 skill 正文（供对话 prompt；透明 agent 永不注入）
    pub user_entry_skills: Vec<String>,
}

pub fn harness_dir() -> PathBuf {
    match std::env::var("EASYVIBE_HARNESS_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            // Windows 无 HOME——USERPROFILE 兜底，都缺时落当前目录（独立 exe 场景 = exe 旁）
            let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())
                .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()))
                .unwrap_or_else(|| ".".into());
            PathBuf::from(format!("{home}/.easyvibe/harness"))
        }
    }
}

/// 出厂底账部署：缺失文件从内嵌底账补齐（写盘时路径换姓）；**不覆盖**用户已编辑的文件。
/// 版本迁移：内置 manifest 版本高于磁盘 → 三份规则正文仅在"磁盘内容仍等于出厂原稿"
/// （即用户未改动）时重写为换姓版；manifest 只抬版本号、保留用户其余字段。
/// 恢复默认（reset）走 deploy_builtin_force。
pub fn deploy_builtin(dir: &std::path::Path) -> Result<(), ApiError> {
    // 版本迁移判定（manifest 版本比较）
    let disk_ver = std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string));
    let builtin_ver = BUILTIN_HARNESS
        .iter()
        .find(|(rel, _)| *rel == "manifest.json")
        .and_then(|(_, c)| serde_json::from_str::<serde_json::Value>(c).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string));
    let migrate = match (&builtin_ver, &disk_ver) {
        (Some(b), Some(d)) => version_gt(b, d),
        // 磁盘无 manifest（首次部署）或版本不可解析：不触发迁移，走缺失补齐
        _ => false,
    };

    for (rel, content) in BUILTIN_HARNESS {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(format!("harness 目录创建失败: {e}")))?;
        }
        if !p.exists() {
            std::fs::write(&p, adapt_builtin_content(content))
                .map_err(|e| ApiError::Internal(format!("harness 底账写入失败 {}: {e}", p.display())))?;
            continue;
        }
        if !migrate {
            continue;
        }
        if *rel == "manifest.json" {
            // 只抬版本号：磁盘 manifest 的其余字段（用户可能加过 routeRules）保留
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&p).unwrap_or_default()) {
                if let (Some(obj), Some(bv)) = (v.as_object_mut(), &builtin_ver) {
                    obj.insert("version".into(), serde_json::Value::String(bv.clone()));
                    if let Ok(s) = serde_json::to_string_pretty(&v) {
                        let _ = std::fs::write(&p, s);
                    }
                }
            }
            continue;
        }
        // 规则正文：磁盘仍等于出厂原稿 = 未改动 → 重写为换姓版；已改动则保留用户版
        let disk = std::fs::read_to_string(&p).unwrap_or_default();
        if disk == *content {
            std::fs::write(&p, adapt_builtin_content(content))
                .map_err(|e| ApiError::Internal(format!("harness 换姓重写失败 {}: {e}", p.display())))?;
        }
    }
    Ok(())
}

/// 恢复默认：全量覆盖用户层（与 deploy_builtin 的"缺失才补"语义相反）；同样写盘时换姓
pub fn deploy_builtin_force(dir: &std::path::Path) -> Result<(), ApiError> {
    for (rel, content) in BUILTIN_HARNESS {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(format!("harness 目录创建失败: {e}")))?;
        }
        std::fs::write(&p, adapt_builtin_content(content))
            .map_err(|e| ApiError::Internal(format!("harness 底账写入失败 {}: {e}", p.display())))?;
    }
    Ok(())
}

/// harness 装载（§12c 插槽内核）：
/// 1) 底账补齐（缺失才写）→ 2) 解析 manifest → 3) 透明框架=路径换姓+按 manifest 中和
/// 4) user_entry 插槽正文读取。装配层唯一解析 manifest，正文如何演化与防线解耦
/// （实弹#3 教训：grep 硬编码与框架文本演化会漂移）。
pub fn load_harness() -> Result<Harness, ApiError> {
    let dir = harness_dir();
    load_harness_from(&dir)
}

/// 从指定目录装载（测试注入点——避免 env 变量在并行测试间的竞态）
pub fn load_harness_from(dir: &std::path::Path) -> Result<Harness, ApiError> {
    deploy_builtin(dir)?;
    let manifest: HarnessManifest = serde_json::from_str(
        &std::fs::read_to_string(dir.join("manifest.json"))
            .map_err(|e| ApiError::Internal(format!("harness manifest 不可读: {e}")))?,
    )
    .map_err(|e| ApiError::Internal(format!("harness manifest 解析失败: {e}")))?;
    let framework = std::fs::read_to_string(dir.join("inject-prompt.md"))
        .map_err(|e| ApiError::Internal(format!("harness 框架不可读: {e}")))?;
    // 路径换姓：框架内引用的规则正文位置指向本机 harness 目录
    let adapted = framework.replace("~/.claude/hooks/", &format!("{}/", dir.to_string_lossy().trim_end_matches('/')));
    let framework_transparent = neutralize_transparent(&adapted, &manifest.transparent_neutralize);
    let mut user_entry_skills = vec![];
    for rel in &manifest.skills.user_entry {
        let p = dir.join(rel);
        let content = std::fs::read_to_string(&p)
            .map_err(|e| ApiError::Internal(format!("user_entry 插槽文件不可读 {}: {e}", p.display())))?;
        user_entry_skills.push(content);
    }
    Ok(Harness { dir: dir.to_path_buf(), manifest, framework_transparent, user_entry_skills })
}

/// 透明执行中和：命中 manifest 声明模式的行替换为透明执行指令（§9 #4 对齐）
pub(crate) fn neutralize_transparent(text: &str, patterns: &[String]) -> String {
    text.lines()
        .map(|l| {
            if patterns.iter().any(|p| !p.is_empty() && l.to_lowercase().contains(&p.to_lowercase())) {
                TRANSPARENT_MODE_LINE
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
