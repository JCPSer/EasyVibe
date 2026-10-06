// 右栏 ThreadStream：回合文档流（问句右对齐 / 回答全宽顶线分隔）+ 审批出口卡 + 空态 + 视图保存 + 澄清卡。
// 拆自 QuickAsk.tsx（2026-10-05 防膨胀）。
import { useMemo, useState } from 'react'
import { AlertTriangle, ArrowRight, AtSign, BookmarkPlus, Check, Copy, Crosshair, Loader2, MessagesSquare } from 'lucide-react'
import { toast } from '@/runtime/toast'
import { saveView } from '@/api/chat'
import { AnswerCards } from '@/components/chat/AnswerCards'
import { ONBOARDING_COPY } from '@/shared/logic/onboardingCopy'
import { GATE_LABEL, type ChatMessage, type Clarify, type PendingApproval } from './types'
import type { CodeMap } from '@/types/map'

export function QuickAskStream({
  listRef, hasMore, loadingMore, loadEarlier, messages, pendingApprovals, onGoWorkbench, map,
  setInput, setClarify, textareaRef, onLocateModule, clarify, sending, backendRepo,
}: {
  listRef: React.RefObject<HTMLDivElement | null>
  hasMore: boolean
  loadingMore: boolean
  loadEarlier: () => void
  messages: ChatMessage[]
  pendingApprovals: PendingApproval[]
  onGoWorkbench?: () => void
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

  const hasFooter = (m: ChatMessage) => m.role === 'assistant' && (m.refs.length > 0 || /```mermaid/.test(m.content))
  const lastFooterIdx = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i]
      if (m.role === 'assistant' && (m.refs.length > 0 || /```mermaid/.test(m.content))) return i
    }
    return -1
  }, [messages])

  const saveAsView = (idx: number) => {
    // 后端未就绪时 repo 为空：保持既有观感（原裸 fetch 会拼出 /repos/null 并落到同一错误文案）
    if (!backendRepo) {
      toast('存视图失败（需要本地后端在线）', 'error')
      return
    }
    const m = messages[idx]
    const q = messages.slice(0, idx).reverse().find((x) => x.role === 'user')?.content ?? '对话视图'
    const name = (viewName.trim() || q).slice(0, 40)
    saveView(backendRepo, {
      name,
      nodes: m.refs.map((id) => `module:${id}`),
      edges: [],
      annotations: [
        { ref: 'conversation', note: q },
        ...(m.content.match(/```mermaid[\s\S]*?```/g) ?? []).map((content) => ({ type: 'mermaid', content, note: q })),
      ],
    })
      .then(async (r) => {
        if (r.status === 409) {
          if (window.confirm(`已存在同名视图「${name}」。覆盖它？（取消则放弃保存）`)) {
            const again = await saveView(
              backendRepo,
              {
                name,
                nodes: m.refs.map((id) => `module:${id}`),
                edges: [],
                annotations: [
                  { ref: 'conversation', note: q },
                  ...(m.content.match(/```mermaid[\s\S]*?```/g) ?? []).map((content) => ({ type: 'mermaid', content, note: q })),
                ],
              },
              '?force=true',
            )
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
    requestAnimationFrame(() => textareaRef.current?.focus())
  }

  return (
    <div ref={listRef} className="flex-1 space-y-5 overflow-y-auto pt-3">
      {hasMore && (
        <div className="flex justify-center">
          <button
            onClick={loadEarlier}
            disabled={loadingMore}
            className="text-cap text-slate-400 transition-colors hover:text-slate-600 disabled:opacity-40 dark:text-slate-500 dark:hover:text-slate-300"
          >
            {loadingMore ? '加载中…' : '↑ 加载更早的消息'}
          </button>
        </div>
      )}
      {messages.length === 0 && pendingApprovals.length === 0 && (
        /* 空态邀请卡：图标 + 一句主张 + 示例 chip 一处说完 */
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
                className="rounded-full bg-slate-100 px-2.5 py-1 text-cap text-slate-600 transition-colors hover:bg-blue-50 hover:text-blue-600 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-blue-950/40 dark:hover:text-blue-400"
              >
                {p}
              </button>
            ))}
          </div>
        </div>
      )}
      {/* 审批卡：只出口——「去工作台审批」，不做内联决策（形态分裂裁决） */}
      {pendingApprovals.map((p) => (
        <div key={p.taskId + ':' + (p.gate ?? '')} className="anim-msg-in rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 dark:border-amber-900/60 dark:bg-amber-950/40">
          <p className="flex items-center gap-1.5 text-cap font-semibold leading-4 text-amber-800 dark:text-amber-200">
            <AlertTriangle size={11} />
            审批请求 <span className="rounded-full bg-white/80 px-1.5 text-micro font-bold text-amber-600 dark:bg-slate-900/80 dark:text-amber-400">{GATE_LABEL[p.gate ?? ''] ?? p.gate ?? '审批'}</span>
          </p>
          <p className="mt-1 text-[12px] leading-5 text-slate-700 dark:text-slate-200">{p.title}</p>
          {onGoWorkbench && (
            <button
              onClick={onGoWorkbench}
              className="mt-1.5 flex items-center gap-1 rounded-lg bg-white px-2 py-1 text-micro font-semibold text-amber-700 shadow-sm transition-colors hover:bg-amber-100 dark:bg-slate-900 dark:text-amber-400 dark:hover:bg-amber-900/40"
            >
              去工作台审批 <ArrowRight size={10} />
            </button>
          )}
        </div>
      ))}
      {messages.map((m, i) =>
        m.role === 'system' ? (
          <div key={m.id ?? i} className="anim-msg-in flex justify-center">
            <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-micro text-slate-400 dark:bg-slate-800 dark:text-slate-500">{m.content}</span>
          </div>
        ) : m.role === 'user' ? (
          /* 用户问句：右对齐、无气泡、零蓝色（颜色宪法：蓝=交互对象） */
          <div key={m.id ?? i} className="anim-msg-in flex justify-end">
            <div className="max-w-[92%] select-text text-[12px] font-semibold leading-5 text-slate-800 dark:text-slate-100">
              {m.images?.map((im, j) => (
                <img key={j} src={im.dataUrl} alt={im.name} className="mb-1.5 max-h-40 rounded-lg" />
              ))}
              {m.mentions && m.mentions.length > 0 && (
                <span className="mb-1 flex flex-wrap justify-end gap-1">
                  {m.mentions.map((mm) => (
                    <span key={mm.id} className="flex items-center gap-0.5 rounded-full bg-slate-100 px-1.5 py-0.5 font-normal text-micro text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                      <AtSign size={8} />
                      {mm.name}
                    </span>
                  ))}
                </span>
              )}
              <span className="block whitespace-pre-wrap break-words">{m.content}</span>
            </div>
          </div>
        ) : (
          /* 助手回答：全宽文档流，顶线分隔；有章节结构时 AnswerCards 拆卡 */
          <div key={m.id ?? i} className="anim-msg-in group/msg border-t border-slate-100 pt-2 dark:border-slate-800">
            <div className="select-text text-[12px] leading-5 text-slate-700 dark:text-slate-200">
              {!m.content.trim() ? (
                <span className="text-slate-400 dark:text-slate-500">（此条未获得回答）</span>
              ) : (
                <AnswerCards content={m.content} />
              )}
            </div>
            {hasFooter(m) && (
              <div className="mt-2 flex items-center gap-1 border-t border-slate-100 pt-1.5 dark:border-slate-800">
                {/* 引用芯片：裸 mono，hover 定位（蓝=交互对象） */}
                {m.refs.map((id) => (
                  <button
                    key={id}
                    onClick={() => onLocateModule(id)}
                    className="flex items-center gap-0.5 font-mono text-micro text-slate-500 transition-colors hover:text-blue-600 dark:text-slate-400 dark:hover:text-blue-400"
                    title="定位到画布"
                  >
                    <Crosshair size={9} />
                    {id}
                  </button>
                ))}
                <span className="ml-auto flex items-center gap-1">
                  {/* 复制：悬停浮现；流内最后一条回答常显（opacity-40） */}
                  <button
                    onClick={() => {
                      void navigator.clipboard?.writeText(m.content).then(
                        () => toast('已复制该条消息', 'info'),
                        () => toast('复制失败（剪贴板不可用）', 'error'),
                      )
                    }}
                    className={`rounded p-1 text-slate-300 transition-opacity hover:text-slate-500 dark:text-slate-600 dark:hover:text-slate-300 ${i === lastFooterIdx ? 'opacity-40' : 'opacity-0 group-hover/msg:opacity-100'}`}
                    title="复制该条回答"
                  >
                    <Copy size={10} />
                  </button>
                  {namingIdx === i ? (
                    /* 存图命名——多张视图靠问题截断无法区分，存前给命名框 */
                    <span className="flex items-center gap-1">
                      <input
                        autoFocus
                        value={viewName}
                        onChange={(e) => setViewName(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') saveAsView(i)
                          if (e.key === 'Escape') setNamingIdx(null)
                        }}
                        placeholder="视图名…"
                        className="w-32 rounded-full border border-emerald-300 bg-white px-2 py-0.5 text-cap outline-none dark:border-emerald-800 dark:bg-slate-900 dark:text-slate-200"
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
                      className="flex items-center gap-1 rounded-full bg-emerald-50 px-2 py-0.5 text-micro font-semibold text-emerald-700 transition-colors hover:bg-emerald-100 dark:bg-emerald-950/40 dark:text-emerald-400 dark:hover:bg-emerald-900/40"
                      title="把本回答（含流程图）存为可复用视图（.easyvibe/views/）"
                    >
                      {savedIdx === i ? <Check size={10} /> : <BookmarkPlus size={10} />}
                      {savedIdx === i ? '已存视图' : '存为视图'}
                    </button>
                  )}
                </span>
              </div>
            )}
          </div>
        ),
      )}
      {/* S1 grill-me 澄清卡：选择题形态，点选即填入输入框 */}
      {clarify && (
        <div className="anim-msg-in rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 dark:border-amber-900/60 dark:bg-amber-950/40">
          <p className="text-cap font-semibold leading-4 text-amber-800 dark:text-amber-200">
            {clarify.question}
            {clarify.why && (
              <span className="ml-1 font-normal text-amber-500" title={clarify.why}>
                ⓘ
              </span>
            )}
          </p>
          <div className="mt-1.5 space-y-1">
            {clarify.options.map((o) => (
              <button
                key={o.label}
                onClick={() => answerClarify(o.label, o.desc)}
                className="flex w-full flex-col items-start gap-0.5 rounded-md border border-amber-200 bg-white px-2 py-1.5 text-left transition-colors hover:border-blue-300 hover:bg-blue-50 dark:border-amber-900/60 dark:bg-slate-900 dark:hover:bg-blue-950/40"
              >
                <span className="text-cap font-medium leading-4 text-slate-700 dark:text-slate-200">{o.label}</span>
                {o.desc && <span className="text-micro leading-4 text-slate-400 dark:text-slate-500">{o.desc}</span>}
              </button>
            ))}
          </div>
        </div>
      )}
      {sending && (
        <p className="anim-msg-in flex items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
          <Loader2 size={10} className="animate-spin" /> 正在查询地图…
        </p>
      )}
    </div>
  )
}
