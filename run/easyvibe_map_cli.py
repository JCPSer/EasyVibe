#!/usr/bin/env python3
"""easyvibe-map CLI — prompt v2.2 reference impl, adapted for PromptHub monorepo.
Subcommands: init / emit-module / append-log / finalize
All shared-file updates are tmp+rename; no locks.
"""
import json, os, sys, fnmatch, datetime, re, collections

REPO = os.environ.get('REPO_ROOT', os.getcwd())
MAP_DIR = os.path.join(REPO, '.easyvibe', 'map')
PARTS = os.path.join(MAP_DIR, 'parts')
PROGRESS = os.path.join(MAP_DIR, 'progress.json')
GROWTH = os.path.join(MAP_DIR, 'growth.log')
MAPJSON = os.path.join(MAP_DIR, 'map.json')
META = os.path.join(MAP_DIR, 'meta.json')
ORDER = os.path.join(MAP_DIR, 'emit_order.json')
SCHEMA = os.environ.get('SCHEMA_PATH',
    '/Users/liyuhang/Documents/EasyVibe/easyvibe-map-schema-v1.json')

CODE_EXT = {'.ts', '.tsx', '.js', '.jsx', '.astro', '.py'}
# product code roots (exclude tests, tooling, docs, build output)
PRODUCT_DIRS = ['apps/desktop/src', 'apps/web/src', 'packages/db/src',
                'packages/shared', 'website/src']
# files matching these glob-ish suffixes are NOT product code
EXCLUDE_SUFFIX = ('.test.ts', '.test.tsx', '.spec.ts', '.spec.tsx',
                  'vite-env.d.ts', 'globals.d.ts', 'env.d.ts', 'desktop-runtime.d.ts',
                  'rehype-highlight.d.ts')
EXCLUDE_DIR_PARTS = ('/tests/', '/test/', '/__tests__/', '/dist/', '/node_modules/',
                     '/.astro/', '/generated/')

ALLOWED_TOP = {'version', 'meta', 'layers', 'modules', 'edges', 'health'}
ALLOWED_META = {'repo', 'generated_at', 'last_patrol_at', 'stats', 'generator',
                'description', 'languages', 'loc', 'map_freshness'}
ALLOWED_STATS = {'files_total', 'files_covered', 'coverage_ratio', 'edges_derived',
                 'edges_inferred', 'retried_modules'}
ALLOWED_LAYER = {'id', 'name', 'order', 'description'}
ALLOWED_MOD = {'id', 'last_analyzed_at', 'name', 'layer', 'responsibility', 'files',
               'key_entries', 'dependencies', 'health', 'notes'}
ALLOWED_EDGE = {'id', 'from', 'to', 'type', 'label', 'strength', 'direction_violation'}
ALLOWED_HEALTH = {'score', 'coupling', 'complexity', 'churn', 'decay_flags',
                  'review_note', 'concerns'}
ALLOWED_KE = {'file', 'symbol', 'kind'}
ALLOWED_CONCERN = {'severity', 'finding', 'suggestion'}
ID_PAT = re.compile(r'^[a-z][a-z0-9_-]*$')
EDGE_ID_PAT = re.compile(r'^e[a-z0-9_-]*$')
KE_KINDS = ('function', 'class', 'interface', 'route', 'cli', 'job', 'config')


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
    """Fields outside the Schema => possible manual edit."""
    if set(old) - ALLOWED_TOP:
        return True
    if set(old.get('meta', {})) - ALLOWED_META:
        return True
    if set(old.get('meta', {}).get('stats', {})) - ALLOWED_STATS:
        return True
    for l in old.get('layers', []):
        if set(l) - ALLOWED_LAYER: return True
    for m in old.get('modules', []):
        if set(m) - ALLOWED_MOD: return True
        if set(m.get('health', {})) - ALLOWED_HEALTH: return True
        for c in m.get('health', {}).get('concerns', []) or []:
            if set(c) - ALLOWED_CONCERN: return True
    for e in old.get('edges', []):
        if set(e) - ALLOWED_EDGE: return True
    if set(old.get('health', {})) - ALLOWED_HEALTH:
        return True
    for c in old.get('health', {}).get('concerns', []) or []:
        if set(c) - ALLOWED_CONCERN: return True
    return False


