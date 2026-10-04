import { useEffect, useState } from 'react'
import { FolderOpen, Play, Sparkles, X } from 'lucide-react'
import { ONBOARDING_COPY } from '@/lib/onboardingCopy'

/** 首启欢迎工作台（调研定稿方案①：AionUi 式"首屏只做一件事"）。
 *  fullScreen 覆盖在应用之上；首启（hasRepo=false）主 CTA 是"添加仓库"，
 *  帮助菜单重看（hasRepo=true）主 CTA 变为"开始探索"。
 *  出口齐全（2026-10-04 实弹补）：Esc / 点遮罩 / X / 问号再点（壳层 toggle）都可关闭。 */
export function WelcomePage({
  hasRepo,
  onAddRepo,
  onClose,
}: {
  hasRepo: boolean
  onAddRepo: () => Promise<boolean>
  onClose: () => void
}) {
  const [adding, setAdding] = useState(false)
  const { welcome, concepts } = ONBOARDING_COPY

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const primary = async () => {
    if (hasRepo) {
      onClose()
      return
    }
    setAdding(true)
    try {
      const ok = await onAddRepo()
      if (ok) onClose() // 成功后由 MapGate 接管（归纳等待页）
    } finally {
      setAdding(false)
    }
  }

  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/45 p-6 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label="欢迎使用 EasyVibe"
      onClick={onClose}
    >
      <div className="anim-scale-in glass w-full max-w-2xl rounded-2xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 p-8 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-start justify-between">
          <p className="flex items-center gap-1.5 text-micro font-bold uppercase tracking-widest text-blue-500">
            <Sparkles size={12} /> {welcome.kicker}
          </p>
          <button onClick={onClose} className="rounded p-1 text-slate-300 dark:text-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-500" aria-label="关闭引导">
            <X size={16} />
          </button>
        </div>
        <h1 className="mt-2 text-[22px] font-bold leading-8 text-slate-800 dark:text-slate-100">{welcome.title}</h1>
        <p className="mt-1.5 max-w-xl text-[13px] leading-6 text-slate-500 dark:text-slate-400">{welcome.subtitle}</p>

        {/* 概念卡片：建立心智模型（每张 20-30 字正文，给长文本语言留冗余） */}
        <div className="mt-5 grid grid-cols-1 gap-2.5 sm:grid-cols-2">
          {concepts.map((c) => (
            <div key={c.id} className="rounded-xl border border-slate-100 dark:border-slate-800 bg-white dark:bg-slate-900 px-3.5 py-3">
              <p className="text-[12.5px] font-bold text-slate-700 dark:text-slate-200">{c.title}</p>
              <p className="mt-1 text-[11.5px] leading-5 text-slate-500 dark:text-slate-400">{c.body}</p>
            </div>
          ))}
        </div>

        <div className="mt-6 flex flex-wrap items-center gap-3">
          <button
            onClick={() => void primary()}
            disabled={adding}
            className="flex items-center gap-1.5 rounded-xl bg-blue-600 px-5 py-2.5 text-[13px] font-bold text-white shadow-sm hover:bg-blue-700 disabled:opacity-50"
          >
            {hasRepo ? <Play size={14} /> : <FolderOpen size={14} />}
            {adding ? '正在打开选择器…' : hasRepo ? '开始探索' : welcome.ctaPrimary}
          </button>
          {!hasRepo && (
            <button
              onClick={onClose}
              className="rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-2.5 text-[12.5px] font-semibold text-slate-500 dark:text-slate-400 hover:border-slate-300 hover:text-slate-700"
              title={welcome.ctaSecondaryHint}
            >
              {welcome.ctaSecondary}
            </button>
          )}
          <span className="ml-auto text-[10.5px] text-slate-300 dark:text-slate-600">{welcome.reopenNote}</span>
        </div>
      </div>
    </div>
  )
}
