#!/usr/bin/env python3
"""easyvibe-map CLI — 提示词 v2 的参考实现。
子命令: init / emit-module / append-log / finalize
共享文件更新一律「临时文件 + rename」，无锁。
"""
import json, os, sys, fnmatch, datetime, re

MAP_DIR = '.easyvibe/map'
PARTS = os.path.join(MAP_DIR, 'parts')
PROGRESS = os.path.join(MAP_DIR, 'progress.json')
GROWTH = os.path.join(MAP_DIR, 'growth.log')
MAPJSON = os.path.join(MAP_DIR, 'map.json')
META = os.path.join(MAP_DIR, 'meta.json')
ORDER = os.path.join(MAP_DIR, 'emit_order.json')
SCHEMA = os.environ.get('SCHEMA_PATH',
    '/Users/liyuhang/Documents/EasyVibe/easyvibe-map-schema-v1.json')
CODE_EXT = {'.dart', '.ts', '.tsx', '.js', '.jsx', '.py', '.go', '.rs',
            '.java', '.kt', '.swift', '.cs', '.cpp', '.c', '.h', '.rb', '.php'}

ALLOWED_TOP = {'version', 'meta', 'layers', 'modules', 'edges', 'health'}
ALLOWED_LAYER = {'id', 'name', 'order', 'description'}
ALLOWED_MOD = {'id', 'name', 'layer', 'responsibility', 'files',
               'key_entries', 'dependencies', 'health', 'notes'}
ALLOWED_EDGE = {'from', 'to', 'type', 'label', 'strength', 'direction_violation'}
ALLOWED_HEALTH = {'score', 'coupling', 'complexity', 'churn', 'decay_flags', 'review_note'}
ID_PAT = re.compile(r'^[a-z][a-z0-9_-]*$')

def now():
    return datetime.datetime.now().astimezone().isoformat(timespec='seconds')

def write_atomic(path, text):
    tmp = path + '.tmp'
    with open(tmp, 'w', encoding='utf-8') as f:
        f.write(text)
    os.replace(tmp, path)

def read_json(path, default=None):
    try:
        with open(path, encoding='utf-8') as f:
            return json.load(f)
    except Exception:
        return default

def set_progress(phase, **kw):
    p = read_json(PROGRESS, {}) or {}
    p.update(kw)
    p['phase'] = phase
    p['updated_at'] = now()
    if phase == 'failed' and 'error' not in kw:
        p['error'] = 'unknown'
    write_atomic(PROGRESS, json.dumps(p, ensure_ascii=False, indent=2) + '\n')

def detect_manual_edits(old):
    """Schema 之外的字段 = 人工修改痕迹。"""
    if set(old) - ALLOWED_TOP:
        return True
    for l in old.get('layers', []):
        if set(l) - ALLOWED_LAYER: return True
    for m in old.get('modules', []):
        if set(m) - ALLOWED_MOD: return True
    for e in old.get('edges', []):
        if set(e) - ALLOWED_EDGE: return True
    if set(old.get('health', {})) - ALLOWED_HEALTH:
        return True
    return False

def validate_module_fragment(m):
    errs = []
    for k in ('id', 'name', 'layer', 'responsibility', 'files', 'dependencies', 'health'):
        if k not in m: errs.append(f'missing {k}')
    if errs: return errs
    if not ID_PAT.match(m['id']): errs.append('bad id pattern')
    if len(m['responsibility']) > 40: errs.append('responsibility >40 chars')
    if not m['files']: errs.append('empty files')
    h = m['health']
    if not (0 <= h.get('score', -1) <= 100): errs.append('score range')
    if h.get('coupling') not in ('low', 'medium', 'high', 'critical'): errs.append('coupling enum')
    if h.get('complexity') not in ('low', 'medium', 'high'): errs.append('complexity enum')
    if h.get('churn') not in ('low', 'medium', 'high'): errs.append('churn enum')
    for ke in m.get('key_entries', []):
        if ke.get('kind') not in ('function', 'class', 'interface', 'route', 'cli', 'job', 'config'):
            errs.append(f"key_entry kind: {ke.get('kind')}")
    return errs

def cmd_init():
    os.makedirs(PARTS, exist_ok=True)
    old = read_json(MAPJSON)
    decision = 'none'
    if old is not None:
        if detect_manual_edits(old):
            bak = f'{MAPJSON}.bak-{now().replace(":", "").replace("+", "_")}'
            write_atomic(bak, json.dumps(old, ensure_ascii=False, indent=2) + '\n')
            decision = f'backed_up:{os.path.basename(bak)}'
        else:
            decision = 'overwritten(pure-generated)'
    set_progress('init', modules_total=0, modules_done=0,
                 current_module=None, percent=0,
                 existing_file_decision=decision)
    print(f'[init] existing map.json: {decision}')

def cmd_emit_module():
    m = json.load(sys.stdin)
    errs = validate_module_fragment(m)
    if errs:
        print(f'[emit-module] REJECTED {m.get("id")}: {errs}', file=sys.stderr)
        sys.exit(1)
    write_atomic(os.path.join(PARTS, m['id'] + '.json'),
                 json.dumps(m, ensure_ascii=False, indent=2) + '\n')
    print(f'[emit-module] OK {m["id"]}')

