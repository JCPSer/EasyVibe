import { useState } from 'react'
import { CheckCircle2, ChevronDown, ChevronUp, X } from 'lucide-react'
import type { CheckKey, OnboardingState } from '@/lib/onboarding'
import { CHECK_KEYS, checkDoneCount } from '@/lib/onboarding'
import { ONBOARDING_COPY } from '@/shared/logic/onboardingCopy'

/** 事件驱动上手指引（调研定稿方案③：Linear 式 checklist）。
 *  完成态由 App 层的真实事件写入（markCheck），本组件纯展示 + 折叠/关闭。
 *  goActions：每项的"带我去"跳转（分工显式化，降低每步执行成本） */
export function OnboardingChecklist({
  state,
  onGo,
  onDismiss,
}: {
  state: OnboardingState
  onGo: (key: CheckKey) => void
  onDismiss: () => void
}) {
  const [collapsed, setCollapsed] = useState(false)
  const done = checkDoneCount(state)
  const total = CHECK_KEYS.length
  const { title, items } = ONBOARDING_COPY.checklist

  return (
    // 宽度收敛在右栏（w-64）以内：2026-10-04 实弹——w-72 溢出压住任务对话输入框的发送按钮
    <div className="pointer-events-auto w-60 overflow-hidden rounded-xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 shadow-xl backdrop-blur">
      <button
        onClick={() => setCollapsed((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left"
        aria-expanded={!collapsed}
      >
        <span className="text-[12px] font-bold text-slate-700 dark:text-slate-200">{title}</span>
        <span className="tnum rounded-full bg-slate-100 dark:bg-slate-800 px-1.5 text-micro font-bold text-slate-500 dark:text-slate-400">
          {done}/{total}
        </span>
        <span className="ml-auto flex items-center gap-0.5">
          {collapsed ? <ChevronUp size={13} className="text-slate-300 dark:text-slate-600" /> : <ChevronDown size={13} className="text-slate-300 dark:text-slate-600" />}
        </span>
      </button>
      {/* 进度条：完成度可视（Linear 式"还差几步"张力） */}
      <div className="h-0.5 bg-slate-100 dark:bg-slate-800">
        <div className="h-full bg-blue-500 transition-[width] duration-500" style={{ width: `${(done / total) * 100}%` }} />
      </div>
      {!collapsed && (
        <ul className="space-y-0.5 p-1.5">
          {CHECK_KEYS.map((k) => {
            const it = items[k]
            const isDone = state.checklist[k] === 'done'
            return (
              <li key={k} className="group flex items-start gap-2 rounded-lg px-2 py-1.5 hover:bg-slate-50 dark:hover:bg-slate-800/70">
                {isDone ? (
                  <CheckCircle2 size={14} className="mt-0.5 shrink-0 text-emerald-500" />
                ) : (
                  <span className="mt-0.5 h-3.5 w-3.5 shrink-0 rounded-full border-2 border-slate-300 dark:border-slate-600" />
                )}
                <div className="min-w-0 flex-1">
                  <p className={`text-[12px] font-semibold leading-4 ${isDone ? 'text-slate-400 dark:text-slate-500 line-through' : 'text-slate-700 dark:text-slate-200'}`}>
                    {it.label}
                  </p>
                  {!isDone && <p className="mt-0.5 text-[10.5px] leading-4 text-slate-400 dark:text-slate-500">{it.hint}</p>}
                </div>
                {!isDone && (
                  <button
                    onClick={() => onGo(k)}
                    className="shrink-0 rounded-md border border-slate-200 dark:border-slate-700 px-1.5 py-0.5 text-micro font-semibold text-slate-400 dark:text-slate-500 opacity-0 transition-opacity hover:border-blue-300 hover:text-blue-600 group-hover:opacity-100"
                  >
                    带我去
                  </button>
                )}
              </li>
            )
          })}
        </ul>
      )}
      <div className="flex justify-end border-t border-slate-50 px-2 py-1">
        <button onClick={onDismiss} className="flex items-center gap-0.5 text-micro text-slate-300 dark:text-slate-600 hover:text-slate-500" title="关闭（可从顶栏 ? 重新打开）">
          <X size={10} /> 收起不再提示
        </button>
      </div>
    </div>
  )
}
