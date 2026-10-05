// ③ 实施（及 ①② 产文档期间）实时终端：模块级环形缓冲 + 跟随滚动。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
// 2026-10-05 实弹#2：终端原来只靠 WS 推送攒缓冲——WS 断线时（全应用无任何感知）
// 页面一切正常唯独这里永远"等待 agent 输出"。补 HTTP 增量补拉兜底（运行期 3s 一拍，
// afterSeq 锚点 + seq 幂等去重），WS 活着时补拉为空转无副作用。
import { useEffect, useRef, useState } from 'react'
import { Terminal, Unplug } from 'lucide-react'
import { terminalLines, terminalLastSeq, pushTerminalLine } from '@/runtime/terminalBuffer'
import { taskDuration } from '../diffParse'
import type { TaskItem } from '../types'

export function TerminalStage({ sel, repo, onOpenRuns }: { sel: TaskItem; repo?: string | null; onOpenRuns?: (sessionId: string) => void }) {
  const isRunning = sel.status === 'running'
  const sessionId = sel.sessionId ?? ''
  // 终端跟随滚动（用户上翻时暂停跟随）
  const termRef = useRef<HTMLPreElement | null>(null)
  const [follow, setFollow] = useState(true)
  // 终端行数戳：模块级缓冲不触发渲染，靠 1s 节拍与任务事件刷新
  const [termTick, setTermTick] = useState(0)
  useEffect(() => {
    if (!isRunning) return
    const t = window.setInterval(() => setTermTick((n) => n + 1), 1000)
    return () => window.clearInterval(t)
  }, [isRunning])
  useEffect(() => {
    if (follow && termRef.current) termRef.current.scrollTop = termRef.current.scrollHeight
  }, [termTick, follow, sel.sessionId])

  // HTTP 补拉兜底：首挂全量（afterSeq=0）+ 运行期 3s 增量——WS 断线时终端仍有输出（延迟 ≤3s）
  useEffect(() => {
    if (!isRunning || !sessionId || !repo) return
    let cancelled = false
    const pull = () => {
      fetch(
        `/api/repos/${encodeURIComponent(repo)}/sessions/${encodeURIComponent(sessionId)}/output?afterSeq=${terminalLastSeq(sessionId) + 1}&limit=5000`,
      )
        .then((r) => (r.ok ? r.json() : null))
        .then((d: { data?: { seq: number; stream: string; line: string }[] } | null) => {
          if (cancelled) return
          for (const row of d?.data ?? []) pushTerminalLine(sessionId, row.seq, row.stream, row.line)
        })
        .catch(() => {})
    }
    pull()
    const t = window.setInterval(pull, 3000)
    return () => {
      cancelled = true
      window.clearInterval(t)
    }
  }, [isRunning, sessionId, repo])

  const lines = terminalLines(sessionId)
  return (
    <div className="flex min-h-0 flex-1 flex-col p-4">
      <div className="mb-2 flex items-center gap-2 text-micro text-slate-400 dark:text-slate-500">
        <Terminal size={11} />
        <span className="font-bold uppercase tracking-wider">
          {sel.gate === 'p:analysis' ? '需求分析产出中' : sel.gate === 'p:solution' ? '方案设计产出中' : '实时执行'}
        </span>
        <span className="tnum ml-auto flex items-center gap-1.5">
          <span className="flex h-1.5 w-1.5 animate-pulse rounded-full bg-red-500" /> LIVE · 已运行 {taskDuration(sel)}
        </span>
        {onOpenRuns && sel.sessionId && (
          <button
            onClick={() => onOpenRuns(sel.sessionId!)}
            className="flex items-center gap-1 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600"
            title="跳「运行」页看本会话的完整流水（含历史回放）"
          >
            完整流水 →
          </button>
        )}
      </div>
      <pre
        ref={termRef}
        onScroll={(e) => {
          const el = e.currentTarget
          setFollow(el.scrollHeight - el.scrollTop - el.clientHeight < 24)
        }}
        className="select-text mono min-h-0 flex-1 overflow-y-auto rounded-xl bg-slate-900 p-3 text-[11px] leading-5 text-slate-300 dark:text-slate-600"
      >
        {lines.length === 0 ? (
          <span className="text-slate-500 dark:text-slate-400">等待 agent 输出…（agent 启动可能需要 1-2 分钟）</span>
        ) : (
          lines.map((l) => (
            <div key={l.seq} className={l.stream === 'stderr' || l.line.startsWith('[err]') ? 'text-red-400' : ''}>{l.line}</div>
          ))
        )}
        {/* WS 断线无回放是已知边界（方案 §6）——但终端现在有 HTTP 补拉兜底，仅极端双通道齐断才不可回放 */}
        <div className="mt-1 flex items-center gap-1 text-slate-600 dark:text-slate-300">
          <Unplug size={10} /> 实时推送断开时输出延迟约 3 秒（HTTP 兜底补拉）
        </div>
      </pre>
      <p className="mt-1.5 text-[10px] text-slate-400 dark:text-slate-500">
        {sel.gate === 'p:analysis'
          ? '需求矩阵将写入 .easyvibe/development_docs/1_requirements_matrix/，产出后在此评审'
          : sel.gate === 'p:solution'
            ? '方案设计将写入 2_requirements_solutions/，产出后在此评审'
            : '改动文件列表在任务完成后由 Diff 呈现（实时全量文件流为后置需求）'}
      </p>
    </div>
  )
}
