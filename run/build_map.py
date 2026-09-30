#!/usr/bin/env python3
import json, os, subprocess, sys, datetime

REPO='/Users/liyuhang/Documents/git_projects/PromptHub-main'
CLI='/Users/liyuhang/Documents/EasyVibe/run/easyvibe_map_cli.py'
MAP=os.path.join(REPO,'.easyvibe','map')
env=dict(os.environ, REPO_ROOT=REPO, SCHEMA_PATH='/Users/liyuhang/Documents/EasyVibe/easyvibe-map-schema-v1.json')

def cli(*args, stdin=None):
    r=subprocess.run([sys.executable,CLI,*args],input=stdin,text=True,capture_output=True,env=env)
    if r.returncode!=0:
        print('CLI FAIL',args,r.stdout,r.stderr); sys.exit(1)
    return r.stdout.strip()

def atomic(path,obj):
    tmp=path+'.tmp'
    open(tmp,'w',encoding='utf-8').write(json.dumps(obj,ensure_ascii=False,indent=2)+'\n')
    os.replace(tmp,path)

def progress(phase, **kw):
    p=json.load(open(os.path.join(MAP,'progress.json')))
    p.update(kw); p['phase']=phase
    p['updated_at']=datetime.datetime.now().astimezone().isoformat(timespec='seconds')
    atomic(os.path.join(MAP,'progress.json'),p)

now=datetime.datetime.now().astimezone().isoformat(timespec='seconds')

# ---------- scanning done -> progress 10 ----------
progress('scanning', percent=10)

layers=[
 {"id":"foundation","name":"基础支撑层","order":5,"description":"跨端共享的类型定义、IPC 频道常量与平台/技能注册表，无库内依赖"},
 {"id":"data","name":"数据持久层","order":4,"description":"SQLite(WASM) 适配器、schema、Prompt/Folder/Skill CRUD 与运行期数据路径"},
 {"id":"domain","name":"领域能力层","order":3,"description":"技能安装校验与多平台分发、升级备份恢复、主进程 AI 调用等独立业务能力"},
 {"id":"application","name":"应用服务层","order":2,"description":"渲染进程状态与前端服务、IPC 契约桥、自部署 Web 服务端路由与域服务"},
 {"id":"interface","name":"交互呈现层","order":1,"description":"桌面 React 界面与多语言、Web 客户端页面、Astro 官网"},
 {"id":"app-entry","name":"应用装配层","order":0,"description":"Electron 主进程装配根（窗口/菜单/更新/安全）与命令行入口"},
]

