#!/usr/bin/env python3
import os, re, json, collections, fnmatch, sys
REPO = os.environ.get('REPO_ROOT', '/Users/liyuhang/Documents/git_projects/PromptHub-main')
CODE_EXT={'.ts','.tsx','.js','.jsx'}
EXC_SUF=('.test.ts','.test.tsx','.spec.ts','.spec.tsx','vite-env.d.ts','globals.d.ts','env.d.ts','desktop-runtime.d.ts','rehype-highlight.d.ts')
EXC_DIR=('/tests/','/test/','/__tests__/','/dist/','/node_modules/','/.astro/','/generated/')
PRODUCT_DIRS=['apps/desktop/src','apps/web/src','packages/db/src','packages/shared','website/src']

MODS = {
 'shared-foundation': ['packages/shared/**','apps/desktop/src/utils/**','apps/desktop/src/types/**'],
 'data-persistence': ['packages/db/src/**','apps/desktop/src/main/database/**','apps/desktop/src/main/data-path.ts','apps/desktop/src/main/runtime-paths.ts','apps/desktop/src/main/services/prompt-workspace.ts'],
 'skill-system': ['apps/desktop/src/main/services/skill-*.ts'],
 'backup-recovery': ['apps/desktop/src/main/services/upgrade-backup*.ts','apps/desktop/src/main/services/recovery-*.ts','apps/desktop/src/main/services/data-layout-migration.ts'],
 'ai-integration': ['apps/desktop/src/main/services/ai-client.ts'],
 'desktop-state': ['apps/desktop/src/renderer/stores/**','apps/desktop/src/renderer/services/**','apps/desktop/src/renderer/hooks/**'],
 'desktop-ui': ['apps/desktop/src/renderer/components/**','apps/desktop/src/renderer/App.tsx','apps/desktop/src/renderer/main.tsx','apps/desktop/src/renderer/i18n/**','apps/desktop/src/renderer/styles/**','apps/desktop/src/renderer/assets/**','apps/desktop/src/renderer/runtime.ts','apps/desktop/src/renderer/utils/**'],
 'web-api': ['apps/web/src/**'],
 'web-client': ['apps/web/src/client/**'],
 'ipc-bridge': ['apps/desktop/src/main/ipc/**','apps/desktop/src/preload/**'],
 'desktop-runtime': ['apps/desktop/src/main/index.ts','apps/desktop/src/main/menu.ts','apps/desktop/src/main/shortcuts.ts','apps/desktop/src/main/security.ts','apps/desktop/src/main/updater.ts','apps/desktop/src/main/webdav.ts','apps/desktop/src/main/startup-log.ts','apps/desktop/src/main/desktop-cli.ts','apps/desktop/src/main/testing/**','apps/desktop/src/cli/**'],
 'website': ['website/src/**'],
}
# order matters: web-client before web-api
ORDER=['web-client','shared-foundation','data-persistence','skill-system','backup-recovery','ai-integration','desktop-state','desktop-ui','web-api','ipc-bridge','desktop-runtime','website']

def prod_files():
    out=[]
    for d in PRODUCT_DIRS:
        for root,_,fs in os.walk(os.path.join(REPO,d)):
            for f in fs:
                p=os.path.join(root,f); rel=os.path.relpath(p,REPO)
                if os.path.splitext(f)[1] not in CODE_EXT: continue
                if any(x in rel for x in EXC_DIR): continue
                if any(rel.endswith(s) for s in EXC_SUF): continue
                out.append(rel)
    return sorted(out)

ALL=prod_files()
def mod_of(rel):
    for m in ORDER:
        for g in MODS[m]:
            if fnmatch.fnmatch(rel,g): return m
    return None

UNMAPPED=[f for f in ALL if mod_of(f) is None]

def resolve(spec, srcfile):
    # returns rel path candidate or None
    if spec.startswith('@prompthub/shared'):
        return ('packages/shared', spec[len('@prompthub/shared'):].lstrip('/'))
    if spec.startswith('@shared'):
        return ('packages/shared', spec[len('@shared'):].lstrip('/'))
    if spec.startswith('@prompthub/db'):
        return ('packages/db/src', spec[len('@prompthub/db'):].lstrip('/'))
    if spec.startswith('@renderer/'):
        return ('apps/desktop/src/renderer', spec[len('@renderer/'):])
    if spec.startswith('@/'):
        return ('apps/desktop/src', spec[2:])
    if spec.startswith('.'):
        base=os.path.dirname(srcfile)
        return ('REL', os.path.normpath(os.path.join(base,spec)))
    return None

def try_resolve(base, rel):
    if rel in ('', None):
        rel='index'
    cands=[]
    for ext in ('','.ts','.tsx','.js','.jsx','/index.ts','/index.tsx'):
        cands.append(os.path.normpath(base+'/'+rel+ext))
    for c in cands:
        if c and os.path.isfile(os.path.join(REPO,c)):
            # normalize to a product file
            for f in ALL:
                if f==c: return f
    return None

imp_re=re.compile(r"""(?:import|export)\s[^'"]*?from\s*['"]([^'"]+)['"]|import\s*\(\s*['"]([^'"]+)['"]\s*\)|require\(\s*['"]([^'"]+)['"]\s*\)""")
counts=collections.Counter()
samples=collections.defaultdict(list)
for f in ALL:
    src=mod_of(f)
    try: text=open(os.path.join(REPO,f),encoding='utf-8',errors='replace').read()
    except: continue
    for line in text.splitlines():
        m=imp_re.search(line)
        if not m: continue
        spec=m.group(1) or m.group(2) or m.group(3)
        if not spec: continue
        r=resolve(spec,f)
        if not r: continue
        base,rel=r
        if base=='REL':
            tgt=try_resolve(os.path.dirname(f), os.path.relpath(rel, os.path.dirname(f)) if not rel.startswith('..') else rel)
        else:
            tgt=try_resolve(base, rel)
        if not tgt: continue
        tmod=mod_of(tgt)
        if tmod and tmod!=src:
            counts[(src,tmod)]+=1
            if len(samples[(src,tmod)])<3: samples[(src,tmod)].append(f'{f} -> {spec}')

print('UNMAPPED FILES:', len(UNMAPPED))
for u in UNMAPPED: print('  ',u)
print('\nCROSS-MODULE IMPORT COUNTS:')
for (a,b),c in sorted(counts.items(), key=lambda x:-x[1]):
    print(f'  {a} -> {b}: {c}')
print('\nsamples:')
for k,v in sorted(samples.items()):
    print(' ',k, v)