def validate_health(h, where, errs):
    if not isinstance(h, dict):
        errs.append(f'{where}: health not object'); return
    if set(h) - ALLOWED_HEALTH:
        errs.append(f'{where}: health extra fields {set(h) - ALLOWED_HEALTH}')
    if not (isinstance(h.get('score'), int) and 0 <= h['score'] <= 100):
        errs.append(f'{where}: score range')
    if not isinstance(h.get('score'), int):
        errs.append(f'{where}: score must be int')
    if h.get('coupling') not in ('low', 'medium', 'high', 'critical'):
        errs.append(f'{where}: coupling enum')
    if h.get('complexity') not in ('low', 'medium', 'high'):
        errs.append(f'{where}: complexity enum')
    if 'churn' in h and h['churn'] not in ('low', 'medium', 'high'):
        errs.append(f'{where}: churn enum')
    if 'decay_flags' in h and not isinstance(h['decay_flags'], list):
        errs.append(f'{where}: decay_flags not list')
    cons = h.get('concerns')
    if cons is not None:
        if not isinstance(cons, list):
            errs.append(f'{where}: concerns not list')
        else:
            if len(cons) > 3:
                errs.append(f'{where}: concerns >3')
            for c in cons:
                if set(c) - ALLOWED_CONCERN:
                    errs.append(f'{where}: concern extra fields')
                if set(c) < ALLOWED_CONCERN:
                    errs.append(f'{where}: concern missing fields')
                if c.get('severity') not in ('critical', 'high'):
                    errs.append(f'{where}: concern severity enum')


def validate_module_fragment(m):
    errs = []
    for k in ('id', 'name', 'layer', 'responsibility', 'files', 'key_entries',
              'dependencies', 'health'):
        if k not in m: errs.append(f'missing {k}')
    if errs: return errs
    if set(m) - ALLOWED_MOD: errs.append(f'extra fields {set(m) - ALLOWED_MOD}')
    if not ID_PAT.match(m['id']): errs.append('bad id pattern')
    if len(m['responsibility']) > 40: errs.append('responsibility >40 chars')
    if not m['files']: errs.append('empty files')
    for ke in m.get('key_entries', []):
        if set(ke) - ALLOWED_KE: errs.append('key_entry extra fields')
        if ke.get('kind') not in KE_KINDS: errs.append(f"key_entry kind: {ke.get('kind')}")
        for f in ('file', 'symbol', 'kind'):
            if f not in ke: errs.append(f'key_entry missing {f}')
    validate_health(m['health'], m['id'], errs)
    return errs


def cmd_init():
    os.makedirs(PARTS, exist_ok=True)
    old = read_json(MAPJSON)
    decision = 'none'
    backup_name = None
    if old is None:
        if os.path.exists(MAPJSON):
            decision = 'unparseable'
    else:
        if detect_manual_edits(old):
            decision = 'manual_suspected'
        else:
            decision = 'compliant_generated'
        if decision in ('manual_suspected', 'non_compliant_generated'):
            ts = now().replace(':', '').replace('+', '_')
            backup_name = f'map.json.bak-{ts}'
            write_atomic(os.path.join(MAP_DIR, backup_name),
                         json.dumps(old, ensure_ascii=False, indent=2) + '\n')
    # growth.log archival + rebuild
    if os.path.exists(GROWTH):
        ts = now().replace(':', '').replace('+', '_')
        os.replace(GROWTH, os.path.join(MAP_DIR, f'growth.log.{ts}.bak'))
    write_atomic(GROWTH, '')
    set_progress('init', modules_total=0, modules_done=0, current_module=None,
                 percent=0, decision_reason=decision)
    if backup_name:
        p = read_json(PROGRESS); p['backup_file'] = backup_name
        write_atomic(PROGRESS, json.dumps(p, ensure_ascii=False, indent=2) + '\n')
    print(f'[init] existing map.json: {decision}' + (f' (backup {backup_name})' if backup_name else ''))


def cmd_emit_module():
    m = json.load(sys.stdin)
    errs = validate_module_fragment(m)
    if errs:
        print(f'[emit-module] REJECTED {m.get("id")}: {errs}', file=sys.stderr)
        sys.exit(1)
    write_atomic(os.path.join(PARTS, m['id'] + '.json'),
                 json.dumps(m, ensure_ascii=False, indent=2) + '\n')
    print(f'[emit-module] OK {m["id"]}')


def cmd_emit_edges():
    """stdin: JSON list of edges for one module; writes parts/<id>.edges.json"""
    payload = json.load(sys.stdin)
    mid = payload['module']
    edges = payload['edges']
    write_atomic(os.path.join(PARTS, mid + '.edges.json'),
                 json.dumps(edges, ensure_ascii=False, indent=2) + '\n')
    print(f'[emit-edges] OK {mid} ({len(edges)} edges)')


def cmd_append_log():
    order = read_json(ORDER); assert order, 'emit_order.json missing'
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
    print(f'[append-log] appended {len(lines)} events '
          f'({len(order["layers"])} layers, {len(order["modules"])} modules)')