modules=[
 {"id":"shared-foundation","name":"共享基础",
  "layer":"foundation","responsibility":"跨端共享的类型定义、IPC 频道与平台/技能常量",
  "files":["packages/shared/**","apps/desktop/src/utils/**","apps/desktop/src/types/**"],
  "key_entries":[
    {"file":"packages/shared/types/prompt.ts","symbol":"Prompt","kind":"interface"},
    {"file":"packages/shared/constants/ipc-channels.ts","symbol":"IPC_CHANNELS","kind":"config"},
    {"file":"packages/shared/types/settings.ts","symbol":"DEFAULT_SETTINGS","kind":"config"}],
  "dependencies":[],
  "health":{"score":88,"coupling":"high","complexity":"low","churn":"medium","decay_flags":[],
    "review_note":"被全库各层引用的稳定叶子模块，耦合计数高但均为类型/常量级引用，风险低；应继续禁止其反向依赖任何上层。"}},

 {"id":"data-persistence","name":"数据持久化",
  "layer":"data","responsibility":"SQLite(WASM) 适配、schema 与三表 CRUD",
  "files":["packages/db/src/**","apps/desktop/src/main/database/**","apps/desktop/src/main/data-path.ts","apps/desktop/src/main/runtime-paths.ts","apps/desktop/src/main/services/prompt-workspace.ts"],
  "key_entries":[
    {"file":"packages/db/src/adapter.ts","symbol":"DatabaseAdapter","kind":"class"},
    {"file":"packages/db/src/schema.ts","symbol":"SCHEMA","kind":"config"},
    {"file":"apps/desktop/src/main/database/index.ts","symbol":"initDatabase","kind":"function"},
    {"file":"packages/db/src/prompt.ts","symbol":"PromptDB","kind":"class"}],
  "dependencies":["shared-foundation"],
  "health":{"score":82,"coupling":"medium","complexity":"medium","churn":"medium","decay_flags":[],
    "review_note":"packages/db 与主进程 database/ 之间的兼容 re-export 层清晰；主进程 index.ts 偏重，见 concerns。"},
  "concerns":[{"severity":"high",
    "finding":"apps/desktop/src/main/database/index.ts 达 598 行，混入初始化、可恢复库探测与恢复执行三类职责",
    "suggestion":"把恢复探测/执行下沉到 recovery 模块，index 只保留初始化与数据库句柄管理"}]},

 {"id":"skill-system","name":"技能系统",
  "layer":"domain","responsibility":"SKILL.md 安装、校验、多平台分发与仓库同步",
  "files":["apps/desktop/src/main/services/skill-*.ts"],
  "key_entries":[
    {"file":"apps/desktop/src/main/services/skill-installer.ts","symbol":"SkillInstaller","kind":"class"},
    {"file":"apps/desktop/src/main/services/skill-validator.ts","symbol":"parseSkillMd","kind":"function"},
    {"file":"apps/desktop/src/main/services/skill-repo-sync.ts","symbol":"syncFrontmatterToRepo","kind":"function"}],
  "dependencies":["shared-foundation","data-persistence","ai-integration"],
  "health":{"score":76,"coupling":"medium","complexity":"medium","churn":"high","decay_flags":[],
    "review_note":"2100 行巨石已拆为 11 个聚焦子模块，方向正确；下一步应让调用方直接依赖子模块而非 barrel。",
    "concerns":[{"severity":"high",
      "finding":"skill-installer.ts 作为拆分后的 barrel，11 个子模块对外仍只暴露 SkillInstaller 单入口，边界靠约定维持",
      "suggestion":"让 installer/repo/platform/remote 的调用方各自收敛到对应子模块，逐步废弃 barrel 转发并加导入约束"}]}},

 {"id":"backup-recovery","name":"备份与恢复",
  "layer":"domain","responsibility":"升级前后数据快照、旧数据布局迁移与数据库恢复",
  "files":["apps/desktop/src/main/services/upgrade-backup*.ts","apps/desktop/src/main/services/recovery-*.ts","apps/desktop/src/main/services/data-layout-migration.ts"],
  "key_entries":[
    {"file":"apps/desktop/src/main/services/upgrade-backup-startup.ts","symbol":"runUpgradeBackupStartupTasks","kind":"function"},
    {"file":"apps/desktop/src/main/services/recovery-candidates.ts","symbol":"buildUpgradeBackupRecoveryCandidate","kind":"function"},
    {"file":"apps/desktop/src/main/services/data-layout-migration.ts","symbol":"isDataLayoutFullyMigrated","kind":"function"}],
  "dependencies":["data-persistence","shared-foundation"],
  "health":{"score":74,"coupling":"medium","complexity":"medium","churn":"medium","decay_flags":[],
    "review_note":"启动期任务与恢复候选构造边界清楚；但备份格式在三端各有一份，见 concerns。",
    "concerns":[{"severity":"high",
      "finding":"备份/恢复路径分散在 desktop main（upgrade-backup*）、desktop renderer（database-backup.ts）与 web（backup.service.ts）三处，格式与保留策略各自维护",
      "suggestion":"将备份文件格式、版本字段与保留策略下沉到 packages 共享，各端只保留存储 IO 适配"}]}},

 {"id":"ai-integration","name":"AI 调用",
  "layer":"domain","responsibility":"主进程多供应商 AI 调用（OpenAI 兼容，用于安全扫描）",
  "files":["apps/desktop/src/main/services/ai-client.ts"],
  "key_entries":[
    {"file":"apps/desktop/src/main/services/ai-client.ts","symbol":"chatCompletion","kind":"function"}],
  "dependencies":["shared-foundation"],
  "health":{"score":76,"coupling":"low","complexity":"low","churn":"medium","decay_flags":[],
    "review_note":"面积小、职责单一，但为安全扫描维护了 renderer AI 逻辑的第二份简化实现。",
    "concerns":[{"severity":"high",
      "finding":"ai-client.ts 的 endpoint 拼接与鉴权是对 renderer/services/ai.ts 的简化复制（文件自述 mirrors），两端行为易漂移",
      "suggestion":"抽出共享的 endpoint/headers 构造到 packages/shared，renderer 与主进程复用同一实现"}]}},

 {"id":"ipc-bridge","name":"IPC 契约桥",
  "layer":"application","responsibility":"preload 桥接与主进程 IPC handler 注册",
  "files":["apps/desktop/src/main/ipc/**","apps/desktop/src/preload/**"],
  "key_entries":[
    {"file":"apps/desktop/src/preload/index.ts","symbol":"api","kind":"config"},
    {"file":"apps/desktop/src/main/ipc/index.ts","symbol":"registerAllIPC","kind":"function"},
    {"file":"apps/desktop/src/main/ipc/skill/version-handlers.ts","symbol":"registerSkillVersionHandlers","kind":"function"}],
  "dependencies":["shared-foundation","data-persistence","skill-system","backup-recovery"],
  "health":{"score":74,"coupling":"medium","complexity":"medium","churn":"medium","decay_flags":[],
    "review_note":"契约集中在 IPC_CHANNELS，方向清晰；但 handler 越界直连数据库，见 concerns。",
    "concerns":[{"severity":"high",
      "finding":"backup.ipc/folder.ipc 等 handler 直接 import ../database 与 ../services（17 处），IPC 层承担了编排职责",
      "suggestion":"handler 只做参数校验与转发，把编排逻辑上移到独立应用服务后再调用"}]}},

 {"id":"web-api","name":"自托管 Web 服务端",
  "layer":"application","responsibility":"Hono 服务端路由、鉴权中间件与 Web 域服务",
  "files":["apps/web/src/index.ts","apps/web/src/app.ts","apps/web/src/config.ts","apps/web/src/database.ts","apps/web/src/runtime-paths.ts","apps/web/src/routes/**","apps/web/src/middleware/**","apps/web/src/services/**","apps/web/src/utils/**"],
  "key_entries":[
    {"file":"apps/web/src/app.ts","symbol":"createApp","kind":"function"},
    {"file":"apps/web/src/services/prompt.service.ts","symbol":"PromptService","kind":"class"},
    {"file":"apps/web/src/middleware/auth.ts","symbol":"auth","kind":"function"}],
  "dependencies":["data-persistence","shared-foundation"],
  "health":{"score":78,"coupling":"low","complexity":"medium","churn":"high","decay_flags":[],
    "review_note":"中间件链（logger/securityHeaders/auth/errorHandler）与路由分组干净；与桌面端存在领域逻辑重复。",
    "concerns":[{"severity":"high",
      "finding":"prompt/skill workspace 引导与备份在 web services 与 desktop main 各有独立实现，字段与行为易分叉",
      "suggestion":"把 workspace 引导与备份格式抽到 packages 共享，web/desktop 仅保留 IO 与鉴权适配"}]}},

 {"id":"desktop-state","name":"渲染进程状态与服务",
  "layer":"application","responsibility":"Zustand 状态与前端业务服务（同步、备份、技能商店、AI）",
  "files":["apps/desktop/src/renderer/stores/**","apps/desktop/src/renderer/services/**","apps/desktop/src/renderer/hooks/**"],
  "key_entries":[
    {"file":"apps/desktop/src/renderer/stores/prompt.store.ts","symbol":"usePromptStore","kind":"function"},
    {"file":"apps/desktop/src/renderer/services/webdav.ts","symbol":"autoSync","kind":"function"},
    {"file":"apps/desktop/src/renderer/services/ai.ts","symbol":"ChatCompletionRequest","kind":"interface"}],
  "dependencies":["shared-foundation","ipc-bridge","desktop-ui"],
  "health":{"score":66,"coupling":"high","complexity":"high","churn":"high","decay_flags":["circular_dep"],
    "review_note":"状态与服务集中，改动频繁；与界面层的环依赖是当前最紧的架构约束。",
    "concerns":[
      {"severity":"critical",
       "finding":"settings.store 通过 ../i18n 反向依赖界面层，与 desktop-ui 互相引用形成环（该边已标 direction_violation）",
       "suggestion":"把 i18n 实例初始化下沉为独立基础模块，store 不再直接依赖 UI 目录"},
      {"severity":"high",
       "finding":"20 个前端服务与 5 个 store 承载 AI、WebDAV、自托管同步、技能商店、备份等互不相关流程",
       "suggestion":"按域拆分 services 子目录（sync/backup/skill-store/ai），store 仅保留状态与动作"}]}},

 {"id":"desktop-ui","name":"桌面界面",
  "layer":"interface","responsibility":"React 渲染层页面、组件库、主题与多语言资源",
  "files":["apps/desktop/src/renderer/App.tsx","apps/desktop/src/renderer/main.tsx","apps/desktop/src/renderer/components/**","apps/desktop/src/renderer/i18n/**","apps/desktop/src/renderer/styles/**","apps/desktop/src/renderer/assets/**","apps/desktop/src/renderer/runtime.ts","apps/desktop/src/renderer/utils/**"],
  "key_entries":[
    {"file":"apps/desktop/src/renderer/App.tsx","symbol":"App","kind":"function"},
    {"file":"apps/desktop/src/renderer/components/layout/index.ts","symbol":"Sidebar","kind":"function"},
    {"file":"apps/desktop/src/renderer/i18n/index.ts","symbol":"i18n","kind":"config"}],
  "dependencies":["desktop-state","shared-foundation","ipc-bridge"],
  "health":{"score":68,"coupling":"medium","complexity":"high","churn":"high","decay_flags":["circular_dep"],
    "review_note":"组件规模大且被状态层反向依赖；应约束组件只消费 store/service，不碰底层。",
    "concerns":[{"severity":"high",
      "finding":"7 个组件分组共 120+ 文件，其中 16 处组件直接调用 window.api 绕过 store/service",
      "suggestion":"把组件内直接的 window.api 调用收敛到 services/store，组件仅消费状态与动作"}]}},

 {"id":"web-client","name":"Web 客户端",
  "layer":"interface","responsibility":"React 客户端页面、路由与 API 客户端封装",
  "files":["apps/web/src/client/**"],
  "key_entries":[
    {"file":"apps/web/src/client/App.tsx","symbol":"App","kind":"function"},
    {"file":"apps/web/src/client/api/endpoints.ts","symbol":"fetchFolders","kind":"function"},
    {"file":"apps/web/src/client/api/auth-session.ts","symbol":"fetchWithAuthRetry","kind":"function"}],
  "dependencies":["web-api","shared-foundation"],
  "health":{"score":82,"coupling":"low","complexity":"medium","churn":"high","decay_flags":[],
    "review_note":"通过统一 api/ 客户端访问服务端、类型来自 shared，边界干净；保持该结构即可。"}},

 {"id":"website","name":"官网",
  "layer":"interface","responsibility":"Astro 产品官网与多语言文档页面",
  "files":["website/src/**"],
  "key_entries":[
    {"file":"website/src/pages/index.astro","symbol":"/","kind":"route"},
    {"file":"website/src/generated/release.ts","symbol":"RELEASE_VERSION","kind":"config"}],
  "dependencies":[],
  "health":{"score":84,"coupling":"low","complexity":"low","churn":"low","decay_flags":[],
    "review_note":"独立 Astro 站点、不在 pnpm workspace 内，与其他产品代码无依赖；发布元数据由脚本生成，保持即可。"}},

 {"id":"desktop-runtime","name":"桌面运行时装配",
  "layer":"app-entry","responsibility":"Electron 主进程装配与菜单/更新/CLI 入口",
  "files":["apps/desktop/src/main/index.ts","apps/desktop/src/main/menu.ts","apps/desktop/src/main/shortcuts.ts","apps/desktop/src/main/security.ts","apps/desktop/src/main/updater.ts","apps/desktop/src/main/webdav.ts","apps/desktop/src/main/startup-log.ts","apps/desktop/src/main/desktop-cli.ts","apps/desktop/src/main/testing/**","apps/desktop/src/cli/**"],
  "key_entries":[
    {"file":"apps/desktop/src/main/updater.ts","symbol":"initUpdater","kind":"function"},
    {"file":"apps/desktop/src/main/menu.ts","symbol":"createMenu","kind":"function"},
    {"file":"apps/desktop/src/cli/run.ts","symbol":"runCli","kind":"cli"}],
  "dependencies":["data-persistence","backup-recovery","shared-foundation","ipc-bridge","skill-system"],
  "health":{"score":72,"coupling":"medium","complexity":"high","churn":"high","decay_flags":[],
    "review_note":"作为组合根向下依赖各层合理；但 index.ts 装配步骤过多，降低可测性与可读性。",
    "concerns":[{"severity":"high",
      "finding":"main/index.ts 同时承担窗口创建、IPC 注册、数据路径迁移、升级备份启动与 CLI 安装等多重装配职责",
      "suggestion":"把迁移/备份启动等步骤抽成独立 bootstrap 模块，index 只做顺序编排与生命周期管理"}]}},
]

