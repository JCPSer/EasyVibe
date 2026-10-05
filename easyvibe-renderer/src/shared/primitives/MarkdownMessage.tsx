import { memo, useEffect, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

// Mermaid 图渲染：懒加载 mermaid（包体大），失败回退为代码块
const MermaidBlock = memo(function MermaidBlock({ chart }: { chart: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    import('mermaid')
      .then((mermaid) => {
        if (cancelled || !ref.current) return
        mermaid.default.initialize({ startOnLoad: false, theme: 'neutral', securityLevel: 'strict' })
        const id = `mmd-${Math.random().toString(36).slice(2, 9)}`
        // 语法预检：模型生成的 mermaid 常有语法错误——parse 失败直接回退代码块。
        // 不能跳过这步直接 render：mermaid 12 render 失败时会向 body 塞入裸错误节点
        // （"Syntax error in text"炸弹图挂在页面底部，试用现场实证）
        try {
          mermaid.default.parse(chart)
        } catch {
          if (!cancelled) setFailed(true)
          return
        }
        mermaid.default
          .render(id, chart)
          .then(({ svg }) => {
            if (!cancelled && ref.current) ref.current.innerHTML = svg
          })
          .catch(() => {
            document.getElementById(id)?.remove() // 清理 mermaid 塞到 body 的错误节点
            if (!cancelled) setFailed(true)
          })
      })
      .catch(() => setFailed(true))
    return () => {
      cancelled = true
    }
  }, [chart])

  if (failed) return <pre className="overflow-x-auto rounded bg-slate-100 dark:bg-slate-800 p-2 text-cap">{chart}</pre>
  return <div ref={ref} className="my-1 overflow-x-auto rounded bg-white dark:bg-slate-900 p-1 [&>svg]:mx-auto" />
})

// 对话消息的 Markdown 渲染：GFM（表格/列表/粗体）+ mermaid 代码块
export const MarkdownMessage = memo(function MarkdownMessage({ content }: { content: string }) {
  return (
    <div className="text-[12px] leading-5 [&_code]:rounded [&_code]:bg-slate-100 dark:[&_code]:bg-slate-800 [&_code]:px-1 [&_code]:py-px [&_code]:font-mono [&_code]:text-cap [&_h1]:text-[13px] [&_h1]:font-bold [&_h2]:text-[12px] [&_h2]:font-bold [&_li]:ml-3 [&_li]:list-disc [&_p]:my-1 [&_pre]:my-1 [&_strong]:font-semibold [&_table]:my-1 [&_td]:border [&_td]:border-slate-200 dark:[&_td]:border-slate-700 [&_td]:px-2 [&_td]:py-1 [&_td]:align-top [&_th]:border [&_th]:border-slate-200 dark:[&_th]:border-slate-700 [&_th]:bg-slate-50 dark:[&_th]:bg-slate-800 [&_th]:px-2 [&_th]:py-1 [&_th]:font-semibold [&_th]:whitespace-nowrap">
      {/* 表格独立横向滚动容器：窄栏（右栏 340px）里宽表不再硬挤换行，表头保持单行 */}
      <div className="-mx-1 overflow-x-auto px-1">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre({ children }) {
            // 从 pre>code 中提取 mermaid
            const code = (children as { props?: { className?: string; children?: string | string[] } } | undefined)?.props
            const cls = code?.className ?? ''
            const text = Array.isArray(code?.children) ? code.children.join('') : (code?.children ?? '')
            if (cls.includes('language-mermaid')) return <MermaidBlock chart={String(text).trim()} />
            return <pre className="overflow-x-auto rounded bg-slate-100 dark:bg-slate-800 p-2 text-cap">{children}</pre>
          },
        }}
      >
        {content}
      </ReactMarkdown>
      </div>
    </div>
  )
})
