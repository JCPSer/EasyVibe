#!/usr/bin/env python3
"""easyvibe-map CLI — prompt v2.2 reference impl (EasyVibe monorepo).

子命令：init / emit-module / emit-edges / append-log / finalize / self-check / coverage
- 前 5 个是自托管地图管线在跑的协议实现（tmp+rename、无锁、覆盖率闸门），本次原样收敛；
- self-check / coverage 由旧 run/ 参考实现并入：只读 map.json 做结构自检 / 覆盖率报告，不写产物。

单一事实源 = 本文件（入 git、可 review）；`.easyvibe/map/` 下同名副本由
`scripts/gen_map_cli.py` 逐字节生成，sha256 一致性由 `scripts/verify_assets.py` 守卫。
本文件在 run/，不在守卫扫描面内。
"""
import json, os, sys, re, datetime, shutil

# REPO 不再硬编码：默认 cwd，运行器通过 REPO_ROOT 环境变量显式指定（见 build_easyvibe_map.py）
REPO = os.environ.get('REPO_ROOT', os.getcwd())
MAP_DIR = os.path.join(REPO, '.easyvibe', 'map')
PARTS = os.path.join(MAP_DIR, 'parts')
PROGRESS = os.path.join(MAP_DIR, 'progress.json')
GROWTH = os.path.join(MAP_DIR, 'growth.log')
MAPJSON = os.path.join(MAP_DIR, 'map.json')
META = os.path.join(MAP_DIR, 'meta.json')
ORDER = os.path.join(MAP_DIR, 'emit_order.json')
SCHEMA = os.environ.get('SCHEMA_PATH', os.path.join(REPO, 'easyvibe-map-schema-v1.1.json'))

# R3/R4：构建期关系判据与图判据的唯一实现（scripts/map_policy.py，受版本控制）
sys.path.insert(0, os.path.join(REPO, 'scripts'))
import map_policy  # noqa: E402
# c-arch-12：同代判据（policy ↔ 产物）的唯一实现（GATE-1 复用，零重复）
sys.path.insert(0, os.path.join(REPO, 'scripts'))
import verify_arch_facts  # noqa: E402

# R4 关口 C 的 DV 棘轮基线（只降不升）与关口 B 的 SCC 上限、退役边 id 一律来自
# scripts/map_edge_policy.json（c-arch-12：归纳期与守卫期同源读取，不再在本文件硬编码）。

MODULE_KEYS = {'id', 'last_analyzed_at', 'name', 'layer', 'responsibility', 'files',
               'key_entries', 'dependencies', 'health', 'notes'}

# ---- 结构自检用的字段白名单（并入自旧 run/ 参考实现：self-check 子命令） ----
ALLOWED_HEALTH = {'score', 'coupling', 'complexity', 'churn', 'decay_flags',
                  'review_note', 'concerns'}
ALLOWED_CONCERN = {'id', 'severity', 'finding', 'suggestion'}
EDGE_ID_PAT = re.compile(r'^e[a-z0-9_-]*$')

# ---------------------------------------------------------------- fs helpers
def _tmp_write(path, text):
    d = os.path.dirname(path)
    if d and not os.path.isdir(d):
        os.makedirs(d, exist_ok=True)
    tmp = path + '.tmp.%d' % os.getpid()
    with open(tmp, 'w', encoding='utf-8') as f:
        f.write(text)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)

def write_json_atomic(path, obj):
    _tmp_write(path, json.dumps(obj, ensure_ascii=False, indent=2) + '\n')

def now_iso():
    return datetime.datetime.now().astimezone().replace(microsecond=0).isoformat()

def load_schema():
    with open(SCHEMA, encoding='utf-8') as f:
        return json.load(f)

