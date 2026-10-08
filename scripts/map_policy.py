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
                "granularity.host.probe_owner"):
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
    return problems
