#!/usr/bin/env python3
"""架构事实与构建期判据的唯一受版本控制实现（c-arch-12 / c-map-toolchain-2）。

- 策略装载：scripts/map_edge_policy.json（受版本控制，可审计 / 可 diff / CI 可见）。
- 判据实现：must_drop 命中、Tarjan SCC、跨层 SCC。
- 架构事实读取（v2）：layers / modules(glob 表) / edges(期望边数·指纹·退役 id) /
  gates(DV·SCC·壳依赖·退役 concern) / granularity(守卫 caps·白名单) —— 全部同源读取。
- 共享构件：glob_match（唯一 glob 语义实现；三分守卫与探针归属判据共用，不再各自复制）。
- 消费方：run/easyvibe_map_cli.py（finalize 关口 + GATE-1 同代闸门 + normalize-edges）、
  scripts/verify_map_acyclic.py、scripts/verify_arch_facts.py、scripts/gen_map_fixture.py、
  scripts/check_*_granularity.py / check_host_boundary.py（粒度 caps·白名单）。

fail-closed：新增字段一律经 require() 读取，缺失即抛 PolicyMissing（绝不静默回退到硬编码默认值
——否则迁移期又会出现「两份事实」）。调用方 try/except 后转 ok:false。
"""
import hashlib
import json
import os
import re
import sys

POLICY_REL = os.path.join("scripts", "map_edge_policy.json")


class PolicyMissing(Exception):
    """策略缺少必需字段（fail-closed）：字段名随异常携带。"""

    def __init__(self, dotted_key):
        self.dotted_key = dotted_key
        super().__init__("policy missing required field: %s" % dotted_key)


def policy_path(root: str) -> str:
    return os.path.join(root, POLICY_REL)


def load_policy(root: str) -> dict:
    p = policy_path(root)
    if not os.path.isfile(p):
        return {"version": 1, "must_drop": [], "observe": []}
    with open(p, encoding="utf-8") as fh:
        return json.load(fh)


def must_drop_pairs(policy: dict):
    """{ (from, to): reason } —— 构建期产物托管/内嵌，不得建模为依赖边。"""
    return {(e["from"], e["to"]): e.get("reason", "") for e in policy.get("must_drop", [])}


def observe_pairs(policy: dict):
    return {(e["from"], e["to"]): e.get("reason", "") for e in policy.get("observe", [])}


def policy_violations(edges, policy: dict):
    """返回命中 must_drop 的边 [(id, from, to)]（交付态期望空）。"""
    md = must_drop_pairs(policy)
    hits = []
    for e in edges:
        if (e.get("from"), e.get("to")) in md:
            hits.append((e.get("id"), e.get("from"), e.get("to")))
    return hits


def tarjan_scc(node_ids, edges):
    """Tarjan 强连通分量，返回 list[list[id]]（与前端 depsAnalysis.ts::tarjanSCC 同口径）。"""
    adj = {n: [] for n in node_ids}
    for e in edges:
        if e.get("from") in adj and e.get("to") in adj:
            adj[e["from"]].append(e["to"])
    index, low, on, stack, out, counter = {}, {}, {}, [], [], [0]
    sys.setrecursionlimit(max(10000, len(node_ids) * 10 + 1000))

    def strong(v):
        index[v] = low[v] = counter[0]
        counter[0] += 1
        stack.append(v)
        on[v] = True
        for w in adj[v]:
            if w not in index:
                strong(w)
                low[v] = min(low[v], low[w])
            elif on.get(w):
                low[v] = min(low[v], index[w])
        if low[v] == index[v]:
            comp = []
            while True:
                w = stack.pop()
                on[w] = False
                comp.append(w)
                if w == v:
                    break
            out.append(comp)

    for v in node_ids:
        if v not in index:
            strong(v)
    return out


def scc_groups(node_ids, edges):
    """仅返回成员数 > 1 的分量。"""
    return [sorted(c) for c in tarjan_scc(node_ids, edges) if len(c) > 1]


