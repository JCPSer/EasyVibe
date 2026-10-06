// 关于分区：界面语言（切换即时全界面生效）+ 应用后端版本 / Harness 版本 / 数据存储（只读事实）。
// 拆自 SettingsPanel.tsx（2026-10-05 防膨胀）。
import { useLang, type Lang } from '@/lib/i18n'
import { Select } from '@/components/ui/SelectMenu'

export function AboutSection({ about }: { about: { backend: string; harness: string } | null }) {
  const { lang, setLang, t } = useLang()
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3">
        <span className="text-[12px] text-slate-500 dark:text-slate-400">{t('common.language')}</span>
        {/* 选项用各语言自名（中文恒显「中文」），与系统设置惯例一致 */}
        <Select
          className="w-40"
          ariaLabel={t('common.language')}
          value={lang}
          options={[
            { value: 'zh', label: '中文' },
            { value: 'en', label: 'English' },
          ]}
          onChange={(v) => setLang(v as Lang)}
        />
      </div>
      {[
        { label: '应用后端版本', value: about?.backend ?? '加载中…' },
        { label: 'Harness 版本', value: about?.harness ?? '加载中…' },
        { label: '数据存储', value: '本机 SQLite（会话/任务/巡检历史）' },
      ].map((row) => (
        <div key={row.label} className="flex items-center justify-between rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3">
          <span className="text-[12px] text-slate-500 dark:text-slate-400">{row.label}</span>
          <span className="mono text-cap font-semibold text-slate-700 dark:text-slate-200">{row.value}</span>
        </div>
      ))}
      <p className="text-micro px-1 leading-4 text-slate-300 dark:text-slate-600">
        地图与产物保存在各仓库的 .easyvibe/ 目录；全部数据不出本机。
      </p>
    </div>
  )
}