# ---------------------------------------------------------------- validator
def validate(inst, sch, path, errors, root):
    if not isinstance(sch, dict):
        return
    if '$ref' in sch:
        ref = sch['$ref'].split('/')[-1]
        return validate(inst, root['$defs'][ref], path, errors, root)
    t = sch.get('type')
    if t == 'object':
        if not isinstance(inst, dict):
            errors.append('%s: expected object' % path); return
        props = sch.get('properties', {})
        for r in sch.get('required', []):
            if r not in inst:
                errors.append('%s: missing required "%s"' % (path, r))
        if sch.get('additionalProperties') is False:
            for k in inst:
                if k not in props:
                    errors.append('%s: additional property "%s"' % (path, k))
        for k, v in inst.items():
            if k in props:
                validate(v, props[k], '%s.%s' % (path, k), errors, root)
    elif t == 'array':
        if not isinstance(inst, list):
            errors.append('%s: expected array' % path); return
        if 'minItems' in sch and len(inst) < sch['minItems']:
            errors.append('%s: minItems %d' % (path, sch['minItems']))
        if 'maxItems' in sch and len(inst) > sch['maxItems']:
            errors.append('%s: maxItems %d' % (path, sch['maxItems']))
        for i, v in enumerate(inst):
            validate(v, sch.get('items', {}), '%s[%d]' % (path, i), errors, root)
    elif t == 'string':
        if not isinstance(inst, str):
            errors.append('%s: expected string' % path); return
        if 'minLength' in sch and len(inst) < sch['minLength']:
            errors.append('%s: minLength %d' % (path, sch['minLength']))
        if 'pattern' in sch and not re.match(sch['pattern'], inst):
            errors.append('%s: pattern %s violated (%r)' % (path, sch['pattern'], inst))
    elif t == 'integer':
        if not isinstance(inst, int) or isinstance(inst, bool):
            errors.append('%s: expected integer' % path); return
        if 'minimum' in sch and inst < sch['minimum']:
            errors.append('%s: minimum %s' % (path, sch['minimum']))
        if 'maximum' in sch and inst > sch['maximum']:
            errors.append('%s: maximum %s' % (path, sch['maximum']))
    elif t == 'number':
        if not isinstance(inst, (int, float)) or isinstance(inst, bool):
            errors.append('%s: expected number' % path); return
    if 'enum' in sch and inst not in sch['enum']:
        errors.append('%s: %r not in enum %s' % (path, inst, sch['enum']))
    if 'const' in sch and inst != sch['const']:
        errors.append('%s: const %r violated (%r)' % (path, sch['const'], inst))

def validate_health(h, where, errs):
    """模块/架构 health 块的字段与枚举校验（并入自旧 run/ 参考实现，供 self-check 用）。"""
    if not isinstance(h, dict):
        errs.append('%s: health not object' % where); return
    if set(h) - ALLOWED_HEALTH:
        errs.append('%s: health extra fields %s' % (where, set(h) - ALLOWED_HEALTH))
    if not (isinstance(h.get('score'), int) and 0 <= h['score'] <= 100):
        errs.append('%s: score range' % where)
    if not isinstance(h.get('score'), int):
        errs.append('%s: score must be int' % where)
    if h.get('coupling') not in ('low', 'medium', 'high', 'critical'):
        errs.append('%s: coupling enum' % where)
    if h.get('complexity') not in ('low', 'medium', 'high'):
        errs.append('%s: complexity enum' % where)
    if 'churn' in h and h['churn'] not in ('low', 'medium', 'high'):
        errs.append('%s: churn enum' % where)
    if 'decay_flags' in h and not isinstance(h['decay_flags'], list):
        errs.append('%s: decay_flags not list' % where)
    cons = h.get('concerns')
    if cons is not None:
        if not isinstance(cons, list):
            errs.append('%s: concerns not list' % where)
        else:
            if len(cons) > 3:
                errs.append('%s: concerns >3' % where)
            for c in cons:
                if set(c) - ALLOWED_CONCERN:
                    errs.append('%s: concern extra fields' % where)
                if set(c) < ALLOWED_CONCERN:
                    errs.append('%s: concern missing fields' % where)
                if c.get('severity') not in ('critical', 'high'):
                    errs.append('%s: concern severity enum' % where)

# ---------------------------------------------------------------- coverage
CODE_EXT = {'.rs', '.ts', '.tsx', '.js', '.mjs', '.py', '.sh', '.sql'}
CONFIG_NAMES = {'Cargo.toml', 'tauri.conf.json', 'build.rs', 'index.html'}
PRODUCT_ROOTS = ['easyvibe-backend', 'easyvibe-renderer', 'easyvibe-desktop', 'run', 'scripts']
# 归属（枚举/coverage）与出边（edging 证据）**正交**：测试面文件仍被 product_files() 枚举
# 并归属模块（故 EXC_PARTS 不排除 /tests/ 等），但按 scripts/map_policy.py::is_test_face
# 谓词不参与出边证据提取，故不构成依赖边（c-arch-15 / R4）。coverage_ratio 因此保持 1.000。
EXC_PARTS = ('/node_modules/', '/target/', '/dist/', '/binaries/', '/assets/', '/icons/',
             '/public/data/', '/src-tauri/gen/')

