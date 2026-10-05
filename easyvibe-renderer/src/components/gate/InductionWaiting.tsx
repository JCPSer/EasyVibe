import { useEffect, useState } from 'react'
import { Loader2 } from 'lucide-react'
import { ONBOARDING_COPY } from '@/lib/onboardingCopy'
import { loadTaskIdea, saveTaskIdea, prefersReducedMotion } from '@/lib/onboarding'

// 首归纳等待页：轮询 progress.json 展示真实阶段与百分比（"正在边推导 80%"而非干转圈）
export function InductionWaiting({ repo }: { repo: string }) {  const [prog, setProg] = useState<{ phase: string; percent: number; modulesDone: number; modulesTotal: number } | null>(null)
  // 2026-10-04 新手引导：等待期是引导黄金时间（调研 B 节）——三层递进：
  // 真实进度叙事（原有）+ 概念卡片轮播 + 任务想法预填（归纳完成后带入任务对话）
  const [cardIdx, setCardIdx] = useState(0)
  const [idea, setIdea] = useState(() => loadTaskIdea() ?? '')
  const [ideaSaved, setIdeaSaved] = useState(false)
  const reduced = prefersReducedMotion()
  useEffect(() => {
    if (reduced) return // 尊重减弱动效：不自动轮播，手动翻页即可
    const t = window.setInterval(() => setCardIdx((i) => (i + 1) % ONBOARDING_COPY.concepts.length), 20000)
    return () => window.clearInterval(t)
  }, [reduced])
  useEffect(() => {
    let stale = false
    const tick = () => {
      fetch(`/api/repos/${repo}/progress`)
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
        .then((d: { data: { phase: string; percent: number; modules_done: number; modules_total: number } | null }) => {
          if (stale || !d.data) return
          setProg({ phase: d.data.phase, percent: d.data.percent, modulesDone: d.data.modules_done, modulesTotal: d.data.modules_total })
        })
        .catch(() => {})
    }
    tick()
    const t = window.setInterval(tick, 2000)
    return () => {
      stale = true
      window.clearInterval(t)
    }
  }, [repo])

  const PHASE_LABEL: Record<string, string> = {
    init: '准备中',
    layering: '分层分析',
    module_scan: '模块扫描',
    emit: '模块归纳',
    edging: '依赖边推导',
    consistency: '一致性校验',
    finalize: '收尾写盘',
    done: '完成',
  }
  const concept = ONBOARDING_COPY.concepts[cardIdx]
  return (
    <div className="flex h-screen flex-col items-center justify-center gap-4 px-6 text-[13px] text-slate-500 dark:text-slate-400">
      {/* 第 1 层：真实进度叙事（永远不让等待页只有 spinner） */}
      <Loader2 size={18} className="animate-spin text-blue-500" />
      <span className="font-semibold text-slate-700 dark:text-slate-200">
        正在归纳代码地图{prog ? `：${PHASE_LABEL[prog.phase] ?? prog.phase} ${prog.percent}%` : '…'}
      </span>
      {prog && (
        <div className="h-1.5 w-64 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800" role="progressbar" aria-valuenow={prog.percent} aria-valuemin={0} aria-valuemax={100}>
          <div className="h-full rounded-full bg-blue-500 transition-[width] duration-500" style={{ width: `${prog.percent}%` }} />
        </div>
      )}
      <span className="max-w-[420px] text-center text-[11px] leading-4 text-slate-400 dark:text-slate-500">
        {prog && prog.modulesTotal > 0
          ? `已归纳 ${prog.modulesDone}/${prog.modulesTotal} 个模块`
          : '后台 agent 执行中（通常数分钟，取决于仓库规模）'}
        ，完成后地图会自动出现
      </span>

      {/* 第 2 层：概念卡片轮播（每张 ~20s 自动翻，可手动点；key 驱动翻页淡入——评审 G7） */}
      <div className="w-full max-w-md rounded-xl border border-slate-100 dark:border-slate-800 bg-white/90 dark:bg-slate-900/90 px-4 py-3 shadow-sm">
        <div key={cardIdx} className="anim-fade-in-fast">
        <div className="flex items-center justify-between">
          <p className="text-[12px] font-bold text-slate-700 dark:text-slate-200">{concept.title}</p>
          <div className="flex gap-1">
            {ONBOARDING_COPY.concepts.map((c, i) => (
              <button
                key={c.id}
                onClick={() => setCardIdx(i)}
                aria-label={`第 ${i + 1} 张：${c.title}`}
                className={`h-1.5 rounded-full transition-all ${i === cardIdx ? 'w-4 bg-blue-500' : 'w-1.5 bg-slate-200 hover:bg-slate-300'}`}
              />
            ))}
          </div>
        </div>
        <p className="mt-1.5 text-[11.5px] leading-5 text-slate-500 dark:text-slate-400">{concept.body}</p>
        </div>
      </div>

      {/* 第 3 层：提前参与——任务想法预填（归纳完成后带入任务对话） */}
      <div className="w-full max-w-md">
        <p className="text-[11px] font-semibold text-slate-500 dark:text-slate-400">{ONBOARDING_COPY.waiting.ideaTitle}</p>
        <div className="mt-1 flex gap-1.5">
          <input
            value={idea}
            onChange={(e) => { setIdea(e.target.value); setIdeaSaved(false) }}
            placeholder={ONBOARDING_COPY.waiting.ideaPlaceholder}
            className="min-w-0 flex-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-[12px] outline-none transition-colors focus:border-blue-300 focus:ring-2 focus:ring-blue-100"
          />
          <button
            onClick={() => { saveTaskIdea(idea.trim()); setIdeaSaved(true) }}
            disabled={!idea.trim()}
            className="shrink-0 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-[11px] font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600 disabled:opacity-40"
          >
            {ONBOARDING_COPY.waiting.ideaButton}
          </button>
        </div>
        {ideaSaved && <p className="mt-1 text-[10.5px] text-emerald-600">{ONBOARDING_COPY.waiting.ideaSaved}</p>}
      </div>
    </div>
  )
}