for _m in modules:
    if 'concerns' in _m:
        _m['health']['concerns']=_m.pop('concerns')

merge_order=["shared-foundation","data-persistence","skill-system","backup-recovery","ai-integration","ipc-bridge","web-api","desktop-state","desktop-ui","web-client","website","desktop-runtime"]

edges_by_mod={
 "shared-foundation":[],
 "data-persistence":[{"from":"data-persistence","to":"shared-foundation","type":"import","label":"@prompthub/shared/types","strength":"weak","direction_violation":False}],
 "skill-system":[
   {"from":"skill-system","to":"shared-foundation","type":"import","label":"@prompthub/shared","strength":"normal","direction_violation":False},
   {"from":"skill-system","to":"data-persistence","type":"db","label":"SkillDB","strength":"normal","direction_violation":False},
   {"from":"skill-system","to":"ai-integration","type":"call","label":"chatCompletion","strength":"weak","direction_violation":False}],
 "backup-recovery":[
   {"from":"backup-recovery","to":"data-persistence","type":"db","label":"DatabaseAdapter","strength":"normal","direction_violation":False},
   {"from":"backup-recovery","to":"shared-foundation","type":"import","label":"@prompthub/shared/types","strength":"weak","direction_violation":False}],
 "ai-integration":[{"from":"ai-integration","to":"shared-foundation","type":"import","label":"SafetyScanAIConfig","strength":"weak","direction_violation":False}],
 "ipc-bridge":[
   {"from":"ipc-bridge","to":"shared-foundation","type":"import","label":"IPC_CHANNELS","strength":"strong","direction_violation":False},
   {"from":"ipc-bridge","to":"data-persistence","type":"db","label":"PromptDB/FolderDB/SkillDB","strength":"strong","direction_violation":False},
   {"from":"ipc-bridge","to":"skill-system","type":"call","label":"SkillInstaller","strength":"normal","direction_violation":False},
   {"from":"ipc-bridge","to":"backup-recovery","type":"call","label":"upgrade-backup-restore","strength":"weak","direction_violation":False}],
 "web-api":[
   {"from":"web-api","to":"data-persistence","type":"db","label":"@prompthub/db","strength":"strong","direction_violation":False},
   {"from":"web-api","to":"shared-foundation","type":"import","label":"@prompthub/shared","strength":"normal","direction_violation":False}],
 "desktop-state":[
   {"from":"desktop-state","to":"shared-foundation","type":"import","label":"@prompthub/shared/types","strength":"strong","direction_violation":False},
   {"from":"desktop-state","to":"ipc-bridge","type":"api","label":"window.api","strength":"normal","direction_violation":False},
   {"from":"desktop-state","to":"desktop-ui","type":"import","label":"../i18n","strength":"weak","direction_violation":True}],
 "desktop-ui":[
   {"from":"desktop-ui","to":"desktop-state","type":"import","label":"stores/services","strength":"strong","direction_violation":False},
   {"from":"desktop-ui","to":"shared-foundation","type":"import","label":"@prompthub/shared/types","strength":"strong","direction_violation":False},
   {"from":"desktop-ui","to":"ipc-bridge","type":"api","label":"window.api","strength":"normal","direction_violation":False}],
 "web-client":[
   {"from":"web-client","to":"web-api","type":"api","label":"/api/*","strength":"strong","direction_violation":False},
   {"from":"web-client","to":"shared-foundation","type":"import","label":"@prompthub/shared","strength":"weak","direction_violation":False}],
 "website":[],
 "desktop-runtime":[
   {"from":"desktop-runtime","to":"data-persistence","type":"import","label":"database","strength":"strong","direction_violation":False},
   {"from":"desktop-runtime","to":"backup-recovery","type":"call","label":"upgrade-backup-startup","strength":"normal","direction_violation":False},
   {"from":"desktop-runtime","to":"shared-foundation","type":"import","label":"@prompthub/shared/types","strength":"normal","direction_violation":False},
   {"from":"desktop-runtime","to":"ipc-bridge","type":"call","label":"registerAllIPC","strength":"weak","direction_violation":False},
   {"from":"desktop-runtime","to":"skill-system","type":"call","label":"SkillInstaller","strength":"weak","direction_violation":False}],
}

