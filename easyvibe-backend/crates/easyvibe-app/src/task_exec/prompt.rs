//! task_exec::prompt —— 阶段 prompt 拼装与 harness 文案路径适配（纯函数，无状态机依赖）。

use super::*;

/// 打回反馈注入 context.remediation（decide 打回与 remediate 复审共用）：
/// round 在既有值上累加，review_feedback/instruction 整段替换——新一轮打回覆盖上一轮。
/// 返回新的 context JSON 字符串（保留既有其他键）。
pub(crate) fn inject_remediation(context_json: &str, feedback: &str, instruction: &str) -> String {
    let mut ctx: serde_json::Value = serde_json::from_str(context_json).unwrap_or_else(|_| serde_json::json!({}));
    let round = ctx["remediation"]["round"].as_u64().unwrap_or(0) + 1;
    ctx["remediation"] = serde_json::json!({
        "round": round,
        "review_feedback": feedback,
        "instruction": instruction,
    });
    serde_json::to_string(&ctx).unwrap_or_else(|_| "{}".into())
}

/// 打回反馈展示段：context.remediation 存在时拼进阶段 prompt（重跑带着意见做），否则空串。
pub(crate) fn remediation_section(context_json: &str) -> String {
    let ctx: serde_json::Value = serde_json::from_str(context_json).unwrap_or_else(|_| serde_json::json!({}));
    match (ctx["remediation"]["round"].as_u64(), ctx["remediation"]["review_feedback"].as_str()) {
        (Some(round), Some(fb)) if !fb.is_empty() => format!(
            "\n## 打回反馈（第 {round} 轮）\n{fb}\n\n处置要求：{}\n",
            ctx["remediation"]["instruction"].as_str().unwrap_or("")
        ),
        _ => String::new(),
    }
}

/// 自定义补充块（方案 v2 §4）：有内容时带固定引导句追加；空槽/停用 = 空串，
/// 装配产物与无 custom 时逐字节一致（无回归契约）。槽内容已在装载时做过路径换姓。
pub(crate) fn custom_block(c: &Option<String>) -> String {
    match c.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => format!("\n\n{}\n{}\n", CUSTOM_BLOCK_HEADER, t),
        _ => String::new(),
    }
}

/// 类型槽选取：新类型槽有内容用新槽，否则回落旧版四阶段共用槽 development（用户裁定 18:10 的兼容语义）
fn stage_slot<'a>(slot: &'a Option<String>, legacy: &'a Option<String>) -> &'a Option<String> {
    match slot.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => slot,
        _ => legacy,
    }
}

/// 五个 harness 类型的补充块（用户裁定 18:10：analysis/design/implement/review 各加各的规则、不混淆）：
/// global=全部上下文；analysis=阶段1 需求分析；design=阶段2 方案设计；
/// implement=阶段3 实施；review=审查+初审。每块 = global + 该类型槽（空则回落 development 兜底槽）。
pub(crate) fn blocks_analysis(custom: &HarnessCustom) -> String {
    format!("{}{}", custom_block(&custom.global), custom_block(stage_slot(&custom.analysis, &custom.development)))
}
pub(crate) fn blocks_design(custom: &HarnessCustom) -> String {
    format!("{}{}", custom_block(&custom.global), custom_block(stage_slot(&custom.design, &custom.development)))
}
pub(crate) fn blocks_implement(custom: &HarnessCustom) -> String {
    format!("{}{}", custom_block(&custom.global), custom_block(stage_slot(&custom.implement, &custom.development)))
}
pub(crate) fn blocks_review(custom: &HarnessCustom) -> String {
    format!("{}{}", custom_block(&custom.global), custom_block(stage_slot(&custom.review, &custom.development)))
}
/// 初审（阶段产物预筛）：同时对照需求与方案两类产物，取 analysis + design 两槽
pub(crate) fn blocks_phase_review(custom: &HarnessCustom) -> String {
    format!(
        "{}{}{}",
        custom_block(&custom.global),
        custom_block(stage_slot(&custom.analysis, &custom.development)),
        custom_block(stage_slot(&custom.design, &custom.development)),
    )
}

