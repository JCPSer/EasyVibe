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
        mermaid.default
          .render(id, chart)
          .then(({ svg }) => {
            if (!cancelled && ref.current) ref.current.innerHTML = svg
          })
          .catch(() => setFailed(true))
      })
      .catch(() => setFailed(true))
    return () => {
      cancelled = true
    }
  }, [chart])

  if (failed) return <pre className="overflow-x-auto rounded bg-slate-100 p-2 text-[10.5px]">{chart}</pre>
  return <div ref={ref} className="my-1 overflow-x-auto rounded bg-white p-1 [&>svg]:mx-auto" />
})

// 对话消息的 Markdown 渲染：GFM（表格/列表/粗体）+ mermaid 代码块
export const MarkdownMessage = memo(function MarkdownMessage({ content }: { content: string }) {
  return (
    <div className="text-[11.5px] leading-5 [&_code]:rounded [&_code]:bg-slate-100 [&_code]:px-1 [&_code]:py-px [&_code]:font-mono [&_code]:text-[10.5px] [&_h1]:text-[13px] [&_h1]:font-bold [&_h2]:text-[12px] [&_h2]:font-bold [&_li]:ml-3 [&_li]:list-disc [&_p]:my-1 [&_pre]:my-1 [&_strong]:font-semibold [&_table]:my-1 [&_td]:border [&_td]:border-slate-200 [&_td]:px-1.5 [&_td]:py-0.5 [&_th]:border [&_th]:border-slate-200 [&_th]:bg-slate-50 [&_th]:px-1.5 [&_th]:py-0.5 [&_th]:font-semibold">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre({ children }) {
            // 从 pre>code 中提取 mermaid
            const code = (children as { props?: { className?: string; children?: string | string[] } } | undefined)?.props
            const cls = code?.className ?? ''
            const text = Array.isArray(code?.children) ? code.children.join('') : (code?.children ?? '')
            if (cls.includes('language-mermaid')) return <MermaidBlock chart={String(text).trim()} />
            return <pre className="overflow-x-auto rounded bg-slate-100 p-2 text-[10.5px]">{children}</pre>
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  )
})