def cmd_append_log():
    """orchestrator 独占：按 emit_order.json 的层序把 parts/ 追加进 growth.log。"""
    order = read_json(ORDER)
    assert order, 'emit_order.json missing'
    lines = []
    for layer in order['layers']:
        lines.append(json.dumps({'type': 'layer', 'layer': layer}, ensure_ascii=False))
    for mid in order['modules']:
        part = read_json(os.path.join(PARTS, mid + '.json'))
        assert part, f'parts/{mid}.json missing'
        out_edges = read_json(os.path.join(PARTS, mid + '.edges.json'), []) or []
        lines.append(json.dumps({'type': 'module', 'module': part,
                                 'out_edges': out_edges}, ensure_ascii=False))
    arch = read_json(os.path.join(PARTS, '_arch_health.json'))
    assert arch, 'parts/_arch_health.json missing'
    lines.append(json.dumps({'type': 'arch_health', 'health': arch}, ensure_ascii=False))
    with open(GROWTH, 'a', encoding='utf-8') as f:
        for ln in lines:
            f.write(ln + '\n')
    print(f'[append-log] appended {len(lines)} events ({len(order["layers"])} layers, {len(order["modules"])} modules)')

def reconstruct_from_log():
    layers, modules, edges, arch = [], [], [], None
    for ln in open(GROWTH, encoding='utf-8'):
        ev = json.loads(ln)
        if ev['type'] == 'layer': layers.append(ev['layer'])
        elif ev['type'] == 'module':
            modules.append(ev['module']); edges.extend(ev.get('out_edges', []))
        elif ev['type'] == 'arch_health': arch = ev['health']
    return layers, modules, edges, arch

def coverage(modules):
    files = []
    for root, _, fs in os.walk('lib'):
        for f in fs:
            if os.path.splitext(f)[1] in CODE_EXT:
                files.append(os.path.join(root, f))
    pats = [f for m in modules for f in m['files']]
    missed = [f for f in files if not any(fnmatch.fnmatch(f, p) for p in pats)]
    cov = (len(files) - len(missed)) / max(len(files), 1)
    return cov, missed

def self_check(m):
    errs = []
    lids = [l['id'] for l in m['layers']]
    if [l['order'] for l in m['layers']] != list(range(len(m['layers']))):
        errs.append('layer orders not consecutive from 0')
    mods = {x['id'] for x in m['modules']}
    for x in m['modules']:
        if x['layer'] not in lids: errs.append(f"bad layer: {x['id']}")
        deps = set(x['dependencies'])
        ef = {e['to'] for e in m['edges'] if e['from'] == x['id']}
        et = {e['from'] for e in m['edges'] if e['to'] == x['id']}
        if deps != ef: errs.append(f'deps!=edges: {x["id"]} symmetric-diff={deps ^ ef}')
        for e in m['edges']:
            if e['from'] not in mods or e['to'] not in mods:
                errs.append(f'edge endpoint missing: {e}')
                break
    arch = m['health']
    avg = sum(x['health']['score'] for x in m['modules']) / len(m['modules'])
    if abs(arch['score'] - round(avg)) < 3:
        errs.append(f'arch score {arch["score"]} suspiciously close to module average {avg:.1f}')
    return errs, avg

def cmd_finalize():
    meta = read_json(META); assert meta, 'meta.json missing'
    layers, modules, edges, arch = reconstruct_from_log()
    assert arch is not None, 'arch_health event missing in growth.log'
    layers = sorted(layers, key=lambda l: l['order'])  # 存储序=order 升序；生长序仅用于追加
    candidate = {'version': '1.0', 'meta': meta, 'layers': layers,
                 'modules': modules, 'edges': edges, 'health': arch}
    # 门 1: Schema 校验
    try:
        import jsonschema
        jsonschema.validate(candidate, json.load(open(SCHEMA)))
        print('[finalize] gate1 schema: PASS (jsonschema)')
    except ImportError:
        print('[finalize] gate1 schema: jsonschema lib absent, skipped')
    # 门 2: 自检清单
    errs, avg = self_check(candidate)
    assert not errs, f'gate2 self-check FAILED: {errs}'
    print(f'[finalize] gate2 self-check: PASS (module avg={avg:.1f}, arch independent)')
    # 门 3: 重放比对（候选即由日志重建，此处验证 parts 与日志一致）
    for x in modules:
        part = read_json(os.path.join(PARTS, x['id'] + '.json'))
        assert part == x, f'gate3 replay mismatch: {x["id"]}'
    print(f'[finalize] gate3 replay==parts: PASS ({len(modules)} modules)')
    # 门 4: 覆盖率
    cov, missed = coverage(modules)
    assert cov > 0.9, f'gate4 coverage {cov:.1%} <= 90%, missed: {missed[:10]}'
    print(f'[finalize] gate4 coverage: PASS ({cov:.1%}, {len(missed)} missed)')
    # 门 5: 原子写入 + done
    write_atomic(MAPJSON, json.dumps(candidate, ensure_ascii=False, indent=2) + '\n')
    with open(GROWTH, 'a', encoding='utf-8') as f:
        f.write(json.dumps({'type': 'done'}, ensure_ascii=False) + '\n')
    set_progress('done', percent=100, current_module=None)
    print('[finalize] gate5 atomic write: PASS -> map.json, phase=done')

if __name__ == '__main__':
    {'init': cmd_init, 'emit-module': cmd_emit_module,
     'append-log': cmd_append_log, 'finalize': cmd_finalize}[sys.argv[1]]()