arch_health={"score":71,"coupling":"high","complexity":"high","churn":"high",
 "decay_flags":["circular_dep","responsibility_overlap","layer_violation"],
 "review_note":"分层总体清晰、依赖大体向下，但存在三处结构性隐患：界面层与状态层互引成环、IPC 边界层直连数据库与领域服务、桌面端与 Web 端重复实现 workspace/备份/AI 逻辑；模块单看多数健康，架构分被这些跨模块形态拉低。",
 "concerns":[
  {"severity":"critical",
   "finding":"desktop-ui 与 desktop-state 通过 i18n 互引形成循环依赖，是当前唯一的跨模块环",
   "suggestion":"把 i18n 初始化与资源下沉为独立基础模块，消除 state→ui 的反向边"},
  {"severity":"high",
   "finding":"桌面端 main/services 与 web services 重复实现 prompt/skill workspace、备份与 AI endpoint 构造",
   "suggestion":"将这些跨端同构逻辑抽到 packages/shared 或新共享包，两端只保留 IO/鉴权适配"},
  {"severity":"high",
   "finding":"ipc-bridge 直接访问数据库与领域服务（17 处 ../database import），边界层职责过重",
   "suggestion":"IPC handler 仅做校验与转发，编排逻辑上移到应用服务层后再调用"}]}

