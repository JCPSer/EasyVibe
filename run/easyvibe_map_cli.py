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
    modules = []
    edges = []
    eid = 0
    for mid in order['modules']:
        m = json.load(open(os.path.join(PARTS, mid + '.json'), encoding='utf-8'))
        ep = os.path.join(PARTS, mid + '.edges.json')
        out = json.load(open(ep, encoding='utf-8'))['out_edges'] if os.path.exists(ep) else []
        for e in out:
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

def replay_from_log():
    lines = [json.loads(l) for l in open(GROWTH, encoding='utf-8') if l.strip()]
    layers, modules, edges, health = [], [], [], None
    for ev in lines:
        if ev['type'] == 'layer':
            layers.append(ev['layer'])
        elif ev['type'] == 'module':
            m = dict(ev['module'])
            out = m.pop('out_edges')
            edges.extend(out)
            modules.append(m)
        elif ev['type'] == 'arch_health':
            health = ev['health']
    return layers, modules, edges, health

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
    m = _load_map_or_die()
    files = product_files()
    patterns = [p for mod in m['modules'] for p in mod['files']]
    covered = [f for f in files if any(glob_match(f, p) for p in patterns)]
    uncovered = [f for f in files if f not in set(covered)]
    ratio = (len(covered) / len(files)) if files else 1.0
    print(json.dumps({'ok': True, 'files_total': len(files), 'files_covered': len(covered),
                      'coverage_ratio': round(ratio, 4), 'uncovered': uncovered[:50]},
                     ensure_ascii=False))

def main():
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(2)
    cmd = sys.argv[1]
    {'init': cmd_init, 'emit-module': cmd_emit_module, 'emit-edges': cmd_emit_edges,
     'append-log': cmd_append_log, 'finalize': cmd_finalize,
     'self-check': cmd_self_check, 'coverage': cmd_coverage}[cmd]()

if __name__ == '__main__':
    main()