def product_files():
    out = []
    for root in PRODUCT_ROOTS:
        base = os.path.join(REPO, root)
        for dirpath, dirnames, filenames in os.walk(base):
            rel_dir = '/' + os.path.relpath(dirpath, REPO).replace(os.sep, '/') + '/'
            if any(p in rel_dir for p in EXC_PARTS):
                continue
            for fn in filenames:
                rel = os.path.relpath(os.path.join(dirpath, fn), REPO).replace(os.sep, '/')
                if '/' + rel in ('/' + rel) and any(p in '/' + rel for p in EXC_PARTS):
                    continue
                ext = os.path.splitext(fn)[1].lower()
                if ext in CODE_EXT or fn in CONFIG_NAMES:
                    out.append(rel)
    return sorted(out)

def glob_to_re(cls):
    return '(?s:' + cls[len('glob:'):] + ')'

def glob_match(path, pattern):
    """fnmatch-style glob with ** crossing separators."""
    rx = ''
    i = 0
    while i < len(pattern):
        c = pattern[i]
        if c == '*':
            if pattern[i:i+2] == '**':
                rx += '.*'
                i += 2
                if i < len(pattern) and pattern[i] == '/':
                    i += 1
                continue
            else:
                rx += '[^/]*'
        elif c == '?':
            rx += '[^/]'
        else:
            rx += re.escape(c)
        i += 1
    return re.match('^' + rx + '$', path) is not None

# ---------------------------------------------------------------- progress
def set_progress(phase, modules_total=0, modules_done=0, current=None, percent=0,
                 decision_reason=None, error=None):
    obj = {}
    if os.path.exists(PROGRESS):
        try:
            obj = json.load(open(PROGRESS, encoding='utf-8'))
        except Exception:
            obj = {}
    obj['phase'] = phase
    obj['modules_total'] = modules_total
    obj['modules_done'] = modules_done
    obj['current_module'] = current
    obj['percent'] = percent
    obj['updated_at'] = now_iso()
    if decision_reason is not None:
        obj['decision_reason'] = decision_reason
    elif 'decision_reason' not in obj:
        obj['decision_reason'] = 'none'
    if error:
        obj['error'] = error
    else:
        obj.pop('error', None)
    write_json_atomic(PROGRESS, obj)

# ---------------------------------------------------------------- init
def decision_on_existing():
    if not os.path.exists(MAPJSON):
        return 'none'
    try:
        old = json.load(open(MAPJSON, encoding='utf-8'))
    except Exception:
        shutil.copy2(MAPJSON, MAPJSON + '.bak.%s' % datetime.datetime.now().strftime('%Y%m%d-%H%M%S'))
        return 'unparseable'
    sch = load_schema()
    errors = []
    validate(old, sch, '$', errors, sch)
    if not errors:
        return 'compliant_generated'
    # schema-external fields -> look at whether content looks machine generated
    if isinstance(old, dict) and old.get('meta', {}).get('generator'):
        return 'non_compliant_generated'
    return 'manual_suspected'

def cmd_init():
    os.makedirs(PARTS, exist_ok=True)
    reason = decision_on_existing()
    if reason in ('non_compliant_generated', 'manual_suspected', 'unparseable'):
        bak = MAPJSON + '.bak.%s' % datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
        shutil.copy2(MAPJSON, bak)
        print('BACKUP -> %s' % bak)
    # growth.log: archive + truncate rebuild (only legal truncation point)
    if os.path.exists(GROWTH):
        bak = GROWTH + '.%s.bak' % datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
        shutil.copy2(GROWTH, bak)
        print('GROWTH ARCHIVED -> %s' % bak)
    _tmp_write(GROWTH, '')
    set_progress('init', 0, 0, None, 0, decision_reason=reason)
    print('init ok; decision_reason=%s' % reason)

# ---------------------------------------------------------------- emit-module
def module_fragment_schema():
    sch = load_schema()
    mod = sch['properties']['modules']['items']
    return mod

def cmd_emit_module():
    raw = sys.stdin.read()
    obj = json.loads(raw)
    sch = load_schema()
    errors = []
    validate(obj, module_fragment_schema(), '$', errors, sch)
    if 'id' not in obj:
        errors.append('$: missing id')
    if errors:
        print(json.dumps({'ok': False, 'errors': errors}, ensure_ascii=False))
        sys.exit(1)
    unknown = set(obj) - MODULE_KEYS
    if unknown:
        print(json.dumps({'ok': False, 'errors': ['unexpected keys %s' % sorted(unknown)]}, ensure_ascii=False))
        sys.exit(1)
    write_json_atomic(os.path.join(PARTS, obj['id'] + '.json'), obj)
    print(json.dumps({'ok': True, 'part': 'parts/%s.json' % obj['id']}, ensure_ascii=False))