meta={"repo":"PromptHub","generated_at":now,
 "generator":"easyvibe-architect/claude-sonnet-4-6",
 "description":"PromptHub 单体仓库：Electron 桌面端 + 自部署 Web 端的本地优先 AI Prompt/Skill 管理平台，共享 SQLite(WASM) 数据层。",
 "languages":["TypeScript","Astro"],"loc":88576,"map_freshness":"fresh"}

emit_order={"layers":layers,"modules":merge_order}  # growth order: bottom-up (foundation -> app-entry)

atomic(os.path.join(MAP,'meta.json'),meta)
atomic(os.path.join(MAP,'emit_order.json'),emit_order)

# clustering done
progress('clustering', percent=35, modules_total=len(modules), modules_done=0, current_module=None)

# ---------- module-analysis: emit parts one by one ----------
bylayer={}
for m in modules: bylayer.setdefault(m['layer'],[]).append(m)
done=0; n=len(modules)
for mid in merge_order:
    m=[x for x in modules if x['id']==mid][0]
    progress('module-analysis', current_module=mid,
             percent=round(35+ (70-35)*done/n))
    cli('emit-module', stdin=json.dumps(m,ensure_ascii=False))
    done+=1
    progress('module-analysis', modules_done=done, current_module=mid,
             percent=round(35+(70-35)*done/n), modules_total=n)

# ---------- edging ----------
progress('edging', percent=80, current_module=None)
_eid=0
for _mid in merge_order:
    for _e in edges_by_mod[_mid]:
        _eid+=1
        _e['id']='e%d'%_eid
for mid in merge_order:
    cli('emit-edges', stdin=json.dumps({"module":mid,"edges":edges_by_mod[mid]},ensure_ascii=False))
atomic(os.path.join(MAP,'parts','_arch_health.json'),arch_health)

# ---------- health done ----------
progress('health', percent=90, current_module=None)

# ---------- assembling: append log ----------
progress('assembling', percent=95, current_module=None)
print(cli('append-log'))
print(cli('finalize'))
