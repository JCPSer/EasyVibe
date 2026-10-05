//! task_exec::harness —— harness 出厂层密封部署、一次性迁移、自定义层装载与透明模式中和（文件 IO + 纯函数）。
//!
//! 两层分离（2026-10-05 方案 v2，用户裁定）：
//! - 出厂层（~/.easyvibe/harness/）：编译期内嵌底账的换姓形态，**密封**——每次启动幂等
//!   全量覆写，不对用户展示、不可修改（deploy_sealed）；
//! - 自定义层（~/.easyvibe/harness-custom/）：用户可编辑的补充规则（global.md /
//!   rule_development.md 两槽），只追加不修改，带独立开关（state.json）。
//! 老版本用户对出厂层的修改经一次性迁移（哨兵防重入）整体挪入自定义层。

use super::*;

/// 出厂 harness 只读底账：编译期内嵌。注意：内嵌的仍是 reference/ 原稿（含 .claude
/// 路径与 <user_name>）——换姓发生在写盘/装载时（adapt_builtin_content），原稿保持不动。
pub const BUILTIN_HARNESS: &[(&str, &str)] = &[
    ("manifest.json", include_str!("../../../../../reference/manifest.json")),
    ("inject-prompt.md", include_str!("../../../../../reference/inject-prompt.md")),
    ("rule_development.md", include_str!("../../../../../reference/rule_development.md")),
    ("rule_bugfix.md", include_str!("../../../../../reference/rule_bugfix.md")),
    ("skills/grill-me/SKILL.md", include_str!("../../../../../reference/grill-me/SKILL.md")),
];

const TRANSPARENT_MODE_LINE: &str = "（透明执行模式：禁止向用户提问或要求确认；需求有歧义时按最合理假设直接执行，并在 [EASYVIBE-RESULT] 的 summary 中说明你做出的假设。）";

/// 自定义补充引导句（装配约定：与出厂冲突时以补充为准）
pub const CUSTOM_BLOCK_HEADER: &str = "【用户补充规则——优先级高于出厂规则，与出厂冲突时以补充为准】";

/// 自定义层槽位：(文件名, state.json 键)。bugfix 槽待任务类型字段落地后追加。
pub const CUSTOM_SLOTS: &[(&str, &str)] = &[
    ("global.md", "global"),
    ("rule_development.md", "development"),
];

/// 单槽大小上限（防 prompt 爆炸）
const CUSTOM_MAX_BYTES: u64 = 64 * 1024;

/// 一次性迁移完成哨兵（harness-custom/ 内）
const MIGRATION_SENTINEL: &str = ".migrated-v2";

/// 自定义层装载产物：None = 无内容或已停用（装配层统一表现）
#[derive(Debug, Clone, Default)]
pub struct HarnessCustom {
    pub global: Option<String>,
    pub development: Option<String>,
}

/// Harness manifest（控制面声明，装配层唯一需要解析的文件）
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
    /// 自定义层（harness-custom/）：仅启用的槽有内容
    pub custom: HarnessCustom,
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

/// 自定义层目录：出厂目录的兄弟（~/.easyvibe/harness-custom/）
pub fn harness_custom_dir() -> PathBuf {
    custom_dir_for(&harness_dir())
}

fn custom_dir_for(factory_dir: &std::path::Path) -> PathBuf {
    factory_dir.with_file_name("harness-custom")
}

/// 出厂层密封部署（v2 语义，替代旧"缺失才补 + 版本迁移"）：幂等全量覆写。
/// 比较基准 = 换姓后的底账（磁盘稳态就是换姓形态；手工写入未换姓原文也必然不等，照覆写）。
pub fn deploy_sealed(dir: &std::path::Path) -> Result<(), ApiError> {
    for (rel, content) in BUILTIN_HARNESS {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(format!("harness 目录创建失败: {e}")))?;
        }
        let expected = adapt_builtin_content(content);
        let same = std::fs::read_to_string(&p).map(|d| d == expected).unwrap_or(false);
        if !same {
            std::fs::write(&p, &expected)
                .map_err(|e| ApiError::Internal(format!("harness 底账写入失败 {}: {e}", p.display())))?;
        }
    }
    Ok(())
}