def cmd_emit_edges():
    raw = sys.stdin.read()
    obj = json.loads(raw)   # {"id": "...", "out_edges": [...]}
    # R3 前置过滤：构建期产物托管/内嵌关系不得写入 parts（fail-closed，打回归纳重判）
    pol = map_policy.load_policy(REPO)
    md = map_policy.must_drop_pairs(pol)
    rejected = ['%s->%s' % (e.get('from'), e.get('to')) for e in obj['out_edges']
                if (e.get('from'), e.get('to')) in md]
    if rejected:
        print(json.dumps({'ok': False,
                          'errors': ['build-time relation rejected: %s' % r for r in rejected]},
                         ensure_ascii=False))
        sys.exit(1)
    sch = load_schema()
    item = sch['properties']['edges']['items']
    errors = []
    for e in obj['out_edges']:
        # id is assigned deterministically at finalize; validate the rest of the shape
        probe = dict(e)
        probe.setdefault('id', 'e0')
        validate(probe, item, '$edge', errors, sch)
    if errors:
        print(json.dumps({'ok': False, 'errors': errors}, ensure_ascii=False))
        sys.exit(1)
    write_json_atomic(os.path.join(PARTS, obj['id'] + '.edges.json'),
                      {'id': obj['id'], 'out_edges': obj['out_edges']})
    print(json.dumps({'ok': True, 'part': 'parts/%s.edges.json' % obj['id']}, ensure_ascii=False))

# ---------------------------------------------------------------- append-log
def append_lines(lines):
    with open(GROWTH, 'a', encoding='utf-8') as f:
        for l in lines:
            f.write(json.dumps(l, ensure_ascii=False) + '\n')
        f.flush()
        os.fsync(f.fileno())

def _module_event(mid):
    m = json.load(open(os.path.join(PARTS, mid + '.json'), encoding='utf-8'))
    ep = os.path.join(PARTS, mid + '.edges.json')
    edges = json.load(open(ep, encoding='utf-8'))['out_edges'] if os.path.exists(ep) else []
    ev = dict(m)
    ev['out_edges'] = edges
    return {'type': 'module', 'module': ev}

def cmd_append_log():
    """append-log [--layers | --module <id> | --arch-health]  (no arg = full replay)"""
    order = json.load(open(ORDER, encoding='utf-8'))
    mode = sys.argv[2] if len(sys.argv) > 2 else None
    lines = []
    if mode == '--layers':
        lines = [{'type': 'layer', 'layer': l} for l in order['layers']]
    elif mode == '--module':
        lines = [_module_event(sys.argv[3])]
    elif mode == '--arch-health':
        lines = [{'type': 'arch_health',
                  'health': json.load(open(os.path.join(PARTS, '_arch_health.json'), encoding='utf-8'))}]
    else:
        lines = [{'type': 'layer', 'layer': l} for l in order['layers']]
        lines += [_module_event(mid) for mid in order['modules']]
        lines.append({'type': 'arch_health',
                      'health': json.load(open(os.path.join(PARTS, '_arch_health.json'), encoding='utf-8'))})
    append_lines(lines)
    print(json.dumps({'ok': True, 'appended': len(lines)}, ensure_ascii=False))

# ---------------------------------------------------------------- finalize
def assemble():
    order = json.load(open(ORDER, encoding='utf-8'))
    # R6/c-arch-12：退役边 id 的唯一真值来自 policy（不再读 untracked emit_order.json）
    try:
        retired = set(map_policy.retired_edge_ids(map_policy.load_policy(REPO)))
    except map_policy.PolicyMissing as e:
        print(json.dumps({'ok': False, 'stage': 'policy',
                          'problems': ['policy missing: %s' % e.dotted_key]}, ensure_ascii=False))
        sys.exit(1)
    modules = []
    edges = []
    eid = 0
    for mid in order['modules']:
        m = json.load(open(os.path.join(PARTS, mid + '.json'), encoding='utf-8'))
        ep = os.path.join(PARTS, mid + '.edges.json')
        out = json.load(open(ep, encoding='utf-8'))['out_edges'] if os.path.exists(ep) else []
        for e in out:
            eid += 1
            while 'e%d' % eid in retired:   # 退役 id 永久腾空（INV-4：边 id 稳定）
                eid += 1
            e = dict(e)
            e['id'] = 'e%d' % eid
            if e.get('direction_violation') is None:
                e.pop('direction_violation', None)
            edges.append(e)
        m['dependencies'] = [e['to'] for e in out]
        modules.append(m)
    layers = sorted(order['layers'], key=lambda l: l['order'])
    return layers, modules, edges

