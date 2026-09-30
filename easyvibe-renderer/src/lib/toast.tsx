import { useEffect, useState } from 'react'
import { AlertCircle, Info } from 'lucide-react'

// R3 清债：统一 toast 反馈——替代裸 alert 兜底（§0 标准"无裸 alert"）
type Toast = { id: number; msg: string; kind: 'info' | 'error' }

let listeners: ((t: Toast) => void)[] = []
let seq = 0

export function toast(msg: string, kind: 'info' | 'error' = 'info') {
  const t = { id: ++seq, msg, kind }
  listeners.forEach((l) => l(t))
}

export function ToastHost() {
  const [items, setItems] = useState<Toast[]>([])
  useEffect(() => {
    const l = (t: Toast) => {
      setItems((prev) => [...prev.slice(-2), t])
      setTimeout(() => setItems((prev) => prev.filter((x) => x.id !== t.id)), 3200)
    }
    listeners.push(l)
    return () => {
      listeners = listeners.filter((x) => x !== l)
    }
  }, [])
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
        </div>
      ))}
    </div>
  )
}