def cross_layer_scc(node_ids, edges, layer_order):
    """成员层序不全相等的 SCC（跨层环）；同层互引不算。

    layer_order: { node_id: order:int }
    """
    groups = []
    for g in scc_groups(node_ids, edges):
        orders = {layer_order.get(n) for n in g}
        if len(orders) > 1:
            groups.append(g)
    return groups


# ---------------------------------------------------------------- 架构事实读取 API（v2）
def require(policy: dict, dotted_key: str):
    """按点号路径取必需字段；缺失即抛 PolicyMissing（fail-closed，不提供静默默认值）。"""
    cur = policy
    for part in dotted_key.split("."):
        if not isinstance(cur, dict) or part not in cur:
            raise PolicyMissing(dotted_key)
        cur = cur[part]
    return cur


def module_ids(policy: dict):
    """规范化模块 id 列表（按 emit_order 升序）。"""
    mods = require(policy, "modules")
    return [m["id"] for m in sorted(mods, key=lambda m: require(m, "emit_order"))]


def modules_map(policy: dict):
    """{ id: {name, layer, emit_order, files(sorted/dedup)} } —— 唯一规范网格形态。"""
    out = {}
    for m in require(policy, "modules"):
        out[m["id"]] = {
            "name": require(m, "name"),
            "layer": require(m, "layer"),
            "emit_order": require(m, "emit_order"),
            "files": sorted(set(require(m, "files"))),
        }
    return out


def layers_map(policy: dict):
    """{ id: {name, order, description?} }。"""
    out = {}
    for l in require(policy, "layers"):
        out[l["id"]] = {"name": require(l, "name"), "order": require(l, "order"),
                        "description": l.get("description", "")}
    return out


def glob_match(path, pattern):
    """唯一 glob 语义实现（C8 / 审查 P2）：`**` 跨分隔符、`*` 不跨、`?` 单字符非分隔符。

    消费方：check_console_granularity / check_renderer_granularity / check_host_boundary 的
    「探针归属 ⊆ policy」判据。此前三者各持一份逐字副本，本函数将其收敛为一份。
    """
    path = str(path).replace("\\", "/")
    pattern = str(pattern).replace("\\", "/")
    rx, i = "", 0
    while i < len(pattern):
        c = pattern[i]
        if c == "*":
            if pattern[i:i + 2] == "**":
                rx += ".*"
                i += 2
                if i < len(pattern) and pattern[i] == "/":
                    i += 1
                continue
            rx += "[^/]*"
        elif c == "?":
            rx += "[^/]"
        else:
            rx += re.escape(c)
        i += 1
    return re.match("^" + rx + "$", path) is not None


