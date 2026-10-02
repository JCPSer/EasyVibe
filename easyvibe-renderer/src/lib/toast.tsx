import { useEffect, useState } from 'react'
import { AlertCircle, Info, X } from 'lucide-react'

// R3 清债：统一 toast 反馈——替代裸 alert 兜底（§0 标准"无裸 alert"）
// D5-2 扩展：可带操作按钮（如"重启更新"）——带操作的 toast 常驻 12s 等用户决策
// 盲测 P1 扩展：sticky（高危预警类）——不自动消失，点了操作按钮才走（错过即失去入口是监督大忌）
// 堆叠修复：sticky 预警按 key 聚合（同任务多次越界只留最新一条）+ 手动关闭——
// 此前同任务 3 条 sticky 预警并排堆在画布上、永不消失、无关闭口，用户只能重启
type Toast = {
  id: number
  msg: string
  kind: 'info' | 'error'
  action?: { label: string; onClick: () => void }
  sticky?: boolean
  /** 聚合键：同 key 新 toast 替换旧 toast（预警刷屏的消解点） */
  key?: string
}

let listeners: ((t: Toast) => void)[] = []
let seq = 0

export function toast(
  msg: string,
  kind: 'info' | 'error' = 'info',
  action?: { label: string; onClick: () => void },
  sticky = false,
  key?: string,
) {
  const t: Toast = { id: ++seq, msg, kind, action, sticky, key }
  listeners.forEach((l) => l(t))
}

export function ToastHost() {
  const [items, setItems] = useState<Toast[]>([])
  useEffect(() => {
    const l = (t: Toast) => {
      // 同 key 替换（保留原位置观感：先删旧再加新，尾部追加）；无 key 走原滑动窗口
      setItems((prev) => {
        const next = t.key ? prev.filter((x) => x.key !== t.key) : prev
        return [...next.slice(-2), t]
      })
      if (!t.sticky) setTimeout(() => setItems((prev) => prev.filter((x) => x.id !== t.id)), t.action ? 12000 : 3200)
    }
    listeners.push(l)
    return () => {
      listeners = listeners.filter((x) => x !== l)
    }
  }, [])
  const dismiss = (id: number) => setItems((prev) => prev.filter((x) => x.id !== id))
  if (items.length === 0) return null
  return (
    <div className="pointer-events-none fixed bottom-5 left-1/2 z-[70] flex -translate-x-1/2 flex-col items-center gap-1.5">
      {items.map((t) => (
        <div
          key={t.id}
          className={`flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11px] font-semibold text-white shadow-lg ${
            t.kind === 'error' ? 'bg-red-500' : 'bg-slate-800'
          }`}
        >
          {t.kind === 'error' ? <AlertCircle size={11} /> : <Info size={11} />}
          {t.msg}
          {t.action && (
            <button
              onClick={() => {
                dismiss(t.id)
                t.action!.onClick()
              }}
              className="pointer-events-auto ml-1 rounded-full bg-white/20 px-2 py-0.5 text-micro hover:bg-white/30"
            >
              {t.action.label}
            </button>
          )}
          {t.sticky && (
            <button
              onClick={() => dismiss(t.id)}
              title="关闭提醒"
              className="pointer-events-auto ml-0.5 rounded-full p-0.5 text-white/60 hover:bg-white/20 hover:text-white"
            >
              <X size={11} />
            </button>
          )}
        </div>
      ))}
    </div>
  )
}
