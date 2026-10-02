import { useEffect, useState } from 'react'
import { Minus, Plus, X } from 'lucide-react'
import { isTauriRuntime } from '@/lib/env'

// 桌面壳自绘窗口控制（VSCode 范式）：系统标题栏退役（decorations=false），
// 前端渲染三枚 macOS 式红绿灯；浏览器环境渲染等宽占位保持布局一致。

type TauriWindow = {
  minimize: () => Promise<void>
  toggleMaximize: () => Promise<void>
  close: () => Promise<void>
}

const BTN = 'flex h-3.5 w-3.5 items-center justify-center rounded-full transition-opacity'

export function WindowControls() {
  const [win, setWin] = useState<TauriWindow | null>(null)
  useEffect(() => {
    if (!isTauriRuntime()) return
    import('@tauri-apps/api/window')
      .then((m) => setWin(m.getCurrentWindow() as unknown as TauriWindow))
      .catch(() => {})
  }, [])
  if (!win) return <span className="w-[68px] shrink-0" aria-hidden />
  return (
    <span className="flex w-[68px] shrink-0 items-center gap-2" data-no-drag>
      <button onClick={() => win.close()} title="关闭" className={`${BTN} bg-[#FF5F57] text-[#7A261A]`}>
        <X size={9} className="opacity-0 transition-opacity hover:opacity-100" strokeWidth={3} />
      </button>
      <button onClick={() => win.minimize()} title="最小化" className={`${BTN} bg-[#FEBC2E] text-[#8A5A00]`}>
        <Minus size={9} className="opacity-0 transition-opacity hover:opacity-100" strokeWidth={3} />
      </button>
      <button onClick={() => win.toggleMaximize()} title="全屏/还原" className={`${BTN} bg-[#28C840] text-[#0B5E1E]`}>
        <Plus size={9} className="opacity-0 transition-opacity hover:opacity-100" strokeWidth={3} />
      </button>
    </span>
  )
}