/// 阶段 1 prompt：只产出需求矩阵，禁改代码（rule_development 2.1）。
/// 产出后由用户在 analysis 关评审——评审通过才进阶段 2。
/// custom：自定义层补充（global + development 块，方案 v2 §4 注入点 #2）。
pub(crate) fn assemble_phase1_prompt(task: &TaskRecord, user: &str, custom: &HarnessCustom) -> String {
    let modules: Vec<String> = serde_json::from_str(&task.modules).unwrap_or_default();
    format!(
        r#"你是需求分析 agent（harness 规则正文 2.1 的执行者）。**只做需求分析，禁止修改任何代码文件。**

## 任务书
- 需求描述：{description}
- 影响模块：{modules}
- 验收标准：{acceptance}
{feedback}
## 要求
1. 研读仓库代码与 .easyvibe/map/map.json，将需求拆解为需求矩阵（背景、目标、描述、优先级、难度、风险）。
2. 需求矩阵写入 .easyvibe/development_docs/{user}/1_requirements_matrix/<yyyy-MM-dd-hh-mm>-<brief>.md，
   文档必须含「评审意见栏」（留空待用户填写）。
3. 更新同目录 INDEX-<yyyy-MM>.md 索引。
4. 不要实施任何代码改动——实施发生在用户评审通过之后。
5. 最后一行输出：[EASYVIBE-RESULT] {{"summary":"需求矩阵已产出：一句话概括","changed_modules":[]}}{custom}"#,
        description = task.description,
        modules = if modules.is_empty() { "（未指定，由你分析）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        feedback = remediation_section(&task.context),
        custom = blocks_analysis(custom),
        user = user,
    )
}

/// 阶段 2 prompt：只产出方案设计，禁改代码（rule_development 2.2）。
/// 基于已评审通过的需求矩阵；产出后由用户在 solution 关评审——通过才进阶段 3 实施。
pub(crate) fn assemble_phase2_prompt(task: &TaskRecord, user: &str, custom: &HarnessCustom) -> String {
    let modules: Vec<String> = serde_json::from_str(&task.modules).unwrap_or_default();
    format!(
        r#"你是方案设计 agent（harness 规则正文 2.2 的执行者）。**只做方案设计，禁止修改任何代码文件。**

## 任务书
- 需求描述：{description}
- 影响模块：{modules}
- 验收标准：{acceptance}

## 输入
- 已评审通过的需求矩阵：.easyvibe/development_docs/{user}/1_requirements_matrix/ 下最新文档（先读它）
{feedback}
## 要求
1. 针对需求矩阵中的每个子需求进行方案设计：背景、目标、描述、详细方案设计。
2. 方案用伪代码或流程图表述，**禁止大段真实代码**（保证方案可读性）。
3. 涉及 UI 变动的部分必须包含 UI 设计说明。
4. 方案写入 .easyvibe/development_docs/{user}/2_requirements_solutions/<yyyy-MM-dd-hh-mm>-<brief>.md，
   文档必须含「评审意见栏」（留空待用户填写）。
5. 更新同目录 INDEX-<yyyy-MM>.md 索引。
6. 不要实施任何代码改动——实施发生在用户评审通过之后。
7. 最后一行输出：[EASYVIBE-RESULT] {{"summary":"方案设计已产出：一句话概括","changed_modules":[]}}{custom}"#,
        description = task.description,
        modules = if modules.is_empty() { "（未指定，由你分析）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        feedback = remediation_section(&task.context),
        custom = blocks_design(custom),
        user = user,
    )
}

/// 独立子 agent 代码审查（用户裁定「全做」B 案，harness 2.3.2 的独立可信落地）：
/// 实施完成后、diff 关之前，spawn 一个审查会话——只读 diff + 架构规则，产出审查报告与结论。
/// 报告落 `.easyvibe/development_docs/<user>/3_test_results/review-<task_id>.md`
/// （与 A 案协议同一规范路径，dev-docs 端点自动捞取）。
/// 结论行协议：`[EASYVIBE-REVIEW] {"verdict":"pass|fail","summary":"一句话"}`
/// 审查自身失败/超时 → None（不阻断：diff 关照常，人机审查兜底）。
/// 出厂规则正文直接内嵌（rules = (rule_development, rule_bugfix) 的内嵌副本——
/// 17:58 裁定：agent 不再 cat 磁盘文件，磁盘出厂副本被改不影响审查）。
pub(crate) fn assemble_review_prompt(task: &TaskRecord, user: &str, custom: &HarnessCustom, rules: (&str, &str)) -> String {
    let modules: Vec<String> = serde_json::from_str(&task.modules).unwrap_or_default();
    format!(
        r#"你是独立代码审查 agent（harness 2.3.2 的执行者），只做审查，不做实现。
被审任务的实施刚完成，工作区的未提交改动就是它的产出。

## 被审任务
- 需求描述：{description}
- 影响模块：{modules}
- 验收标准：{acceptance}

## 审查材料
- 改动全文：工作目录即仓库根目录，执行 `git diff` 查看（不要 git checkout/stash 等任何写操作）
- 模块职责与边界：.easyvibe/map/map.json

## 审查依据的出厂规则（内嵌副本，按被审任务性质对照其一）
### 「功能开发」规则（rule_development）
{rule_dev}

### 「Bug 修复」规则（rule_bugfix）
{rule_fix}

## 审查维度（逐项给出结论）
代码规范、代码结构、可读性、可维护性、性能；对照影响面合约检查越界改动；
对照验收标准检查完整性。**除写审查报告外，禁止修改任何文件。**

## 输出（两者都必须）
1. 审查报告写入 .easyvibe/development_docs/{user}/3_test_results/review-{task_id}.md
   （含明确的审查意见：通过 / 打回 + 理由；发现问题逐条列出）
2. 最后一行输出：[EASYVIBE-REVIEW] {{"verdict":"pass","summary":"一句话结论"}}
   verdict 只能是 pass 或 fail；有任一阻断性问题必须 fail。{custom}"#,
        description = task.description,
        modules = if modules.is_empty() { "（未指定）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        user = user,
        task_id = task.id,
        rule_dev = rules.0,
        rule_fix = rules.1,
        custom = blocks_review(custom),
    )
}

/// 阶段初审 prompt：只审不改（禁止修改文件），按修改时间找最新产物文档通读，
/// 对照任务书核质量，最后一行输出 [EASYVIBE-REVIEW] 结论。
pub(crate) fn assemble_phase_review_prompt(task: &TaskRecord, phase: u8, user: &str, custom: &HarnessCustom) -> String {
    let (name, dir, focus) = if phase == 1 {
        (
            "需求矩阵",
            format!(".easyvibe/development_docs/{user}/1_requirements_matrix/"),
            "- 覆盖度：任务描述里的每个诉求都有对应需求条目吗？有没有漏项？\n\
             - 可测性：每条验收标准是否可判定（有明确的完成口径，而非'优化/提升'这类模糊词）？\n\
             - 无歧义：需求条目之间是否自洽，有没有互相矛盾或重复？",
        )
    } else {
        (
            "方案设计",
            format!(".easyvibe/development_docs/{user}/2_requirements_solutions/"),
            "- 对齐性：方案是否逐条回应了需求矩阵（R 编号可回溯）？有没有矩阵里的需求被方案漏掉？\n\
             - 可行性：改动路径与当前代码结构是否矛盾（是否引用了不存在的文件/接口）？\n\
             - 风险：方案有没有明显的回归风险未给验证手段？",
        )
    };
    format!(
        r#"你是 EasyVibe 的阶段产物审查 agent。**只审不改：禁止创建/修改/删除任何文件**。

## 任务书（审查的对照基准）
- 需求描述：{description}
- 验收标准：{acceptance}

## 待审产物
{name}，目录：{dir}（该目录下修改时间最新的 .md 文档——用 ls -t 找到它并完整读取）

## 审查要点
{focus}

## 输出
审查过程不需要长篇大论；最后一行必须严格是：
[EASYVIBE-REVIEW] {{"verdict":"pass 或 fail","summary":"一句话结论（≤80 字：通过理由或关键问题）"}}{custom}"#,
        description = task.description,
        acceptance = if task.acceptance.is_empty() { "（未指定——按需求描述推断合理验收口径）".into() } else { task.acceptance.clone() },
        name = name,
        dir = dir,
        focus = focus,
        custom = blocks_phase_review(custom),
    )
}

/// 组装任务执行 prompt：harness 框架（路径适配）+ 自定义补充（global 块，方案 v2 §4
/// 注入点 #1——framework 后、任务书前）+ 表单字段 + 事前注入上下文。
/// 路由/拷问/豁免全交给 LLM 决断（§9 #3/#5）。
pub fn assemble_task_prompt(harness: &Harness, task: &TaskRecord) -> String {
    let modules = serde_json::from_str::<Vec<String>>(&task.modules).unwrap_or_default();
    format!(
        r#"{framework}{custom}

---

## 本次任务（来自 EasyVibe 结构化任务表单）

- 需求描述：{description}
- 影响模块：{modules}
- 验收标准：{acceptance}

## 事前注入上下文（模块职责 / 边界 / 问题 / 违规边——据此把握结构与边界）

```json
{context}
```

## 已评审通过的输入（分阶段评审的产物——先读再动手，严格按方案实施）

- 需求矩阵：.easyvibe/development_docs/{user}/1_requirements_matrix/ 下最新文档
- 方案设计：.easyvibe/development_docs/{user}/2_requirements_solutions/ 下最新文档（若有）

## 执行要求

- 工作目录即仓库根目录；框架与规则正文中的路径已适配到本机，直接 cat 读取。
- **严格按已评审通过的方案实施**，不要偏离方案另作设计；发现方案有硬伤时停下来在 [EASYVIBE-RESULT] 的 summary 中说明。
- 改动规模评估与是否走完整评审流程由你决断（框架内的豁免条款），全程留痕。
- 统计/验证类结论用工具数准，禁止估算。
- 实施完成后对接口/界面进行测试，测试结果写入 .easyvibe/development_docs/{user}/3_test_results/（规范路径，含 INDEX 索引）——对应规则正文 2.3.1。
- 代码审查（规则正文 2.3.2）由系统独立审查 agent 执行，你无需自审；不要伪造审查结论。
- 完成后最后一行输出：`[EASYVIBE-RESULT] {{"summary": "一句话总结", "changed_modules": ["模块id"]}}` 便于系统归档。"#,
        framework = harness.framework_transparent,
        custom = custom_block(&harness.custom.global),
        description = task.description,
        modules = if modules.is_empty() { "（未指定，由你分析）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        context = task.context,
        user = std::env::var("USER").unwrap_or_else(|_| "default".into()),
    )
}

/// harness 产物路径换姓（方案 v3 §4.4）：三种 .claude 形态 → .easyvibe 自有路径。
/// 顺序敏感：先长后短，`<user_name>` 兜底最后；`~/.claude/hooks/` 是受保护的
/// load 时替换锚点（task_exec.rs load_harness_from），换姓前先占位保护、最后还原。
/// 1) `.claude/<user_name>/development_docs` → `.easyvibe/development_docs/<user>`
///    （inject-prompt.md 的 task.json 路径，形态与 2 不同，漏换则 task.json 写回旧位置）
/// 2) `.claude/development_docs` → `.easyvibe/development_docs`（规则正文 20+ 处）
/// 3) 裸 `.claude/` → `.easyvibe/`（rule_development.md 附录目录树根行，复审残留）
/// 4) `<user_name>` → 本机用户名（剩余形态兜底，取不到用 default）
pub fn adapt_builtin_content(content: &str) -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    const HOOKS_ANCHOR: &str = "\u{1}HOOKS\u{1}";
    content
        .replace("~/.claude/hooks/", HOOKS_ANCHOR)
        .replace(".claude/<user_name>/development_docs", &format!(".easyvibe/development_docs/{user}"))
        .replace(".claude/development_docs", ".easyvibe/development_docs")
        .replace(".claude/", ".easyvibe/")
        .replace("<user_name>", &user)
        .replace(HOOKS_ANCHOR, "~/.claude/hooks/")
}
