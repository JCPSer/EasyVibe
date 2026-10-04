import { Waypoints, ShieldCheck, Lightbulb, FileText } from 'lucide-react'
import { MarkdownMessage } from '@/components/MarkdownMessage'

// M4-2 答案卡片三型（按 ui-mockups/对话面板原型.png 重画）：
// 助手回答按章节（##/### 标题）拆分为卡片，按关键词定型——
// 依赖/Dependency → 依赖卡；架构/Architecture/检查 → 体检卡；建议/Recommendation → 建议卡；其他 → 普通卡。
// 无章节结构时回退为纯 Markdown 渲染（零副作用）。
interface Section {
  title: string
  body: string
}

function splitSections(content: string): Section[] | null {
  // 顶层标题切分（行首 ## 或 ###）；代码块内的 # 不参与（简易状态机）
  const lines = content.split('\n')
  const sections: Section[] = []
  let cur: Section | null = null
  let inFence = false
  for (const line of lines) {
    if (line.trimStart().startsWith('```')) {
      inFence = !inFence
      if (cur) cur.body += line + '\n'
      continue
    }
    if (!inFence) {
      const m = line.match(/^(#{2,3})\s+(.+)$/)
      if (m) {
        if (cur) sections.push(cur)
        cur = { title: m[2].trim(), body: '' }
        continue
      }
    }
    if (cur) cur.body += line + '\n'
  }
  if (cur) sections.push(cur)
  if (sections.length < 2) return null // 章节太少不成卡（回退纯渲染）
  const head = lines.slice(0, lines.findIndex((l) => /^(#{2,3})\s+/.test(l))).join('\n').trim()
  if (head) sections.unshift({ title: '', body: head })
  return sections
}

const CARD_STYLE = [
  { match: /依赖|dependenc|引用/i, icon: Waypoints, cls: 'border-blue-200 bg-blue-50/50', iconCls: 'text-blue-500' },
  { match: /架构|architecture|体检|违规|分层/i, icon: ShieldCheck, cls: 'border-emerald-200 bg-emerald-50/50', iconCls: 'text-emerald-600' },
  { match: /建议|recommend|优化|修复/i, icon: Lightbulb, cls: 'border-amber-200 bg-amber-50/50', iconCls: 'text-amber-500' },
]

export function AnswerCards({ content }: { content: string }) {
  const sections = splitSections(content)
  if (!sections) return <MarkdownMessage content={content} />
  return (
    <div className="space-y-2">
      {sections.map((s, i) => {
        const style = CARD_STYLE.find((c) => c.match.test(s.title)) ?? { icon: FileText, cls: 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900', iconCls: 'text-slate-400 dark:text-slate-500' }
        const Icon = style.icon
        return (
          <div key={i} className={`anim-scale-in rounded-lg border px-3 py-2 ${style.cls}`}>
            {s.title && (
              <p className="mb-1 flex items-center gap-1.5 text-[11px] font-bold text-slate-700 dark:text-slate-200">
                <Icon size={12} className={style.iconCls} />
                {s.title}
              </p>
            )}
            <div className="text-[12px] leading-5 text-slate-600 dark:text-slate-300">
              <MarkdownMessage content={s.body.trim()} />
            </div>
          </div>
        )
      })}
    </div>
  )
}