/// 一次性迁移（§3.3）：老版本用户对出厂层的修改 → 整体挪入自定义层。
/// 哨兵防重入；播种"槽文件已存在则不覆盖"保证崩溃重入安全。
/// 步骤 2 内部窗口（rename 后重铺前崩溃）：重入时出厂目录缺失 → tampered 判定重铺；
/// 播种原料回退读最新 harness.backup-*。
pub(crate) fn migrate_builtin_to_custom(factory_dir: &std::path::Path, custom_dir: &std::path::Path) {
    let sentinel = custom_dir.join(MIGRATION_SENTINEL);
    if sentinel.exists() {
        return;
    }
    let backup = latest_harness_backup(factory_dir);
    // 播种原料必须在 rename/重铺**之前**读进内存——重铺后出厂目录里已是新内容
    // （步骤 2 内部崩溃窗口：目录缺失时回退读最新备份，旧内容仍找得回）
    let read_old = |rel: &str| -> Option<String> {
        std::fs::read_to_string(factory_dir.join(rel)).ok()
            .or_else(|| backup.as_ref().and_then(|b| std::fs::read_to_string(b.join(rel)).ok()))
    };
    let old_contents: Vec<(String, Option<String>)> = BUILTIN_HARNESS.iter().map(|(rel, _)| (rel.to_string(), read_old(rel))).collect();
    // 出厂层是否偏离出厂内容（文件缺失 = 偏离；目录缺失 = 偏离，须重铺）
    let factory_tampered = !factory_dir.exists() || BUILTIN_HARNESS.iter().any(|(rel, content)| {
        match std::fs::read_to_string(factory_dir.join(rel)) {
            Ok(d) => d != adapt_builtin_content(content),
            Err(_) => true,
        }
    });
    if factory_tampered {
        if factory_dir.exists() {
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let backup_path = factory_dir.with_file_name(format!("harness.backup-{ts}"));
            if let Err(e) = std::fs::rename(factory_dir, &backup_path) {
                warn!("[harness] 迁移归档失败（继续重铺，旧内容可能丢失）: {e}");
            } else {
                info!("[harness] 迁移：旧出厂层已归档到 {}", backup_path.display());
            }
        }
        if let Err(e) = deploy_sealed(factory_dir) {
            warn!("[harness] 迁移重铺失败: {e}");
        }
    }
    // 播种（不覆盖既有槽文件——崩溃重入也不重复播种）
    const SEEDS: &[(&str, &str)] = &[
        ("inject-prompt.md", "global.md"),
        ("rule_development.md", "rule_development.md"),
    ];
    for (rel, slot) in SEEDS {
        let slot_path = custom_dir.join(slot);
        if slot_path.exists() {
            continue;
        }
        let old = old_contents.iter().find(|(r, _)| r == rel).and_then(|(_, c)| c.clone());
        let Some(old) = old else { continue };
        let differs = BUILTIN_HARNESS
            .iter()
            .find(|(r, _)| r == rel)
            .map(|(_, c)| old != adapt_builtin_content(c))
            .unwrap_or(true);
        if differs {
            let _ = std::fs::create_dir_all(custom_dir);
            match std::fs::write(&slot_path, &old) {
                Ok(_) => info!("[harness] 迁移：{} 的用户修改已播种到 {}", rel, slot),
                Err(e) => warn!("[harness] 迁移播种失败 {}: {e}", slot_path.display()),
            }
        }
    }
    // manifest / grill-me 的差异不在 custom 范围：提示但不播种
    let old_manifest = old_contents.iter().find(|(r, _)| r == "manifest.json").and_then(|(_, c)| c.clone());
    if let Some(old) = old_manifest {
        let builtin = BUILTIN_HARNESS.iter().find(|(r, _)| *r == "manifest.json").map(|(_, c)| *c).unwrap_or("");
        if old != builtin {
            info!("[harness] 迁移：manifest.json 的用户字段随出厂重置（可在 harness.backup-* 找回）");
        }
    }
    let _ = std::fs::create_dir_all(custom_dir);
    if let Err(e) = std::fs::write(&sentinel, b"") {
        warn!("[harness] 迁移哨兵写入失败（下次启动将重入，播种不覆盖故安全）: {e}");
    }
}

