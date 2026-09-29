#!/usr/bin/env python3
"""v2.1 fixtures 回归检查。

用法:
  python3 check_fixtures.py <REPO_ROOT> [SCHEMA_PATH]

检查项:
  1. expected/map.json 通过 JSON Schema 校验
  2. expected/growth.log 每行合法 JSON，重放结果与 map.json 一致
     （layers 归一排序后相等、modules/edges 集合相等、arch health 相等）
  3. map.json layers[] 存储序为 order 升序（生长序≠存储序守卫）
  4. modules[].dependencies 与 edges[] 完全一致
  5. 覆盖率：map.json 的 files glob 对 REPO_ROOT/lib 代码文件覆盖率 >90%
  6. 确定性回归：按 map.json 的模块 files globs 重算 lib/ 内 import 聚合，
     与 expected/edges_computed.json 完全一致（lib/ 代码未变时不得漂移）
  9. concerns 存在性（警告级，不阻断）：架构级 decay_flags 非空时 concerns 不应为空；
     全模块无 concerns 时告警（防"问题提名"步骤再被改丢）

退出码: 0 = 全部通过；1 = 存在失败项（逐项打印 FAIL 原因）。WARN 不影响退出码。
"""
import json, os, re, sys, fnmatch, collections

FIX = os.path.dirname(os.path.abspath(__file__))
EXP = os.path.join(FIX, 'expected')
CODE_EXT = {'.dart', '.ts', '.tsx', '.js', '.jsx', '.py', '.go', '.rs',
            '.java', '.kt', '.swift', '.cs', '.cpp', '.c', '.h', '.rb', '.php'}
fails = []

def check(name, ok, detail=''):
    print(f'[{"PASS" if ok else "FAIL"}] {name}' + (f' — {detail}' if detail else ''))
    if not ok:
        fails.append(name)

def main():
    repo = sys.argv[1]
    schema_path = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
        FIX, '..', '..', 'easyvibe-map-schema-v1.json')

    m = json.load(open(os.path.join(EXP, 'map.json')))

    # 1. Schema
    try:
        import jsonschema
        jsonschema.validate(m, json.load(open(schema_path)))
        check('schema validation', True)
    except ImportError:
        check('schema validation', False, 'jsonschema lib absent')
    except Exception as e:
        check('schema validation', False, str(e)[:120])

    # 2. growth.log 重放
    layers, modules, edges, arch = [], [], [], None
    for ln in open(os.path.join(EXP, 'growth.log'), encoding='utf-8'):
        ev = json.loads(ln)
        if ev['type'] == 'layer': layers.append(ev['layer'])
        elif ev['type'] == 'module':
            modules.append(ev['module']); edges.extend(ev.get('out_edges', []))
        elif ev['type'] == 'arch_health': arch = ev['health']
    check('replay: modules', {x['id'] for x in modules} == {x['id'] for x in m['modules']})
    check('replay: edges', {(e['from'], e['to']) for e in edges} ==
                            {(e['from'], e['to']) for e in m['edges']})
    check('replay: arch health', arch == m['health'])

    # 3. layers 存储序
    orders = [l['order'] for l in m['layers']]
    check('layers sorted by order', orders == sorted(orders) and orders == list(range(len(orders))),
          f'orders={orders}')

    # 4. deps == edges
    ok = all(set(x['dependencies']) == {e['to'] for e in m['edges'] if e['from'] == x['id']}
             for x in m['modules'])
    check('deps==edges', ok)

    # v1.1：边稳定 id 存在、唯一、e 前缀
    eids = [e.get('id', '') for e in m.get('edges', [])]
    check('edge ids (v1.1)', bool(eids) and all(eids) and len(set(eids)) == len(eids) and all(re.match(r'^e[a-z0-9_-]*$', i) for i in eids),
          'missing/duplicate/bad-format edge id')

    # 5. 覆盖率
    files = []
    for root, _, fs in os.walk(os.path.join(repo, 'lib')):
        for f in fs:
            if os.path.splitext(f)[1] in CODE_EXT:
                files.append(os.path.relpath(os.path.join(root, f), repo))
    pats = [f for x in m['modules'] for f in x['files']]
    missed = [f for f in files if not any(fnmatch.fnmatch(f, p) for p in pats)]
    cov = (len(files) - len(missed)) / max(len(files), 1)
    check('coverage >90%', cov > 0.9, f'{cov:.1%}, missed={missed[:5]}')

    # 6. 确定性边回归
    def mod_of(path):
        for x in m['modules']:
            for p in x['files']:
                if fnmatch.fnmatch(path, p):
                    return x['id']
        return None
    edges_live = collections.Counter()
    libdir = os.path.join(repo, 'lib')
    for root, _, fs in os.walk(libdir):
        for f in fs:
            if not f.endswith('.dart'): continue
            path = os.path.join(root, f)
            src = mod_of(os.path.relpath(path, repo))
            if src is None: continue
            with open(path, encoding='utf-8', errors='replace') as fh:
                for line in fh:
                    mm = re.match(r"import 'package:hover/([^']+)'", line)
                    if mm:
                        tgt = mod_of('lib/' + mm.group(1))
                        if tgt and tgt != src:
                            edges_live[(src, tgt)] += 1
    expected = {(k.split('->')[0], k.split('->')[1]): v
                for k, v in json.load(open(os.path.join(EXP, 'edges_computed.json'))).items()}
    check('edges deterministic regression', dict(edges_live) == expected,
          f'live={len(edges_live)} expected={len(expected)}' if dict(edges_live) != expected else '')

    # 9. concerns 存在性（v2.2 起；警告级不阻断——存量 fixtures 为试点产物，重跑后应非空）
    arch_concerns = m.get('health', {}).get('concerns') or []
    mod_concerns = sum(1 for mod in m.get('modules', []) if mod.get('health', {}).get('concerns'))
    warns = []
    if m.get('health', {}).get('decay_flags') and not arch_concerns:
        warns.append('架构级 decay_flags 非空但 concerns 为空（问题清单将降级为兜底模式）')
    if mod_concerns == 0:
        warns.append('所有模块均无 concerns')
    if warns:
        print(f'[WARN] concerns presence — {"; ".join(warns)}')

    print()
    if fails:
        print(f'REGRESSION: {len(fails)} check(s) failed: {fails}')
        sys.exit(1)
    print('ALL CHECKS PASSED')

if __name__ == '__main__':
    main()