def apply_module_correction(modules, edges, mid, patch):
    """校正事件：以补丁的 out_edges 重放某模块出边，dependencies 恒由出边派生（INV-1/INV-2）。"""
    m = next((x for x in modules if x['id'] == mid), None)
    if m is None:
        return
    if 'out_edges' in patch:
        edges[:] = [e for e in edges if e['from'] != mid]
        edges.extend(patch['out_edges'])
        cur_out = patch['out_edges']
    else:
        cur_out = [e for e in edges if e['from'] == mid]
    derived = [e['to'] for e in cur_out]
    if 'dependencies' in patch and patch['dependencies'] != derived:
        print(json.dumps({'ok': False, 'stage': 'replay',
                          'problems': ['correction deps mismatch: %s declared=%s derived=%s'
                                       % (mid, patch['dependencies'], derived)]}, ensure_ascii=False))
        sys.exit(1)
    m['dependencies'] = derived


def replay_from_log():
    lines = [json.loads(l) for l in open(GROWTH, encoding='utf-8') if l.strip()]
    layers, modules, edges, health = [], [], [], None
    for ev in lines:
        t = ev.get('type')
        if t == 'layer':
            layers.append(ev['layer'])
        elif t == 'module':
            m = dict(ev['module'])
            out = m.pop('out_edges')
            m['dependencies'] = [e['to'] for e in out]   # ★ 派生：与 assemble 同源，不再信日志原值
            edges.extend(out)
            modules.append(m)
        elif t == 'arch_health':
            health = ev['health']
        elif t == 'correction':                          # 追加式校正（append-only，未知类型仍静默跳过）
            tgt = ev.get('target', '')
            if tgt == 'arch_health':
                health = dict(health or {})
                health.update(ev.get('patch', {}))
            elif tgt.startswith('module:'):
                apply_module_correction(modules, edges, tgt.split(':', 1)[1], ev.get('patch', {}))
    return layers, modules, edges, health

def check_gates(layers, modules, edges):
    """R4 关口 A/B/C（fail-closed，期望值同源于 policy）。返回问题列表，空 = 通过。"""
    problems = []
    policy = map_policy.load_policy(REPO)
    try:
        scc_max = map_policy.scc_max(policy)
        dv_baseline = map_policy.dv_max(policy)
    except map_policy.PolicyMissing as e:
        return ['P policy missing: %s' % e.dotted_key]
    # 关口 A：构建期产物托管/内嵌关系不得成为依赖边（R3）
    for eid, frm, to in map_policy.policy_violations(edges, policy):
        problems.append('A build-time relation leaked as edge: %s %s->%s' % (eid, frm, to))
    # 关口 B：跨层 SCC 即 fail（只锁跨层；同层互引不阻断）；上限读 policy.gates.scc_max
    layer_order = {l['id']: l['order'] for l in layers}
    mod_order = {m['id']: layer_order.get(m['layer']) for m in modules}
    xscc = map_policy.cross_layer_scc([m['id'] for m in modules], edges, mod_order)
    if len(xscc) > scc_max:
        problems.append('B cross-layer cycle: %s' % xscc)
    # 关口 C：direction_violation 计数棘轮（只降不升）；基线读 policy.gates.dv_max
    dv = sum(1 for e in edges if e.get('direction_violation'))
    if dv > dv_baseline:
        problems.append('C direction_violation %d > baseline %d' % (dv, dv_baseline))
    return problems


def gate_same_generation(policy, layers, modules, edges, health):
    """GATE-1（R5/c-arch-12）：policy ↔ 产物 的集合级同代判据（复用 verify_arch_facts 唯一实现）。

    任一集合级不符即返回问题（调用方 fail-closed 拒绝落盘）。不判顺序/prose，避免无意义假红。
    """
    product = {
        'modules': [{'id': m['id'], 'layer': m['layer'], 'files': m['files'],
                     'health': m.get('health') or {}} for m in modules],
        'edges': edges,
        'layers': [{'id': l['id'], 'order': l['order']} for l in layers],
        'health': health or {},
    }
    return verify_arch_facts.policy_vs_map(policy, product, 'finalize')