def reconstruct_from_log():
    layers, modules, edges, arch = [], [], [], None
    for ln in open(GROWTH, encoding='utf-8'):
        ln = ln.strip()
        if not ln: continue
        ev = json.loads(ln)
        if ev['type'] == 'layer': layers.append(ev['layer'])
        elif ev['type'] == 'module':
            modules.append(ev['module']); edges.extend(ev.get('out_edges', []))
        elif ev['type'] == 'arch_health': arch = ev['health']
    return layers, modules, edges, arch


def is_product_file(path):
    if os.path.splitext(path)[1] not in CODE_EXT:
        return False
    if any(part in path for part in EXCLUDE_DIR_PARTS):
        return False
    if any(path.endswith(s) for s in EXCLUDE_SUFFIX):
        return False
    return True


def list_product_files():
    files = []
    for d in PRODUCT_DIRS:
        base = os.path.join(REPO, d)
        for root, _, fs in os.walk(base):
            for f in fs:
                p = os.path.join(root, f)
                rel = os.path.relpath(p, REPO)
                if is_product_file(rel):
                    files.append(rel)
    return sorted(files)


def coverage(modules):
    files = list_product_files()
    pats = [p for m in modules for p in m['files']]
    missed = [f for f in files if not any(fnmatch.fnmatch(f, p) for p in pats)]
    cov = (len(files) - len(missed)) / max(len(files), 1)
    return cov, len(files), missed


def self_check(m):
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
    for x in m['modules']:
        if x['layer'] not in lids: errs.append(f"bad layer: {x['id']}")
        deps = set(x['dependencies'])
        ef = {e['to'] for e in m['edges'] if e['from'] == x['id']}
        if deps != ef:
            errs.append(f'deps!=edges: {x["id"]} symmetric-diff={deps ^ ef}')
        for g in x['files']:
            if not any(fnmatch.fnmatch(f, g) for f in list_product_files()) and \
               not os.path.exists(os.path.join(REPO, g.rstrip('/*'))):
                errs.append(f'unmatched glob: {x["id"]} {g}')
        validate_health(x['health'], 'mod:' + x['id'], errs)
    for e in m['edges']:
        if e['from'] not in mods or e['to'] not in mods:
            errs.append(f'edge endpoint missing: {e["id"]}')
    validate_health(m['health'], 'arch', errs)
    avg = sum(x['health']['score'] for x in m['modules']) / len(m['modules'])
    if abs(m['health']['score'] - round(avg)) < 3:
        errs.append(f'arch score {m["health"]["score"]} too close to module avg {avg:.1f}')
    return errs, avg


def cmd_finalize():
    meta = read_json(META)
    assert meta, 'meta.json missing'
    st = meta.get('stats')
    layers, modules, edges, arch = reconstruct_from_log()
    assert arch is not None, 'arch_health event missing in growth.log'
    layers = sorted(layers, key=lambda l: l['order'])
    # gate 3 pre: parts vs log
    for x in modules:
        part = read_json(os.path.join(PARTS, x['id'] + '.json'))
        assert part == x, f'gate3 replay mismatch: {x["id"]}'
    # gate 4: coverage -> fill meta.stats
    cov, total, missed = coverage(modules)
    assert cov > 0.9, f'gate4 coverage {cov:.1%} <= 90%, missed: {missed[:10]}'
    covered = total - len(missed)
    meta['stats'] = {
        'files_total': total,
        'files_covered': covered,
        'coverage_ratio': round(cov, 4),
        'edges_derived': len(edges),
        'edges_inferred': 0,
        'retried_modules': 0,
    }
    candidate = {'version': '1.0', 'meta': meta, 'layers': layers,
                 'modules': modules, 'edges': edges, 'health': arch}
    # gate 1: schema
    import jsonschema
    jsonschema.validate(candidate, json.load(open(SCHEMA)))
    print('[finalize] gate1 schema: PASS')
    # gate 2
    errs, avg = self_check(candidate)
    assert not errs, f'gate2 self-check FAILED: {errs}'
    print(f'[finalize] gate2 self-check: PASS (module avg={avg:.1f})')
    print(f'[finalize] gate3 replay==parts: PASS ({len(modules)} modules)')
    print(f'[finalize] gate4 coverage: PASS ({cov:.1%}, {len(missed)} missed)')
    write_atomic(MAPJSON, json.dumps(candidate, ensure_ascii=False, indent=2) + '\n')
    with open(GROWTH, 'a', encoding='utf-8') as f:
        f.write(json.dumps({'type': 'done'}, ensure_ascii=False) + '\n')
    set_progress('done', percent=100, current_module=None)
    print('[finalize] gate5 atomic write: PASS -> map.json, phase=done')


if __name__ == '__main__':
    {'init': cmd_init, 'emit-module': cmd_emit_module,
     'emit-edges': cmd_emit_edges, 'append-log': cmd_append_log,
     'finalize': cmd_finalize}[sys.argv[1]]()
