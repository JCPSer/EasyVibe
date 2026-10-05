import { Component, type ReactNode } from 'react'

// R1 兜底：渲染异常拦截，白屏换成可恢复提示
export class CanvasBoundary extends Component<{ children: ReactNode }, { err: Error | null }> {
  state = { err: null as Error | null }
  static getDerivedStateFromError(err: Error) {
    return { err }
  }
  render() {
    if (this.state.err) {
      return (
        <div className="flex h-screen flex-col items-center justify-center gap-2 text-[13px] text-slate-500 dark:text-slate-400">
          <span className="font-semibold text-slate-700 dark:text-slate-200">页面渲染出错（已拦截白屏）</span>
          <span className="max-w-[420px] text-center text-slate-400 dark:text-slate-500">{String(this.state.err).slice(0, 200)}</span>
          <button className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-1.5 text-blue-600 hover:bg-slate-50 dark:hover:bg-slate-800/70" onClick={() => location.reload()}>
            刷新恢复
          </button>
        </div>
      )
    }
    return this.props.children
  }
}
