#!/usr/bin/env python3
"""构建期关系判据（R3）与图判据（R4）的唯一实现。

- 策略装载：scripts/map_edge_policy.json（受版本控制，可审计 / 可 diff / CI 可见）。
- 判定函数：must_drop 命中、Tarjan SCC、跨层 SCC。
- 消费方：run/easyvibe_map_cli.py（finalize 关口 + normalize-edges）、
  scripts/verify_map_acyclic.py（命令行验收）、地图归纳参考实现的环门禁。
"""
import json
import os
import sys

POLICY_REL = os.path.join("scripts", "map_edge_policy.json")


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