def edge_fingerprint(edges):
    """边集的结构投影指纹（§3.3）：只投影 id/from/to/type/strength/dv，排序后 sha256。

    载体确定性：sort_keys + 固定分隔符 + 固定排序键（from,to,id）→ 与输入顺序无关、跨平台稳定。
    """
    canon = [{"id": e.get("id"), "from": e.get("from"), "to": e.get("to"),
              "type": e.get("type"), "strength": e.get("strength"),
              "dv": bool(e.get("direction_violation"))} for e in edges]
    canon.sort(key=lambda x: (x["from"], x["to"], x["id"]))
    text = json.dumps(canon, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def expect_edges(policy: dict) -> int:
    return int(require(policy, "edges.expect_count"))


def expect_edges_sha(policy: dict) -> str:
    return str(require(policy, "edges.sha256"))


def retired_edge_ids(policy: dict) -> set:
    return set(require(policy, "edges.retired_ids"))


def dv_max(policy: dict) -> int:
    return int(require(policy, "gates.dv_max"))


def scc_max(policy: dict) -> int:
    return int(require(policy, "gates.scc_max"))


def expect_shell_deps(policy: dict) -> list:
    return list(require(policy, "gates.expect_shell_deps"))


def retired_concerns(policy: dict) -> set:
    return set(require(policy, "gates.retired_concerns"))


def granularity(policy: dict, guard: str) -> dict:
    """某守卫的粒度配置（caps / 白名单 / 退役 id / 探针归属）。缺失即 fail-closed。"""
    return dict(require(policy, "granularity.%s" % guard))


# ---------------------------------------------------------------- 测试面谓词（c-arch-15 / R4）
def _string_list(policy: dict, dotted_key: str) -> list:
    """取「非空字符串列表」型策略字段；非列表/含非字符串即 fail-closed（与字段缺失同责）。"""
    vals = require(policy, dotted_key)
    if not isinstance(vals, list) or not all(isinstance(v, str) for v in vals):
        raise PolicyMissing(dotted_key)
    return list(vals)


def test_face_globs(policy: dict) -> list:
    """测试面路径粗筛表（`gates.test_face_globs`）：路径形态层面的测试面识别。"""
    return _string_list(policy, "gates.test_face_globs")


def test_face_files(policy: dict) -> list:
    """测试面冻结表（`gates.test_face_files`）：权威、禁 glob、逐条显式登记。"""
    return _string_list(policy, "gates.test_face_files")


def is_test_face(policy: dict, path) -> bool:
    """测试面谓词：`path ∈ 冻结表 ∪ 任一粗筛 glob`。

    语义（与归属正交）：真 ⇒ 该文件**不参与出边证据提取**；但**归属/coverage 不受影响**
    （测试面文件仍被 product_files() 枚举、仍被模块 glob 归属）。缺失任一字段即 fail-closed。
    """
    p = str(path).replace("\\", "/")
    if p in set(test_face_files(policy)):
        return True
    return any(glob_match(p, g) for g in test_face_globs(policy))


# ---------------------------------------------------------------- 叙事（prose）投影与摘要（c-arch-17 / R3）
def churn_basis(policy: dict) -> dict:
    """churn 口径登记（`gates.churn_basis`）：files / metric / window_days / bands / source。

    fail-closed：缺失即抛 PolicyMissing——避免「口径只活在 note 措辞里」，下轮再判歧义（Δ9/R5）。
    """
    return dict(require(policy, "gates.churn_basis"))


def notes_projection(policy: dict) -> dict:
    """prose 摘要投影口径（`gates.notes_projection`）：fields / normalize / algo。"""
    return dict(require(policy, "gates.notes_projection"))


def notes_sha256(policy: dict) -> str:
    """顶层 arch note 的规范化摘要（`gates.notes_sha256`）。"""
    return str(require(policy, "gates.notes_sha256"))


def notes_sha256_by_module(policy: dict) -> dict:
    """逐模块 prose 的规范化摘要（`gates.notes_sha256_by_module`）：键集 == policy.modules。

    口径（c-arch-18 / R3）：逐格摘要 = sha256(canonical_json({module, review_note, notes}))——
    `notes`（模块注记）与 `health.review_note` 一并纳入，故任一字段改一位即红、删字段亦红
    （缺失以 None 参与摘要，不静默跳过）。
    """
    return dict(require(policy, "gates.notes_sha256_by_module"))


def prose_quantity_rules(policy: dict) -> dict:
    """计数型 prose 规则（`gates.prose_quantity_rules`）：mode / families / whitelist / exempt_marker。

    fail-closed：缺失即抛 PolicyMissing——否则「叙事不得写滑动窗口量」这条判据会静默消失。
    """
    return dict(require(policy, "gates.prose_quantity_rules"))


def prose_provenance_keys(policy: dict) -> list:
    """叙事基准（`gates.prose_provenance_keys`）：`meta.provenance` 必须具备的键（fail-closed）。"""
    return _string_list(policy, "gates.prose_provenance_keys")


# ---------------------------------------------------------------- prose 模板注入（c-arch-18 / R6②）
# 唯一真值：authoring 面（parts/**）里的 prose 携带下列 token，**生成期**由 `meta.provenance` 填充；
# 交付地图（live / fixture 投影）不得残留任何 token。CLI（run/easyvibe_map_cli.py）与本模块共用本实现，
# 判据 L 亦据此 fail-closed（残留 token 即红）——杜绝「模板态被原样交付」。
PROSE_TEMPLATE_TOKENS = ("{{provenance.head_short}}", "{{provenance.generated_at}}")


def prose_template_tokens() -> tuple:
    return PROSE_TEMPLATE_TOKENS


def inject_prose(text, prov):
    """把 prose 模板 token 按 `prov`（provenance 字典）填充（幂等；非字符串/无 prov 原样返回）。"""
    if not isinstance(text, str) or not isinstance(prov, dict):
        return text
    for tok in PROSE_TEMPLATE_TOKENS:
        key = tok[len("{{provenance."):-len("}}")]
        text = text.replace(tok, str(prov.get(key, "")))
    return text


def unresolved_prose_tokens(text) -> list:
    """返回文本中**残留**的模板 token（空列表 = 已解析）。"""
    return [t for t in PROSE_TEMPLATE_TOKENS if t in (text or "")]


def active_concerns(policy: dict) -> list:
    """架构级**在册** concern id（`gates.active_concerns`）：交付地图 `health.concerns` 的唯一真值。

    fail-closed：缺失/非字符串列表即抛 PolicyMissing。与 `retired_concerns`（已闭环、禁复现）互斥，
    由 validate_policy F0 断言。「架构级关注点」因此从 prose 断言升级为受控事实（P0-2 复修）。
    """
    return _string_list(policy, "gates.active_concerns")


def coupling_ratchet(policy: dict) -> dict:
    """耦合棘轮（`gates.coupling_ratchet`）：coupling_high 载体计数上限 + 枢纽出入度上限（只降不升）。"""
    return dict(require(policy, "gates.coupling_ratchet"))


def accepted_coupling(policy: dict) -> dict:
    """显式「已接受」裁定（`gates.accepted_coupling`）：载体 id / 理由 / 上限 / 复核时点。"""
    return dict(require(policy, "gates.accepted_coupling"))


def edge_scan(policy: dict) -> dict:
    """边扫描口径登记（`gates.edge_scan`）：命令 / 原始行数 / 宿主伪边 / 模块对数 / 非 import 边集。"""
    return dict(require(policy, "gates.edge_scan"))


def edge_scan_consistency(policy: dict) -> list:
    """`gates.edge_scan` 口径自洽（**唯一实现**，P3-7 返修：消除 g5 与 F0 的两份算术）。

    返回问题消息列表（**不带前缀**，调用方按语境加 `F0`/`G5`）：
      · pairs == raw_lines − len(host_pseudo_edges)
      · pairs + len(non_import_edges) == edges.expect_count
    形状非法（非整数/空列表/缺 expect_count）亦在此 fail-closed 报告。
    """
    problems = []
    try:
        esc = require(policy, "gates.edge_scan")
    except PolicyMissing as e:
        return ["%s" % e]
    pairs_n = esc.get("expected_module_pairs")
    raw_n = esc.get("expected_raw_lines")
    pseudo = esc.get("host_pseudo_edges")
    nonimp = esc.get("expected_non_import_edges")
    if not isinstance(raw_n, int) or not isinstance(pairs_n, int):
        problems.append("gates.edge_scan.expected_raw_lines/expected_module_pairs 须为整数")
    if not isinstance(pseudo, list) or not pseudo:
        problems.append("gates.edge_scan.host_pseudo_edges 须为非空列表")
    if not isinstance(nonimp, list):
        problems.append("gates.edge_scan.expected_non_import_edges 须为列表")
    try:
        expect_count = int(require(policy, "edges.expect_count"))
    except (PolicyMissing, TypeError, ValueError):
        problems.append("gates.edge_scan 口径不自洽：edges.expect_count 缺失/非整数")
        expect_count = None
    if isinstance(raw_n, int) and isinstance(pairs_n, int) and isinstance(pseudo, list):
        if pairs_n != raw_n - len(pseudo):
            problems.append("gates.edge_scan 口径不自洽：module_pairs != raw_lines - len(host_pseudo_edges)")
        if isinstance(nonimp, list) and expect_count is not None and pairs_n + len(nonimp) != expect_count:
            problems.append("gates.edge_scan 口径不自洽：module_pairs + non_import != edges.expect_count")
    return problems


def normalize_review_note(text) -> str:
    """prose 摘要前规范化：EOL→\\n ▸ 逐行 rstrip ▸ 连续空行折叠为单空行 ▸ 首尾 strip。

    目的：只对**实质文本变化**敏感，CRLF / 尾随空格 / 空行差异不得造成「假漂移」。
    """
    s = "" if text is None else str(text)
    s = s.replace("\r\n", "\n").replace("\r", "\n")
    out, blank = [], False
    for line in s.split("\n"):
        line = line.rstrip()
        if line == "":
            if out and not blank:
                out.append("")
            blank = True
        else:
            out.append(line)
            blank = False
    return "\n".join(out).strip()


def _canonical_sha(obj) -> str:
    """规范化 JSON（sort_keys + 固定分隔符）的 sha256 —— 与 edge_fingerprint 同构，跨平台稳定。"""
    text = json.dumps(obj, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def prose_arch_digest(live_map: dict) -> str:
    """顶层 arch note 的摘要（口径见 notes_projection）。"""
    note = ((live_map or {}).get("health") or {}).get("review_note")
    return _canonical_sha({"arch": normalize_review_note(note)})


def prose_module_digest(module_id: str, live_map: dict) -> str:
    """某模块 prose 的摘要（review_note + notes 合并，含模块 id，防跨格串扰）。

    R3 扩面：此前只投影 `health.review_note`，21 格 `notes`（含已失真的产品文件读数）在闸门之外。
    现按 `gates.notes_projection.fields` 的口径把 `notes` 一并纳入；缺失字段以 None 参与摘要，
    故「删字段」同样必红（与「改一位」同责）。
    """
    for m in (live_map or {}).get("modules", []):
        if m.get("id") == module_id:
            return _canonical_sha({"module": module_id,
                                   "review_note": normalize_review_note((m.get("health") or {}).get("review_note")),
                                   "notes": normalize_review_note(m.get("notes"))})
    return _canonical_sha({"module": module_id, "review_note": None, "notes": None})


_HEX64 = re.compile(r"^[0-9a-f]{64}$")


def validate_policy(policy: dict) -> list:
    """policy 自洽性（F0）：返回问题列表，空 = 自洽。字段缺失亦记为问题。"""
    problems = []
    try:
        layers = require(policy, "layers")
        mods = require(policy, "modules")
    except PolicyMissing as e:
        return ["F0 %s" % e]
    lids = [l.get("id") for l in layers]
    if len(set(lids)) != len(lids):
        problems.append("F0 layers id 重复")
    orders = [l.get("order") for l in layers]
    if len(set(orders)) != len(orders):
        problems.append("F0 layers order 重复")
    mids = [m.get("id") for m in mods]
    if len(set(mids)) != len(mids):
        problems.append("F0 modules id 重复")
    eo = [m.get("emit_order") for m in mods]
    if len(set(eo)) != len(eo):
        problems.append("F0 modules emit_order 重复")
    lset = set(lids)
    for m in mods:
        if m.get("layer") not in lset:
            problems.append("F0 module %s layer 不在 layers" % m.get("id"))
        if not m.get("files"):
            problems.append("F0 module %s files 为空" % m.get("id"))
    for key in ("edges.expect_count", "edges.sha256", "edges.retired_ids",
                "gates.dv_max", "gates.scc_max", "gates.expect_shell_deps", "gates.retired_concerns",
                # c-arch-13：出边数上界（I9）、单文件行数上界与棘轮（I10）、直连集中度（I11a/b/c）
                "gates.server_api_out_edges_max", "gates.server_api_out_edges",
                # c-arch-16：组合根（assembly）出边登记（I13b；缺失即 fail-closed）
                "gates.assembly_out_edges",
                "gates.single_file_loc_max", "gates.single_file_loc_caps",
                "gates.db_direct_total", "gates.db_direct_focus_max",
                "gates.db_direct_focus_files_max",
                # c-arch-15：测试面谓词（出边证据剔除面；归属/coverage 不受影响）
                "gates.test_face_files", "gates.test_face_globs",
                # c-arch-14：console 粒度由单格标量结构泛化为多格结构（grids/caps/逐格前缀）
                "granularity.console.grids", "granularity.console.caps",
                "granularity.console.import_prefixes_by_grid",
                "granularity.renderer.grids", "granularity.renderer.caps",
                "granularity.host.probe_owner",
                # c-arch-17：churn 口径（R5）与 prose 摘要闸门（R3）——叙事面同样有受控落点与判据
                "gates.churn_basis", "gates.notes_projection",
                "gates.notes_sha256", "gates.notes_sha256_by_module",
                # c-arch-18：计数型 prose 规则（R2）/ 叙事基准（R6）/ 耦合棘轮与已接受裁定（R5）/
                # 边扫描口径（R8）——叙事与耦合趋势同样要有机器判据
                "gates.prose_quantity_rules", "gates.prose_provenance_keys",
                "gates.coupling_ratchet", "gates.accepted_coupling", "gates.edge_scan",
                # c-arch-18 返修（P0-2）：架构级在册 concern 集（交付地图 health.concerns 的唯一真值）
                "gates.active_concerns"):
        try:
            require(policy, key)
        except PolicyMissing as e:
            problems.append("F0 %s" % e)
    # 粒度白名单/上限/探针归属所点名的格必须真实存在（防悬空格 / 拼写漂移）
    mset = set(mids)
    try:
        # c-arch-14：console 多格网格三集合必须严格相等且 ⊆ 模块 id 集
        # （防「加了格忘了 cap / 忘了前缀」的静默形态：任一不等即 fail-closed）
        cg = require(policy, "granularity.console")
        c_grids = set(cg.get("grids", []))
        c_caps = set(cg.get("caps", {}))
        c_pref = set(cg.get("import_prefixes_by_grid", {}))
        if c_grids != c_caps:
            problems.append("F0 granularity.console grids != caps: grids-only=%s caps-only=%s"
                            % (sorted(c_grids - c_caps), sorted(c_caps - c_grids)))
        if c_grids != c_pref:
            problems.append("F0 granularity.console grids != import_prefixes_by_grid: "
                            "grids-only=%s prefixes-only=%s"
                            % (sorted(c_grids - c_pref), sorted(c_pref - c_grids)))
        for mid in c_grids | c_caps | c_pref:
            if mid not in mset:
                problems.append("F0 granularity.console 指向未知格: %s" % mid)
        for mid in require(policy, "granularity.console.caps"):
            if mid not in mset:
                problems.append("F0 granularity.console.caps 指向未知格: %s" % mid)
        for mid in require(policy, "granularity.renderer.grids"):
            if mid not in mset:
                problems.append("F0 granularity.renderer.grids 含未知格: %s" % mid)
        for mid in require(policy, "granularity.renderer.caps"):
            if mid not in mset:
                problems.append("F0 granularity.renderer.caps 指向未知格: %s" % mid)
        for mid in require(policy, "granularity.host.probe_owner").values():
            if mid not in mset:
                problems.append("F0 granularity.host.probe_owner 指向未知格: %s" % mid)
    except PolicyMissing:
        pass  # 缺字段已由上面的 key 循环登记，此处不重复报
    # c-arch-15 F0：测试面两表须为非空、无空串、无重复（冻结表逐条显式登记，禁 glob）
    try:
        tff = require(policy, "gates.test_face_files")
        tfg = require(policy, "gates.test_face_globs")
    except PolicyMissing:
        tff = tfg = None
    if isinstance(tff, list) and isinstance(tfg, list):
        for name, vals in (("gates.test_face_files", tff), ("gates.test_face_globs", tfg)):
            if not vals:
                problems.append("F0 %s 为空列表（测试面判据不得悬空）" % name)
            elif any((not isinstance(v, str)) or not v.strip() for v in vals):
                problems.append("F0 %s 含空条目/非字符串" % name)
        strings = [v for v in tff if isinstance(v, str)]
        if len(set(strings)) != len(strings):
            problems.append("F0 gates.test_face_files 存在重复条目")
    # c-arch-17 F0：churn 口径（R5）与 prose 摘要闸门（R3）的形状断言（CI 可跑；缺失即已由上面登记）
    try:
        cb = require(policy, "gates.churn_basis")
    except PolicyMissing:
        cb = None
    if isinstance(cb, dict):
        for k in ("files", "metric", "window_days", "bands", "source"):
            if k not in cb:
                problems.append("F0 gates.churn_basis 缺 %s" % k)
        bands = cb.get("bands")
        if not isinstance(bands, dict) or not isinstance(bands.get("high"), int) \
                or not isinstance(bands.get("medium"), int):
            problems.append("F0 gates.churn_basis.bands 须为 {high:int, medium:int}")
        elif bands["high"] <= bands["medium"]:
            problems.append("F0 gates.churn_basis.bands 非单调（high <= medium）")
        if not isinstance(cb.get("window_days"), int) or cb.get("window_days", 0) <= 0:
            problems.append("F0 gates.churn_basis.window_days 须为正整数")
        for k in ("files", "metric", "source"):
            if not isinstance(cb.get(k), str) or not cb.get(k, "").strip():
                problems.append("F0 gates.churn_basis.%s 须为非空字符串" % k)
    try:
        nproj = require(policy, "gates.notes_projection")
    except PolicyMissing:
        nproj = None
    if isinstance(nproj, dict):
        flds = nproj.get("fields")
        if not isinstance(flds, list) or not flds or not all(isinstance(x, str) and x.strip() for x in flds):
            problems.append("F0 gates.notes_projection.fields 须为非空字符串列表")
        for k in ("normalize", "algo"):
            if not isinstance(nproj.get(k), str) or not nproj.get(k, "").strip():
                problems.append("F0 gates.notes_projection.%s 须为非空字符串" % k)
    try:
        nsha = require(policy, "gates.notes_sha256")
        nbym = require(policy, "gates.notes_sha256_by_module")
    except PolicyMissing:
        nsha = nbym = None
    if nsha is not None and (not isinstance(nsha, str) or not _HEX64.match(nsha)):
        problems.append("F0 gates.notes_sha256 须为 64 位小写 hex（fail-closed）")
    if isinstance(nbym, dict):
        if set(nbym.keys()) != mset:
            problems.append("F0 gates.notes_sha256_by_module 键集 != modules: policy-only=%s module-only=%s"
                            % (sorted(set(nbym) - mset), sorted(mset - set(nbym))))
        bad = sorted(k for k, v in nbym.items() if not isinstance(v, str) or not _HEX64.match(v))
        if bad:
            problems.append("F0 gates.notes_sha256_by_module 非 64hex 的格: %s" % bad)
    elif nbym is not None:
        problems.append("F0 gates.notes_sha256_by_module 须为 dict")
    # c-arch-18 F0：计数型 prose 规则（R2）形状（缺失已由 key 循环登记）
    try:
        pqr = require(policy, "gates.prose_quantity_rules")
    except PolicyMissing:
        pqr = None
    if isinstance(pqr, dict):
        if pqr.get("mode") != "digits-forbidden":
            problems.append("F0 gates.prose_quantity_rules.mode 须为 digits-forbidden")
        fams = pqr.get("families")
        if not isinstance(fams, dict) or not fams:
            problems.append("F0 gates.prose_quantity_rules.families 须为非空 dict（禁悬空）")
        else:
            for fid, pat in fams.items():
                if not isinstance(pat, str) or not pat.strip():
                    problems.append("F0 gates.prose_quantity_rules.families.%s 须为非空正则" % fid)
                    continue
                try:
                    re.compile(pat)
                except re.error:
                    problems.append("F0 gates.prose_quantity_rules.families.%s 正则不可编译" % fid)
        if not isinstance(pqr.get("exempt_marker"), str) or not pqr.get("exempt_marker", "").strip():
            problems.append("F0 gates.prose_quantity_rules.exempt_marker 须为非空字符串")
    # c-arch-18 F0：叙事基准键（R6）与边扫描口径（R8）形状
    try:
        ppk = require(policy, "gates.prose_provenance_keys")
    except PolicyMissing:
        ppk = None
    if isinstance(ppk, list) and not ppk:
        problems.append("F0 gates.prose_provenance_keys 须为非空列表")
    # c-arch-18 返修（P0-2）：架构级在册 concern 集（`gates.active_concerns`）——形状断言 +
    # 与 retired_concerns 互斥（同一 id 不得既在册又已闭环）。空列表合法（全部闭环时显式留空）。
    try:
        ac = require(policy, "gates.active_concerns")
    except PolicyMissing:
        ac = None
    if ac is not None and (not isinstance(ac, list)
                           or any((not isinstance(v, str)) or not v.strip() for v in ac)):
        problems.append("F0 gates.active_concerns 须为字符串列表（可为空）")
    elif isinstance(ac, list):
        if len(set(ac)) != len(ac):
            problems.append("F0 gates.active_concerns 存在重复条目")
        try:
            both = sorted(set(ac) & set(require(policy, "gates.retired_concerns")))
        except PolicyMissing:
            both = []
        if both:
            problems.append("F0 gates.active_concerns 与 retired_concerns 重叠: %s" % both)
    # P2-5/P3-7 返修：edge_scan 口径自洽走**唯一实现**（缺失字段不再抛异常，返回结构化问题）
    for p in edge_scan_consistency(policy):
        problems.append("F0 %s" % p)
    # c-arch-18 F0：耦合棘轮与已接受裁定（R5）——上限为正整数、载体 id 属图、裁定与棘轮逐分量互证
    try:
        rat = require(policy, "gates.coupling_ratchet")
        acc = require(policy, "gates.accepted_coupling")
    except PolicyMissing:
        rat = acc = None
    if isinstance(rat, dict) and isinstance(acc, dict):
        cmax = rat.get("coupling_high_cells_max")
        dmax = rat.get("degrees_max")
        if not isinstance(cmax, int) or cmax < 0:
            problems.append("F0 gates.coupling_ratchet.coupling_high_cells_max 须为非负整数")
        if not isinstance(dmax, dict) or not dmax:
            problems.append("F0 gates.coupling_ratchet.degrees_max 须为非空 dict")
        else:
            for mid, caps in dmax.items():
                if mid not in mset:
                    problems.append("F0 gates.coupling_ratchet.degrees_max 指向未知格: %s" % mid)
                if not isinstance(caps, dict) or not isinstance(caps.get("out"), int) \
                        or not isinstance(caps.get("in"), int):
                    problems.append("F0 gates.coupling_ratchet.degrees_max.%s 须为 {out:int, in:int}" % mid)
        cells = acc.get("cells")
        if not isinstance(cells, list) or not cells:
            problems.append("F0 gates.accepted_coupling.cells 须为非空列表（禁空头接受）")
        else:
            for c in cells:
                if not isinstance(c, dict):
                    problems.append("F0 gates.accepted_coupling.cells 含非对象条目")
                    continue
                cid = c.get("id")
                if cid not in mset:
                    problems.append("F0 gates.accepted_coupling 指向未知格: %s" % cid)
                for k in ("role", "reason", "review_by"):
                    if not isinstance(c.get(k), str) or not c.get(k, "").strip():
                        problems.append("F0 gates.accepted_coupling.%s 缺 %s（禁空头接受）" % (cid, k))
                caps = c.get("caps")
                if not isinstance(caps, dict) or caps != (dmax or {}).get(cid):
                    problems.append("F0 gates.accepted_coupling.%s.caps 与 coupling_ratchet.degrees_max 不一致" % cid)
        if isinstance(cmax, int) and len(cells or []) != cmax:
            problems.append("F0 coupling_high_cells_max %s != accepted_coupling.cells %d" % (cmax, len(cells or [])))
    return problems