def cmd_finalize():
    set_progress('assembling', percent=95)
    schema = load_schema()
    layers, modules, edges = assemble()
    meta_new = json.load(open(META, encoding='utf-8')) if os.path.exists(META) else {}
    files = product_files()
    patterns = [p for m in modules for p in m['files']]
    covered = [f for f in files if any(glob_match(f, p) for p in patterns)]
    uncovered = [f for f in files if f not in set(covered)]
    ratio = (len(covered) / len(files)) if files else 1.0

    meta = {
        'repo': 'EasyVibe',
        'generated_at': now_iso(),
        'generator': 'easyvibe-architect/claude-sonnet-4-6',
        'description': meta_new.get('description', ''),
        'languages': meta_new.get('languages', []),
        'loc': meta_new.get('loc', 0),
        'map_freshness': 'fresh',
        'stats': {
            'files_total': len(files),
            'files_covered': len(covered),
            'coverage_ratio': round(ratio, 4),
            'edges_derived': len(edges),
            'edges_inferred': 0,
            'retried_modules': 0,
        },
    }
    ah = json.load(open(os.path.join(PARTS, '_arch_health.json'), encoding='utf-8'))
    out = {'version': '1.0', 'meta': meta, 'layers': layers, 'modules': modules,
           'edges': edges, 'health': ah}

    errors = []
    validate(out, schema, '$', errors, schema)
    if errors:
        set_progress('failed', percent=95, error='schema: %s' % errors[:5])
        print(json.dumps({'ok': False, 'stage': 'schema', 'errors': errors}, ensure_ascii=False))
        sys.exit(1)
    if ratio <= 0.90:
        set_progress('failed', percent=95, error='coverage %.3f' % ratio)
        print(json.dumps({'ok': False, 'stage': 'coverage', 'ratio': ratio,
                          'uncovered': uncovered[:50]}, ensure_ascii=False))
        sys.exit(1)
    # R4 关口 A/B/C（构建期边 / 跨层环 / DV 棘轮）
    gate_problems = check_gates(layers, modules, edges)
    if gate_problems:
        set_progress('failed', percent=95, error='; '.join(gate_problems[:5]))
        print(json.dumps({'ok': False, 'stage': 'gate', 'problems': gate_problems}, ensure_ascii=False))
        sys.exit(1)
    # c-arch-12 GATE-1：policy ↔ 产物 集合级同代（fail-closed，任一真漂移即拒绝落盘）
    samegen = gate_same_generation(map_policy.load_policy(REPO), layers, modules, edges, ah)
    if samegen:
        set_progress('failed', percent=95, error='; '.join(samegen[:5]))
        print(json.dumps({'ok': False, 'stage': 'same-generation', 'problems': samegen},
                         ensure_ascii=False))
        sys.exit(1)
    # replay compare
    rl, rm, re_, rh = replay_from_log()
    rl_sorted = sorted(rl, key=lambda l: l['order'])
    problems = []
    if rl_sorted != layers:
        problems.append('layers mismatch')
    if rm != modules:
        problems.append('modules mismatch')
    if len(re_) != len(edges):
        problems.append('edge count %d vs log %d' % (len(edges), len(re_)))
    if rh != ah:
        problems.append('arch health mismatch')
    # order continuity
    if [l['order'] for l in layers] != list(range(len(layers))):
        problems.append('layer order not 0..n')
    ids = {m['id'] for m in modules}
    for m in modules:
        if m['layer'] not in {l['id'] for l in layers}:
            problems.append('module %s bad layer' % m['id'])
        if m['dependencies'] != [e['to'] for e in edges if e['from'] == m['id']]:
            problems.append('module %s deps != edges' % m['id'])
    for e in edges:
        if e['from'] not in ids or e['to'] not in ids:
            problems.append('edge %s bad endpoint' % e['id'])
    if problems:
        set_progress('failed', percent=95, error='; '.join(problems[:5]))
        print(json.dumps({'ok': False, 'stage': 'replay', 'problems': problems}, ensure_ascii=False))
        sys.exit(1)
    write_json_atomic(MAPJSON, out)
    set_progress('done', modules_total=len(modules), modules_done=len(modules),
                 current=None, percent=100)
    append_lines([{'type': 'done'}])
    print(json.dumps({'ok': True, 'modules': len(modules), 'edges': len(edges),
                      'files_total': len(files), 'coverage': round(ratio, 4)},
                     ensure_ascii=False))

