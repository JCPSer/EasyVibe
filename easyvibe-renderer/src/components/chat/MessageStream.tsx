// 消息流（聊天主体）：待审批内联卡 + 分页上翻 + 空态邀请卡 + 气泡/全宽回答 + 视图保存 + 澄清卡。
// 拆自 ChatPanel.tsx（2026-10-05 防膨胀）。
import { useMemo, useState } from 'react'
import { AlertTriangle, AtSign, BookmarkPlus, Check, CheckCircle2, Copy, Crosshair, Loader2, MessagesSquare } from 'lucide-react'
import { toast } from '@/lib/toast'
import { AnswerCards } from '@/components/AnswerCards'
import { MarkdownMessage } from '@/components/MarkdownMessage'
import { ONBOARDING_COPY } from '@/lib/onboardingCopy'
import { GATE_LABEL, type ChatMessage, type Clarify, type PendingApproval } from './types'
import type { CodeMap } from '@/types/map'

export function MessageStream({
  listRef, pendingApprovals, decided, rejectingApproval, setRejectingApproval, rejectNote, setRejectNote, decide,
  hasMore, loadingMore, loadEarlier, messages, map, setInput, setClarify, textareaRef, onLocateModule, clarify, sending, backendRepo,
}: {
  listRef: React.RefObject<HTMLDivElement | null>
  pendingApprovals: PendingApproval[]
  decided: Record<string, 'approved' | 'rejected'>
  rejectingApproval: PendingApproval | null
  setRejectingApproval: (p: PendingApproval | null) => void
  rejectNote: string
  setRejectNote: (v: string) => void
  decide: (p: PendingApproval, decision: 'approved' | 'rejected', note?: string) => void
  hasMore: boolean
  loadingMore: boolean
  loadEarlier: () => void
  messages: ChatMessage[]
  map: CodeMap | null
  setInput: (v: string) => void
  setClarify: (c: Clarify | null) => void
  textareaRef: React.RefObject<HTMLTextAreaElement | null>
  onLocateModule: (moduleId: string) => void
  clarify: Clarify | null
  sending: boolean
  backendRepo: string | null
}) {
  const [savedIdx, setSavedIdx] = useState<number | null>(null)
  const [namingIdx, setNamingIdx] = useState<number | null>(null)
  const [viewName, setViewName] = useState('')
  const samplePrompts = useMemo(() => {
    const sp = ONBOARDING_COPY.samplePrompts
    const first = map?.modules[0]
    return [
      ...(first ? [sp.withModule.replace('{module}', first.name)] : []),
      ...sp.generic,
    ]
  }, [map])

  const saveAsView = (idx: number) => {
    const m = messages[idx]
    const q = messages.slice(0, idx).reverse().find((x) => x.role === 'user')?.content ?? '对话视图'
    const name = (viewName.trim() || q).slice(0, 40)
    fetch(`/api/repos/${backendRepo}/views`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name,
        nodes: m.refs.map((id) => `module:${id}`),
        edges: [],
        annotations: [
          { ref: 'conversation', note: q },
          ...(m.content.match(/```mermaid[\s\S]*?```/g) ?? []).map((content) => ({ type: 'mermaid', content, note: q })),
        ],
      }),
    })
      .then(async (r) => {
        if (r.status === 409) {
          if (window.confirm(`已存在同名视图「${name}」。覆盖它？（取消则放弃保存）`)) {
            const again = await fetch(`/api/repos/${backendRepo}/views?force=true`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                name,
                nodes: m.refs.map((id) => `module:${id}`),
                edges: [],
                annotations: [
                  { ref: 'conversation', note: q },
                  ...(m.content.match(/```mermaid[\s\S]*?```/g) ?? []).map((content) => ({ type: 'mermaid', content, note: q })),
                ],
              }),
            })
            if (!again.ok) throw new Error(String(again.status))
          } else {
            return
          }
        } else if (!r.ok) {
          throw new Error(String(r.status))
        }
        setSavedIdx(idx)
        setNamingIdx(null)
        setTimeout(() => setSavedIdx(null), 2500)
      })
      .catch(() => toast('存视图失败（需要本地后端在线）', 'error'))
  }

  const answerClarify = (label: string, desc?: string) => {
    setClarify(null)
    setInput(`选择：${label}${desc ? `（${desc}）` : ''}`)
    setTimeout(() => {
      const btn = document.querySelector<HTMLTextAreaElement>('textarea[placeholder^="问点什么"]')
      btn?.focus()
    }, 50)
  }

  return (
    <div ref={listRef} className="flex-1 space-y-3 overflow-y-auto">
      {/* M4-2 内联审批卡：等待中的审批门（AionUI：选项即按钮，决策后原地留痕） */}
      {pendingApprovals.map((p) => (
        <div key={p.taskId + ':' + (p.gate ?? '')} className="anim-msg-in flex justify-start">
          <div className="max-w-[92%] rounded-lg border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-3 py-2">
            <p className="flex items-center gap-1.5 text-[11px] font-semibold leading-4 text-amber-800">
              <AlertTriangle size={11} />
              审批请求 <span className="rounded-full bg-white/80 dark:bg-slate-900/80 px-1.5 text-micro font-bold text-amber-600">{GATE_LABEL[p.gate ?? ''] ?? p.gate ?? '审批'}</span>
            </p>
            <p className="mt-1 text-[12px] leading-5 text-slate-700 dark:text-slate-200">{p.title}</p>
            {decided[p.taskId + ':' + (p.gate ?? '')] ? (
              <p className="mt-1.5 flex items-center gap-1 text-cap font-semibold text-emerald-600">
                <CheckCircle2 size={11} /> 已{decided[p.taskId + ':' + (p.gate ?? '')] === 'approved' ? '通过' : '驳回'}
              </p>
            ) : rejectingApproval?.taskId === p.taskId && rejectingApproval.gate === p.gate ? (
              <div className="mt-1.5 space-y-1.5">
                <textarea
                  value={rejectNote}
                  onChange={(e) => setRejectNote(e.target.value)}
                  rows={2}
                  placeholder="驳回理由（必填，留痕可追溯）"
                  className="w-full resize-none rounded-lg border border-red-200 dark:border-red-900/60 bg-white dark:bg-slate-900 px-2 py-1.5 text-[11px] leading-4 text-slate-700 dark:text-slate-200 outline-none focus:border-red-400"
                />
                <div className="flex gap-2">
                  <button
                    onClick={() => decide(p, 'rejected', rejectNote.trim())}
                    disabled={!rejectNote.trim()}
                    className="flex-1 rounded-lg bg-red-600 px-3 py-1.5 text-[11px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
                  >
                    确认驳回
                  </button>
                  <button
                    onClick={() => {
                      setRejectingApproval(null)
                      setRejectNote('')
                    }}
                    className="flex-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-3 py-1.5 text-[11px] font-bold text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70"
                  >
                    取消
                  </button>
                </div>
              </div>
            ) : (
              <div className="mt-1.5 flex gap-2">
                <button
                  onClick={() => decide(p, 'approved')}
                  className="flex-1 rounded-lg bg-blue-600 px-3 py-1.5 text-[11px] font-bold text-white hover:bg-blue-700"
                >
                  通过
                </button>
                <button
                  onClick={() => {
                    setRejectingApproval(p)
                    setRejectNote('')
                  }}
                  className="flex-1 rounded-lg border border-red-200 dark:border-red-900/60 bg-white dark:bg-slate-900 px-3 py-1.5 text-[11px] font-bold text-red-600 hover:bg-red-50 dark:hover:bg-red-950/40"
                >
                  驳回
                </button>
              </div>
            )}
          </div>
        </div>
      ))}
      {hasMore && (
        <div className="flex justify-center">
          <button
            onClick={loadEarlier}
            disabled={loadingMore}
            className="rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-3 py-1 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
          >
            {loadingMore ? '加载中…' : '↑ 加载更早的消息'}
          </button>
        </div>
      )}
      {messages.length === 0 && pendingApprovals.length === 0 && (
        /* 2026-10-04 设计师评审#8：空态邀请卡化——图标+一句主张+示例 chip 一处说完，
           不再「居中灰字<br/>换行」像 API 文档页；示例不再常驻输入框上方 */
        <div className="flex flex-col items-center gap-3 px-4 pt-12 text-center">
          <span className="flex h-10 w-10 items-center justify-center rounded-2xl bg-blue-50 text-blue-500 dark:bg-blue-950/40 dark:text-blue-400">
            <MessagesSquare size={18} />
          </span>
          <div>
            <p className="text-[12px] font-semibold text-slate-600 dark:text-slate-300">基于语义代码地图提问</p>
            <p className="mt-0.5 text-micro text-slate-400 dark:text-slate-500">回答可存为视图，引用模块可定位到画布</p>
          </div>
          <div className="flex flex-wrap justify-center gap-1.5">
            {samplePrompts.map((p) => (
              <button
                key={p}
                onClick={() => { setInput(p); textareaRef.current?.focus() }}
                className="rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1 text-[11px] text-slate-500 dark:text-slate-400 shadow-sm transition-colors hover:border-blue-300 hover:text-blue-600"
              >
                {p}
              </button>
            ))}
          </div>
        </div>
      )}
      {messages.map((m, i) =>
        m.role === 'system' ? (
          <div key={i} className="anim-msg-in flex justify-center">
            <span className="rounded-full bg-slate-100 dark:bg-slate-800 px-2.5 py-0.5 text-micro text-slate-400 dark:text-slate-500">{m.content}</span>
          </div>
        ) : (
          <div key={i} className={`anim-msg-in group/msg flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}>
            <div
              className={`select-text max-w-[88%] rounded-lg px-3 py-2 text-[12px] leading-5 ${
                /* 2026-10-04 设计师评审#4/#11：用户气泡去饱和（blue-600 实底=与发送按钮同色抢层级，
                   glass 菜单叠纯蓝发浊）；蓝=交互对象，灰/浅tint=内容容器——颜色宪法 */
                m.role === 'user'
                  ? 'border border-blue-100 bg-blue-50 text-blue-900 dark:border-blue-900/50 dark:bg-blue-950/40 dark:text-blue-100'
                  : 'bg-white dark:bg-slate-900 text-slate-700 dark:text-slate-200'
              }`}
            >
              {m.images?.map((im, j) => (
                <img key={j} src={im.dataUrl} alt={im.name} className="mb-1.5 max-h-40 rounded-lg" />
              ))}
              {m.mentions && m.mentions.length > 0 && (
                <div className="mb-1 flex flex-wrap gap-1">
                  {m.mentions.map((mm) => (
                    <span key={mm.id} className="flex items-center gap-0.5 rounded-full bg-blue-100/80 px-1.5 py-0.5 text-micro text-blue-700 dark:bg-blue-900/50 dark:text-blue-200">
                      <AtSign size={8} />
                      {mm.name}
                    </span>
                  ))}
                </div>
              )}
              {m.role === 'assistant' && !m.content.trim() ? (
                <span className="text-slate-400 dark:text-slate-500">（此条未获得回答）</span>
              ) : m.role === 'assistant' ? (
                /* M4-2 答案卡片三型（对话面板原型）：有章节结构的回答拆卡渲染 */
                <AnswerCards content={m.content} />
              ) : (
                <MarkdownMessage content={m.content} />
              )}
              {/* 审计 P2：消息单条复制（此前只能导出全文或手选）——悬停浮现，组内免打扰。
                  图标按钮化：9px 文字违字阶纪律（设计师评审#6），tooltip 承载说明 */}
              <span className="mt-1 flex justify-end opacity-0 transition-opacity group-hover/msg:opacity-100">
                <button
                  onClick={() => {
                    void navigator.clipboard?.writeText(m.content).then(
                      () => toast('已复制该条消息', 'info'),
                      () => toast('复制失败（剪贴板不可用）', 'error'),
                    )
                  }}
                  className={`rounded p-1 ${
                    m.role === 'user'
                      ? 'text-blue-300 hover:text-blue-600 dark:text-blue-700 dark:hover:text-blue-300'
                      : 'text-slate-300 dark:text-slate-600 hover:text-slate-500'
                  }`}
                  title="复制该条消息"
                >
                  <Copy size={10} />
                </button>
              </span>
              {(m.role === 'assistant' || m.role === 'user') && (m.refs.length > 0 || /```mermaid/.test(m.content)) && (
                <div className="mt-2 flex flex-wrap items-center gap-1 border-t border-slate-200 dark:border-slate-700 pt-2">
                  {m.refs.map((id) => (
                    <button
                      key={id}
                      onClick={() => onLocateModule(id)}
                      className="flex items-center gap-0.5 rounded-full bg-white dark:bg-slate-900 px-2 py-0.5 font-mono text-micro text-blue-600 shadow-sm hover:bg-blue-50 dark:hover:bg-blue-950/40"
                      title="定位到画布"
                    >
                      <Crosshair size={9} />
                      {id}
                    </button>
                  ))}
                  {namingIdx === i ? (
                    /* 深挖#C：存图命名——多张视图靠问题截断无法区分，存前给命名框 */
                    <span className="ml-auto flex items-center gap-1">
                      <input
                        autoFocus
                        value={viewName}
                        onChange={(e) => setViewName(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') saveAsView(i)
                          if (e.key === 'Escape') setNamingIdx(null)
                        }}
                        placeholder="视图名…"
                        className="w-36 rounded-full border border-emerald-300 dark:border-emerald-800 bg-white dark:bg-slate-900 px-2 py-0.5 text-cap outline-none"
                      />
                      <button
                        onClick={() => saveAsView(i)}
                        disabled={!viewName.trim()}
                        className="rounded-full bg-emerald-600 px-2 py-0.5 text-micro font-bold text-white disabled:opacity-40"
                      >
                        存
                      </button>
                    </span>
                  ) : (
                    <button
                      onClick={() => {
                        const q = messages.slice(0, i).reverse().find((x) => x.role === 'user')?.content ?? '对话视图'
                        setViewName(q.replace(/\s+/g, ' ').slice(0, 24))
                        setNamingIdx(i)
                      }}
                      className="ml-auto flex items-center gap-1 rounded-full border border-emerald-200 dark:border-emerald-900/60 bg-emerald-50 dark:bg-emerald-950/40 px-2 py-0.5 text-micro font-semibold text-emerald-700 hover:bg-emerald-100"
                      title="把本回答（含流程图）存为可复用视图（.easyvibe/views/）"
                    >
                      {savedIdx === i ? <Check size={10} /> : <BookmarkPlus size={10} />}
                      {savedIdx === i ? '已存视图' : '存为视图'}
                    </button>
                  )}
                </div>
              )}
            </div>
          </div>
        ),
      )}
      {/* S1 grill-me 澄清卡：选择题形态，点选即回答 */}
      {clarify && (
        <div className="anim-msg-in flex justify-start">
          <div className="max-w-[92%] rounded-lg border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-3 py-2">
            <p className="text-[11px] font-semibold leading-4 text-amber-800">
              {clarify.question}
              {clarify.why && (
                <span className="ml-1 font-normal text-amber-500" title={clarify.why}>
                  ⓘ
                </span>
              )}
            </p>
            <div className="mt-1.5 space-y-1">
              {clarify.options.map((o) => (
                /* 改进#3：选项纵排（横排挤压曾把"仅桌面端"断成 4 行）——标签一行、理由一行 */
                <button
                  key={o.label}
                  onClick={() => answerClarify(o.label, o.desc)}
                  className="flex w-full flex-col items-start gap-0.5 rounded-md border border-amber-200 dark:border-amber-900/60 bg-white dark:bg-slate-900 px-2 py-1.5 text-left hover:border-blue-300 hover:bg-blue-50 dark:hover:bg-blue-950/40"
                >
                  <span className="text-[11px] font-medium leading-4 text-slate-700 dark:text-slate-200">{o.label}</span>
                  {o.desc && <span className="text-micro leading-4 text-slate-400 dark:text-slate-500">{o.desc}</span>}
                </button>
              ))}
            </div>
          </div>
        </div>
      )}
      {sending && (
        <div className="anim-msg-in flex justify-start">
          <div className="flex items-center gap-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/70 px-3 py-2 text-[11px] text-slate-400 dark:text-slate-500">
            <Loader2 size={12} className="animate-spin" /> 正在查询地图…
          </div>
        </div>
      )}
    </div>
  )
}
