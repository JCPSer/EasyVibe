import { useState } from 'react'
import { Plus, WifiOff, X } from 'lucide-react'

/**
 * 顶栏中区：仓库切换器（在线/离线两态 + 已挂载列表 + 添加/移除双选项确认）。
 * 从 App 抽出，JSX 逐字保留；弹层开合/移除确认状态下沉为组件本地状态。
 */
export function TopCenter({
  backendOnline,
  backendRepo,
  repos,
  onSwitchRepo,
  onAddRepo,
  onRemoveRepo,
}: {
  backendOnline: boolean | null
  backendRepo: string | null
  repos: { id: string; name: string }[]
  onSwitchRepo: (id: string) => void
  onAddRepo: () => void
  /** 返回是否移除成功；成功才收起确认卡（与拆分前 removeRepo 的 setConfirmRemove 时机一致） */
  onRemoveRepo: (id: string, wipe: boolean) => Promise<boolean>
}) {
  const [repoPanelOpen, setRepoPanelOpen] = useState(false)
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null) // 重审 P1：注销双选项确认（保留/清除数据）

  return (
    <div className="relative">
        <button
          onClick={() => setRepoPanelOpen((v) => !v)}
          className={
            backendOnline === false
              ? "flex items-center gap-1.5 rounded-lg border border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 px-2.5 py-1 text-[12px] font-semibold text-amber-700 hover:border-amber-400"
              : "flex items-center gap-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1 text-[12px] font-semibold text-slate-700 dark:text-slate-200 hover:border-blue-300"
          }
          title={backendOnline === false ? "后端不在线：当前为演示数据，点击看详情" : "切换/管理仓库"}
        >
          {backendOnline === false ? (
            <>
              <WifiOff size={11} />
              演示数据 · 后端离线
              <span className="text-amber-400">▾</span>
            </>
          ) : (
            <>
              <span className={`h-2 w-2 rounded-full ${backendOnline === null ? 'animate-pulse bg-slate-300' : 'bg-emerald-500'}`} />
              {backendOnline === null ? '连接后端中…' : backendRepo ?? '未选择仓库'}
              <span className="text-slate-300 dark:text-slate-600">▾</span>
            </>
          )}
        </button>
        {repoPanelOpen && (
          <>
            {/* 点外部关闭（真人测试 Bug#1：此前无 outside-click 处理，跨页面悬浮） */}
            <div className="fixed inset-0 z-30" onClick={() => setRepoPanelOpen(false)} />
            <div className="glass absolute left-0 top-full z-40 mt-1.5 w-80 rounded-xl border border-slate-200 dark:border-slate-700 p-2 shadow-xl">
            {backendOnline === false ? (
              /* 重审 P2：离线态的真相面板——不装成"尚未挂载"（那是在线零仓库的状态） */
              <div className="space-y-1.5 px-1.5 py-1.5">
                <p className="flex items-center gap-1 text-[12px] font-bold text-amber-700">
                  <WifiOff size={12} /> 后端不在线
                </p>
                <p className="text-[11px] leading-4 text-slate-500 dark:text-slate-400">
                  当前画布是内置演示数据（hover-client）。归纳 / 巡检 / 任务 / 仓库管理都需要本地后端在线。
                </p>
                <p className="text-[10px] leading-4 text-slate-400 dark:text-slate-500">
                  应用启动后后端在冷加载？每 5 秒自动重连，恢复后此面板自动可用。
                </p>
              </div>
            ) : (
              <>
            <p className="px-1.5 pb-1.5 text-micro font-semibold text-slate-400 dark:text-slate-500">已挂载仓库</p>
            <div className="max-h-52 space-y-0.5 overflow-y-auto">
              {repos.map((r) => (
                <div key={r.id} className="rounded-lg px-1.5 py-1 hover:bg-slate-50 dark:hover:bg-slate-800/70">
                  <div className="flex items-center gap-1.5">
                    <button
                      className={`min-w-0 flex-1 truncate text-left text-[12px] ${r.id === backendRepo ? 'font-bold text-blue-700' : 'text-slate-700 dark:text-slate-200'}`}
                      onClick={() => {
                        onSwitchRepo(r.id)
                        setRepoPanelOpen(false)
                      }}
                      title={r.name}
                    >
                      {r.name}
                    </button>
                    {confirmRemove === r.id ? (
                      <button
                        onClick={() => setConfirmRemove(null)}
                        className="shrink-0 rounded p-0.5 text-slate-400 dark:text-slate-500 hover:text-slate-600"
                        title="取消"
                      >
                        <X size={12} />
                      </button>
                    ) : (
                      <button
                        onClick={() => setConfirmRemove(confirmRemove === r.id ? null : r.id)}
                        className="shrink-0 rounded p-0.5 text-slate-300 dark:text-slate-600 hover:bg-red-50 dark:hover:bg-red-950/40 hover:text-red-500"
                        title="移除仓库…"
                      >
                        <X size={12} />
                      </button>
                    )}
                  </div>
                  {/* 重审 P1：注销确认双选项——数据保留 or 连同清除（后端 wipe_repo），
                      不再是无差别的 window.confirm（用户不知道数据去了哪） */}
                  {confirmRemove === r.id && (
                    <div className="mt-1 space-y-1 rounded-lg border border-red-100 bg-red-50/50 p-1.5">
                      <p className="text-[10px] leading-4 text-slate-500 dark:text-slate-400">
                        正在运行的任务/归纳会被终止。本地数据怎么处理？
                      </p>
                      <div className="flex gap-1">
                        <button
                          onClick={() => { void onRemoveRepo(r.id, false).then((ok) => { if (ok) setConfirmRemove(null) }) }}
                          className="flex-1 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-1 text-[10px] font-bold text-slate-600 dark:text-slate-300 hover:border-blue-300 hover:text-blue-600"
                          title="任务/会话/巡检历史留在本地库，重新添加仓库后可见"
                        >
                          移除，保留数据
                        </button>
                        <button
                          onClick={() => { void onRemoveRepo(r.id, true).then((ok) => { if (ok) setConfirmRemove(null) }) }}
                          className="flex-1 rounded-md bg-red-500 px-2 py-1 text-[10px] font-bold text-white hover:bg-red-600"
                          title="抹掉该仓库的任务/会话/审批/巡检历史/事件/仓库级设置（不可恢复）"
                        >
                          移除并清除数据
                        </button>
                      </div>
                    </div>
                  )}
                </div>
              ))}
              {repos.length === 0 && <p className="px-1.5 py-2 text-[11px] text-slate-400 dark:text-slate-500">尚未挂载任何仓库</p>}
            </div>
            <button
              onClick={() => {
                setRepoPanelOpen(false)
                onAddRepo()
              }}
              className="mt-1.5 flex w-full items-center justify-center gap-1 rounded-lg bg-blue-600 px-2 py-1.5 text-[11px] font-semibold text-white hover:bg-blue-700"
            >
              <Plus size={11} />
              打开本地仓库…
            </button>
              </>
            )}
          </div>
          </>
        )}
    </div>
  )
}