# ---------------------------------------------------------------- normalize-edges（R1/R5 迁移）
def cmd_normalize_edges():
    """把命中策略 must_drop 的存量依赖边按「同一事务」剥离（幂等，fail-closed，可回滚）。

    步骤：备份 → parts/*.edges.json 过滤 → parts/*.json dependencies 派生 →
          retired_edge_ids 合并 → growth.log append correction → 重跑 finalize。

    叙事单源（INV-5）：arch note 的唯一写入方是 live map 的 `parts/_arch_health.json`；
    本命令**不再**写回任何 arch review note——旧代的硬编码常量与写回路径已删除，
    使任何迁移路径都不可能再用旧叙事覆盖已纠正的 note。
    """
    order = json.load(open(ORDER, encoding='utf-8'))
    policy = map_policy.load_policy(REPO)
    md = map_policy.must_drop_pairs(policy)
    ts = datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
    changes = []
    for mid in order['modules']:
        ep = os.path.join(PARTS, mid + '.edges.json')
        if not os.path.exists(ep):
            continue
        obj = json.load(open(ep, encoding='utf-8'))
        out = obj.get('out_edges', [])
        kept = [e for e in out if (e['from'], e['to']) not in md]
        if len(kept) != len(out):
            removed = [e for e in out if (e['from'], e['to']) in md]
            changes.append({'mid': mid, 'obj': obj, 'kept': kept, 'removed': removed})
    if not changes:
        print(json.dumps({'ok': True, 'changed': 0,
                          'note': 'no must_drop edge present (idempotent no-op)'}, ensure_ascii=False))
        return
    # [0] 备份（回滚单位 = parts + growth.log，见方案 §3.3.6 / Q10）
    backups = []

    def backup(path):
        if os.path.exists(path):
            b = '%s.bak.%s' % (path, ts)
            shutil.copy2(path, b)
            backups.append((path, b))

    backup(GROWTH)
    for ch in changes:
        backup(os.path.join(PARTS, ch['mid'] + '.edges.json'))
        backup(os.path.join(PARTS, ch['mid'] + '.json'))
    backup(ORDER)
    try:
        # [1] 过滤出边（保持相对顺序，不重排）+ [2] dependencies 派生
        for ch in changes:
            ch['obj']['out_edges'] = ch['kept']
            write_json_atomic(os.path.join(PARTS, ch['mid'] + '.edges.json'), ch['obj'])
            mp = os.path.join(PARTS, ch['mid'] + '.json')
            m = json.load(open(mp, encoding='utf-8'))
            m['dependencies'] = [e['to'] for e in ch['kept']]
            write_json_atomic(mp, m)
        # [2b] ✂ 已删除：arch note 写回（INV-5 叙事单源）——不再有可覆盖已纠正 note 的路径
        # [3] 退役 id 合并（永久腾空，INV-4）
        retired = sorted(set(order.get('retired_edge_ids', [])) |
                         {e['id'] for ch in changes for e in ch['removed'] if e.get('id')})
        order['retired_edge_ids'] = retired
        write_json_atomic(ORDER, order)
        # [4] append-only correction（仅 module 面；arch note 不再入日志——见 [2b]）
        corr = []
        for ch in changes:
            corr.append({'type': 'correction', 'target': 'module:' + ch['mid'],
                         'patch': {'out_edges': ch['kept'],
                                   'dependencies': [e['to'] for e in ch['kept']]},
                         'reason': 'R1 build-time hosting is not a dependency edge'})
        append_lines(corr)
        # [5] 重生成 map.json（内部自带 R4 关口 A/B/C + replay 校验）
        cmd_finalize()
    except SystemExit as exc:
        for path, b in backups:   # fail-closed 回滚
            shutil.copy2(b, path)
        print(json.dumps({'ok': False, 'stage': 'normalize-rollback',
                          'restored': [p for p, _ in backups]}, ensure_ascii=False))
        raise SystemExit(exc.code if isinstance(exc.code, int) else 1)
    print(json.dumps({'ok': True, 'changed': len(changes),
                      'retired': retired,
                      'backups': [b for _, b in backups]}, ensure_ascii=False))


