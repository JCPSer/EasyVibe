//! task_exec::harness —— harness 出厂层（只读圣物）+ 自定义层装载与透明模式中和（文件 IO + 纯函数）。
//!
//! 两层分离（2026-10-05，用户裁定）：
//! - 出厂层（~/.easyvibe/harness/）：**任何方式不动**——只在首次缺失时部署一份换姓参考副本
//!   （给框架内 cat 指令的落点），之后运行时零写入；装配事实源是编译期内嵌底账
//!   （assemble_from_builtin），磁盘出厂文件被改不影响任何装配产物；
//! - 自定义层（~/.easyvibe/harness-custom/）：用户可编辑的补充规则，五个 harness 类型
//!   （global 通用 / analysis 需求分析 / design 方案设计 / implement 代码开发 / review 代码审查）
//!   各加各的规则、只增不碰原规则；旧版四阶段共用槽 development 仅作新槽兜底；
//!   state.json 总开关（停用≠删除）。
//! 老版本用户对出厂层的修改经一次性迁移（哨兵防重入，只读播种）整体挪入自定义层。

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

/// 自定义层槽位：(文件名, state.json 键)。
/// 五个 harness 类型（global 通用 + analysis/design/implement/review）+ 旧版四阶段
/// 共用槽 development（用户裁定 18:10：各类型分别加规则；development 仅作新槽的兜底）。
pub const CUSTOM_SLOTS: &[(&str, &str)] = &[
    ("global.md", "global"),
    ("rule_analysis.md", "analysis"),
    ("rule_design.md", "design"),
    ("rule_implement.md", "implement"),
    ("rule_review.md", "review"),
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
    pub analysis: Option<String>,
    pub design: Option<String>,
    pub implement: Option<String>,
    pub review: Option<String>,
    /// 旧版四阶段共用槽：仅作新类型槽的兜底（新槽为空时回落用它）
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
    /// 出厂规则正文内嵌副本（审查 prompt 直接内嵌，agent 不再 cat 磁盘文件——
    /// 用户裁定 17:58：任何方式不动原 harness，磁盘参考副本被改也不影响审查行为）
    pub rule_development: String,
    pub rule_bugfix: String,
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

/// 出厂层首次部署（用户裁定 17:58：任何方式不动原 harness）：
/// **只在文件缺失时写一份换姓参考副本**（给框架内 cat 指令的落点），
/// 已存在的文件一律不碰——软件运行时对出厂层零写入，用户磁盘手改也不覆写。
pub fn deploy_sealed(dir: &std::path::Path) -> Result<(), ApiError> {
    for (rel, content) in BUILTIN_HARNESS {
        let p = dir.join(rel);
        if p.exists() {
            continue;
        }
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(format!("harness 目录创建失败: {e}")))?;
        }
        std::fs::write(&p, adapt_builtin_content(content))
            .map_err(|e| ApiError::Internal(format!("harness 底账写入失败 {}: {e}", p.display())))?;
    }
    Ok(())
}

/// 一次性迁移（§3.3，17:58 修订：**只读**出厂差异播 custom，不 rename、不重铺、不动原 harness）。
/// 哨兵防重入；播种"槽文件已存在则不覆盖"保证崩溃重入安全。
/// 原料读取：出厂目录优先，缺失回退最新 harness.backup-*（老版本归档形态）。
pub(crate) fn migrate_builtin_to_custom(factory_dir: &std::path::Path, custom_dir: &std::path::Path) {
    let sentinel = custom_dir.join(MIGRATION_SENTINEL);
    if sentinel.exists() {
        return;
    }
    let backup = latest_harness_backup(factory_dir);
    let read_old = |rel: &str| -> Option<String> {
        std::fs::read_to_string(factory_dir.join(rel)).ok()
            .or_else(|| backup.as_ref().and_then(|b| std::fs::read_to_string(b.join(rel)).ok()))
    };
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
        let Some(old) = read_old(rel) else { continue };
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
    // manifest / grill-me 的差异不在 custom 范围：仅日志提示
    if let Some(old) = read_old("manifest.json") {
        let builtin = BUILTIN_HARNESS.iter().find(|(r, _)| *r == "manifest.json").map(|(_, c)| *c).unwrap_or("");
        if old != builtin {
            info!("[harness] 迁移：manifest.json 的用户字段不入 custom 槽（可在 harness.backup-* 找回）");
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

/// harness 生产装载（17:58 修订：装配事实源 = 编译期内嵌底账，**不读磁盘出厂文件**；
/// 磁盘出厂层只是首次部署的参考副本，运行时零写入、零依赖）：
/// 首次部署（缺失才写）→ 迁移（只读播种）→ 内嵌装配 + custom 层装载
pub fn load_harness() -> Result<Harness, ApiError> {
    let dir = harness_dir();
    let custom_dir = custom_dir_for(&dir);
    deploy_sealed(&dir)?;
    migrate_builtin_to_custom(&dir, &custom_dir);
    assemble_from_builtin(&dir, &custom_dir)
}

/// 从内嵌底账装配（生产事实源）。manifest/框架/skills/规则正文全部来自 BUILTIN_HARNESS，
/// 只经过 adapt_builtin_content 换姓与 neutralize 中和——磁盘上的出厂文件被用户改动
/// 不影响任何装配产物。
pub(crate) fn assemble_from_builtin(dir: &std::path::Path, custom_dir: &std::path::Path) -> Result<Harness, ApiError> {
    let entry = |rel: &str| -> &'static str {
        BUILTIN_HARNESS.iter().find(|(r, _)| r == &rel).map(|(_, c)| *c).unwrap_or("")
    };
    let manifest: HarnessManifest = serde_json::from_str(entry("manifest.json"))
        .map_err(|e| ApiError::Internal(format!("内嵌 harness manifest 解析失败: {e}")))?;
    let adapted = entry("inject-prompt.md")
        .replace("~/.claude/hooks/", &format!("{}/", dir.to_string_lossy().trim_end_matches('/')));
    let framework_transparent = neutralize_transparent(&adapted, &manifest.transparent_neutralize);
    let mut user_entry_skills = vec![];
    for rel in &manifest.skills.user_entry {
        let content = entry(rel);
        if content.is_empty() {
            return Err(ApiError::Internal(format!("内嵌 user_entry 插槽缺失: {rel}")));
        }
        user_entry_skills.push(content.to_string());
    }
    let custom = load_custom(custom_dir);
    Ok(Harness {
        dir: dir.to_path_buf(),
        manifest,
        framework_transparent,
        user_entry_skills,
        custom,
        rule_development: adapt_builtin_content(entry("rule_development.md")),
        rule_bugfix: adapt_builtin_content(entry("rule_bugfix.md")),
    })
}

/// 从指定目录纯装载（**不 deploy、不迁移**——测试注入点：手工写 manifest/框架验证
/// 换姓与中和的语义；生产装配见 assemble_from_builtin）。
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
    Ok(Harness {
        dir: dir.to_path_buf(),
        manifest,
        framework_transparent,
        user_entry_skills,
        custom,
        rule_development: String::new(),
        rule_bugfix: String::new(),
    })
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
            "analysis" => custom.analysis = Some(adapted),
            "design" => custom.design = Some(adapted),
            "implement" => custom.implement = Some(adapted),
            "review" => custom.review = Some(adapted),
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