/// 出厂目录的兄弟中凡 harness.backup-* 都算备份，按名倒序取最新（名字含时间戳）
fn latest_harness_backup(factory_dir: &std::path::Path) -> Option<PathBuf> {
    let parent = factory_dir.parent()?;
    let stem = factory_dir.file_name()?.to_str()?;
    let mut best: Option<PathBuf> = None;
    for e in std::fs::read_dir(parent).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with(&format!("{stem}.backup-")) && e.path().is_dir() && best.as_ref().map(|b| b.file_name().unwrap_or_default() < e.file_name()).unwrap_or(true) {
            best = Some(e.path());
        }
    }
    best
}

/// harness 生产装载：迁移（哨兵短路）→ 密封自检重铺 → 纯装载
pub fn load_harness() -> Result<Harness, ApiError> {
    let dir = harness_dir();
    let custom_dir = custom_dir_for(&dir);
    migrate_builtin_to_custom(&dir, &custom_dir);
    deploy_sealed(&dir)?;
    load_harness_from_parts(&dir, &custom_dir)
}

/// 从指定目录纯装载（**不 deploy、不迁移**——测试注入点，避免 env 变量在并行测试间的竞态
/// 与"手工写入的文件被密封覆写"的相互干扰）。v2 拆分：生产入口 load_harness 才做密封。
pub fn load_harness_from(dir: &std::path::Path) -> Result<Harness, ApiError> {
    load_harness_from_parts(dir, &custom_dir_for(dir))
}

fn load_harness_from_parts(dir: &std::path::Path, custom_dir: &std::path::Path) -> Result<Harness, ApiError> {
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
    let custom = load_custom(custom_dir);
    Ok(Harness { dir: dir.to_path_buf(), manifest, framework_transparent, user_entry_skills, custom })
}

/// 自定义层装载：state.json 控制各槽开关（缺失默认启用）；停用或缺文件 = None。
/// 内容过一遍 adapt_builtin_content（用户从旧文档抄 .claude 路径同样生效）；64KB 超限拒载不阻断。
fn load_custom(custom_dir: &std::path::Path) -> HarnessCustom {
    let state = read_custom_state(custom_dir);
    let mut custom = HarnessCustom::default();
    for (file, key) in CUSTOM_SLOTS {
        let enabled = state[key].as_bool().unwrap_or(true);
        if !enabled {
            continue;
        }
        let p = custom_dir.join(file);
        let Ok(meta) = std::fs::metadata(&p) else { continue };
        if meta.len() > CUSTOM_MAX_BYTES {
            warn!("[harness] 自定义槽 {} 超限（{}B > {}B），拒载", file, meta.len(), CUSTOM_MAX_BYTES);
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&p) else { continue };
        let adapted = adapt_builtin_content(&content);
        match *key {
            "global" => custom.global = Some(adapted),
            "development" => custom.development = Some(adapted),
            _ => {}
        }
    }
    custom
}

/// 读 custom 层开关状态（state.json；缺失/损坏返回空对象 = 全部默认启用）
pub fn read_custom_state(custom_dir: &std::path::Path) -> serde_json::Value {
    std::fs::read_to_string(custom_dir.join("state.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// 写 custom 层开关（toggle 端点用；不热装载——由端点层完成，与既有 put 同纪律）
pub fn write_custom_state(custom_dir: &std::path::Path, state: &serde_json::Value) -> Result<(), ApiError> {
    std::fs::create_dir_all(custom_dir).map_err(|e| ApiError::Internal(format!("harness-custom 目录创建失败: {e}")))?;
    let s = serde_json::to_string_pretty(state).map_err(|e| ApiError::Internal(format!("开关状态序列化失败: {e}")))?;
    std::fs::write(custom_dir.join("state.json"), s).map_err(|e| ApiError::Internal(format!("开关状态写入失败: {e}")))
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