# ---------------------------------------------------------------- self-check / coverage
def self_check(m):
    """对一份完整地图做 schema 之外的结构自检（并入自旧 run/ 参考实现）。"""
    errs = []
    lids = [l['id'] for l in m['layers']]
    if [l['order'] for l in m['layers']] != list(range(len(m['layers']))):
        errs.append('layer orders not consecutive from 0')
    if len(set(lids)) != len(lids): errs.append('duplicate layer id')
    mods = {x['id'] for x in m['modules']}
    if len(mods) != len(m['modules']): errs.append('duplicate module id')
    eids = [e['id'] for e in m['edges']]
    if not eids or not all(eids) or len(set(eids)) != len(eids) or \
       not all(EDGE_ID_PAT.match(i) for i in eids):
        errs.append('edge ids missing/dup/bad-format')
    files_cache = product_files()
    for x in m['modules']:
        if x['layer'] not in lids: errs.append('bad layer: %s' % x['id'])
        deps = set(x['dependencies'])
        ef = {e['to'] for e in m['edges'] if e['from'] == x['id']}
        if deps != ef:
            errs.append('deps!=edges: %s symmetric-diff=%s' % (x['id'], deps ^ ef))
        for g in x['files']:
            if not any(glob_match(f, g) for f in files_cache) and \
               not os.path.exists(os.path.join(REPO, g.rstrip('/*'))):
                errs.append('unmatched glob: %s %s' % (x['id'], g))
        validate_health(x['health'], 'mod:' + x['id'], errs)
    for e in m['edges']:
        if e['from'] not in mods or e['to'] not in mods:
            errs.append('edge endpoint missing: %s' % e['id'])
    validate_health(m['health'], 'arch', errs)
    avg = sum(x['health']['score'] for x in m['modules']) / len(m['modules'])
    if abs(m['health']['score'] - round(avg)) < 3:
        errs.append('arch score %s too close to module avg %.1f' % (m['health']['score'], avg))
    return errs, avg

def _load_map_or_die():
    if not os.path.exists(MAPJSON):
        print(json.dumps({'ok': False, 'errors': ['map.json missing']}, ensure_ascii=False))
        sys.exit(1)
    return json.load(open(MAPJSON, encoding='utf-8'))

def cmd_self_check():
    m = _load_map_or_die()
    errs, avg = self_check(m)
    if errs:
        print(json.dumps({'ok': False, 'errors': errs}, ensure_ascii=False))
        sys.exit(1)
    print(json.dumps({'ok': True, 'modules': len(m['modules']), 'avg': round(avg, 1)},
                     ensure_ascii=False))

def cmd_coverage():
    # 归属（本子命令的枚举/coverage）与出边（edging 证据）**正交**：测试面文件仍被
    # product_files() 枚举并归属模块，但按 scripts/map_policy.py::is_test_face 谓词
    # 不参与出边证据提取，故不构成依赖边（c-arch-15 / R4）——此处口径不变、ratio 不降。
    m = _load_map_or_die()
    files = product_files()
    patterns = [p for mod in m['modules'] for p in mod['files']]
    covered = [f for f in files if any(glob_match(f, p) for p in patterns)]
    uncovered = [f for f in files if f not in set(covered)]
    ratio = (len(covered) / len(files)) if files else 1.0
    print(json.dumps({'ok': True, 'files_total': len(files), 'files_covered': len(covered),
                      'coverage_ratio': round(ratio, 4), 'uncovered': uncovered[:50]},
                     ensure_ascii=False))

def cmd_gate_selfcheck():
    """R5/c-arch-12 GATE-1 自证：现网产物必绿；删格/改 glob/改边数注入必红（不写盘）。"""
    policy = map_policy.load_policy(REPO)
    layers, modules, edges = assemble()
    ahp = os.path.join(PARTS, '_arch_health.json')
    ah = json.load(open(ahp, encoding='utf-8')) if os.path.exists(ahp) else {}
    results = []

    def add(name, problems, expect_red):
        red = bool(problems)
        results.append((name, red == expect_red, "; ".join(problems[:1])))

    add("GATE-1 正例（policy ↔ 现网产物）→ 必绿",
        gate_same_generation(policy, layers, modules, edges, ah), False)
    m2 = json.loads(json.dumps(modules))[:-1]                      # 删一格
    add("GATE-1 删一格 → 必红", gate_same_generation(policy, layers, m2, edges, ah), True)
    m3 = json.loads(json.dumps(modules))
    m3[0]['files'] = list(m3[0]['files']) + ['__drift__/**']        # 改一格 glob
    add("GATE-1 改一格 glob → 必红", gate_same_generation(policy, layers, m3, edges, ah), True)
    add("GATE-1 少一条边 → 必红",
        gate_same_generation(policy, layers, modules, edges[:-1], ah), True)
    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    print("GATE-1 自证 %s" % ("全 PASS" if ok else "有 FAIL"))
    return 0 if ok else 1


def main():
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(2)
    cmd = sys.argv[1]
    if cmd == 'finalize' and '--selfcheck' in sys.argv[2:]:
        sys.exit(cmd_gate_selfcheck())
    {'init': cmd_init, 'emit-module': cmd_emit_module, 'emit-edges': cmd_emit_edges,
     'append-log': cmd_append_log, 'finalize': cmd_finalize, 'normalize-edges': cmd_normalize_edges,
     'self-check': cmd_self_check, 'coverage': cmd_coverage}[cmd]()

if __name__ == '__main__':
    main()
