// 执行 agent 分区：探测/预设/命令与参数（独立脏状态与保存，不混入模型服务快照）。
// 拆自 SettingsPanel.tsx（2026-10-05 防膨胀）。
import { useCallback, useEffect, useState } from 'react'
import { Check, Loader2, RotateCcw, Save, ShieldCheck } from 'lucide-react'
import { toast } from '@/runtime/toast'
import { useLang } from '@/runtime/i18n'
import { agentDetect, agentStatus as fetchAgentStatus, agentTest } from '@/api/system'
import { deleteSetting, putSetting } from '@/api/settings'
import { field } from './common'
import { Field } from './controls'

export function AgentSection() {
    const { t } = useLang()
    interface AgentPreset { id: string; label: string; defaultArgs: string[]; type: string; stability: string }
    interface AgentStatus {
      detected: { command: string; path: string; version: string | null }[]
      configured: { command: string | null; args: unknown; argsTask: unknown; argsReview: unknown; preset: string | null; type: string | null }
      effective: { command: string; args: string[]; type: string; source: string; found: boolean }
      presets: AgentPreset[]
      lastTest: { ok: boolean; latencyMs: number; protocol: string; sample: string } | null
    }
    interface AgentEdit { command: string; argsGlobal: string; argsTask: string; argsReview: string; preset: string }
    const [agentStatus, setAgentStatus] = useState<AgentStatus | null>(null)
    const [agentEdit, setAgentEdit] = useState<AgentEdit | null>(null)
    const [agentSnap, setAgentSnap] = useState('')
    const [agentBusy, setAgentBusy] = useState<'save' | 'test' | 'detect' | null>(null)
    const agentDirty = agentEdit !== null && JSON.stringify(agentEdit) !== agentSnap

    const loadAgent = useCallback(async () => {
      const r = await fetchAgentStatus()
      const d = await r.json().catch(() => null)
      const st: AgentStatus | null = d?.data ?? null
      if (!st) return
      setAgentStatus(st)
      const arr = (v: unknown) => (Array.isArray(v) ? (v as string[]).join(' ') : '')
      const edit: AgentEdit = {
        command: st.configured.command ?? '',
        argsGlobal: arr(st.configured.args) || st.effective.args.join(' '),
        argsTask: arr(st.configured.argsTask),
        argsReview: arr(st.configured.argsReview),
        preset: st.configured.preset ?? 'claude',
      }
      setAgentEdit(edit)
      setAgentSnap(JSON.stringify(edit))
    }, [])

    useEffect(() => {
      void loadAgent()
    }, [loadAgent])

    const splitArgs = (s: string) => s.trim().split(/\s+/).filter(Boolean)

    const saveAgent = async () => {
      if (!agentEdit || agentBusy) return
      setAgentBusy('save')
      try {
        const put = (key: string, value: unknown) => putSetting({ scope: 'global', key, value })
        const puts: Promise<Response>[] = [
          put('agent.command', agentEdit.command.trim() || 'claude'),
          put('agent.args.global', splitArgs(agentEdit.argsGlobal)),
          put('agent.preset', agentEdit.preset),
        ]
        // 槽位：留空 = 删除键（跟随全局）——整体替换语义
        for (const [key, val] of [['agent.args.task', agentEdit.argsTask], ['agent.args.review', agentEdit.argsReview]] as const) {
          puts.push(val.trim() ? put(key, splitArgs(val)) : deleteSetting('global', key))
        }
        const rs = await Promise.all(puts)
        if (rs.some((x) => !x.ok)) throw new Error(t('settings.agent.writeFail'))
        toast(t('settings.agent.agentSaved'))
        await loadAgent()
      } catch (e) {
        toast(String(e), 'error')
      } finally {
        setAgentBusy(null)
      }
    }

    const testAgent = async () => {
      if (agentBusy) return
      setAgentBusy('test')
      try {
        const r = await agentTest()
        const d = await r.json().catch(() => null)
        if (!r.ok) throw new Error(d?.error ?? t('settings.agent.testFail'))
        toast(d.data.ok ? t('settings.agent.lastTestOk', { ms: d.data.latencyMs }) : t('settings.agent.lastTestFail', { protocol: d.data.protocol }), d.data.ok ? 'info' : 'error')
        await loadAgent()
      } catch (e) {
        toast(String(e), 'error')
      } finally {
        setAgentBusy(null)
      }
    }

    const detectAgent = async () => {
      if (agentBusy) return
      setAgentBusy('detect')
      try {
        const r = await agentDetect()
        if (!r.ok) throw new Error(t('settings.agent.detectFail'))
        await loadAgent()
        toast(t('settings.agent.detectDone'))
      } catch (e) {
        toast(String(e), 'error')
      } finally {
        setAgentBusy(null)
      }
    }

    const applyPreset = (p: AgentPreset) =>
      setAgentEdit((prev) => prev && { ...prev, preset: p.id, command: p.id, argsGlobal: p.defaultArgs.join(' ') })

  return (
    <div className="space-y-4">
      <p className="text-cap text-slate-400 dark:text-slate-500">
        {t('settings.agent.intro')}
      </p>

      {/* 状态卡：探测结果 + 生效配置 + 测试连接 */}
      <div className="space-y-2 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-cap font-semibold text-slate-500 dark:text-slate-400">{t('settings.agent.detectResult')}</span>
          {(agentStatus?.presets ?? []).map((p) => {
            const det = agentStatus?.detected.find((x) => x.command === p.id)
            return (
              <span
                key={p.id}
                className={`rounded-full px-2 py-0.5 text-micro font-semibold ${
                  det ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600' : 'bg-slate-50 dark:bg-slate-950/70 text-slate-400 dark:text-slate-500'
                }`}
                title={det?.path ?? t('settings.agent.notFoundTip')}
              >
                {det ? `● ${t('settings.agent.versionInstalled', { label: p.label, version: det.version ?? '' })}`.trim() : `○ ${t('settings.agent.notInstalledLabel', { label: p.label })}`}
              </span>
            )
          })}
          <button
            onClick={() => void detectAgent()}
            disabled={agentBusy !== null}
            className="ml-auto flex items-center gap-0.5 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro font-semibold text-slate-400 dark:text-slate-500 hover:border-blue-300 hover:text-blue-600 disabled:opacity-40"
          >
            <RotateCcw size={10} className={agentBusy === 'detect' ? 'animate-spin' : ''} /> {t('settings.agent.detect')}
          </button>
        </div>
        <p className="text-micro leading-4 text-slate-400 dark:text-slate-500">
          {t('settings.agent.effectivePrefix')}<code className="mono">{agentStatus?.effective.command ?? '…'}</code>
          {agentStatus && (
            <span>（{(() => {
              const srcKey = ({ settings: 'settings.agent.srcSettings', env: 'settings.agent.srcEnv', default: 'settings.agent.srcDefault' } as Record<string, string>)[agentStatus.effective.source]
              return srcKey ? t(srcKey) : agentStatus.effective.source
            })()}）</span>
          )}
          {agentStatus && !agentStatus.effective.found && <span className="font-semibold text-red-500"> · {t('settings.agent.exeNotFound')}</span>}
        </p>
        {agentStatus?.lastTest && (
          <p className={`flex items-center gap-1 text-micro font-semibold ${agentStatus.lastTest.ok ? 'text-emerald-600' : 'text-red-500'}`}>
            <ShieldCheck size={10} />
            {agentStatus.lastTest.ok
              ? t('settings.agent.lastTestOk', { ms: agentStatus.lastTest.latencyMs })
              : t('settings.agent.lastTestFail', { protocol: agentStatus.lastTest.protocol })}
          </p>
        )}
        <div className="flex items-center gap-2 pt-1">
          <button
            onClick={() => void testAgent()}
            disabled={agentBusy !== null}
            className="flex items-center gap-1 rounded-md bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700 disabled:opacity-40"
            title={t('settings.agent.testTip')}
          >
            {agentBusy === 'test' ? <Loader2 size={10} className="animate-spin" /> : <Check size={10} />} 测试连接
          </button>
          <span className="text-micro text-slate-300 dark:text-slate-600">{t('settings.agent.testHint')}</span>
        </div>
      </div>

      {/* 预设 */}
      <Field label={t('settings.agent.preset')} hint={t('settings.agent.presetHint')}>
        <div className="flex flex-wrap gap-1.5">
          {(agentStatus?.presets ?? []).map((p) => (
            <button
              key={p.id}
              onClick={() => applyPreset(p)}
              className={`flex items-center gap-1 rounded-full border px-2.5 py-1 text-cap font-semibold transition-colors ${
                agentEdit?.preset === p.id
                  ? 'border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 text-blue-700'
                  : 'border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
              }`}
            >
              {p.label}
              {p.stability === 'experimental' && (
                <span className="rounded-full bg-amber-50 dark:bg-amber-950/40 px-1 text-[9px] font-bold text-amber-600" title={t('settings.agent.experimentalTip')}>
                  {t('settings.agent.experimental')}
                </span>
              )}
            </button>
          ))}
        </div>
      </Field>

      <Field label={t('settings.agent.cmd')} hint={t('settings.agent.cmdHint')}>
        <input
          className={`${field} mono`}
          value={agentEdit?.command ?? ''}
          onChange={(e) => setAgentEdit((p) => p && { ...p, command: e.target.value, preset: 'custom' })}
        />
      </Field>
      <Field label={t('settings.agent.argsGlobal')} hint={t('settings.agent.argsGlobalHint')}>
        <input
          className={`${field} mono`}
          value={agentEdit?.argsGlobal ?? ''}
          onChange={(e) => setAgentEdit((p) => p && { ...p, argsGlobal: e.target.value, preset: 'custom' })}
        />
      </Field>
      <Field label={t('settings.agent.argsTask')} hint={t('settings.agent.argsTaskHint')}>
        <input
          className={`${field} mono`}
          placeholder={t('settings.agent.argsPh')}
          value={agentEdit?.argsTask ?? ''}
          onChange={(e) => setAgentEdit((p) => p && { ...p, argsTask: e.target.value })}
        />
      </Field>
      <Field label={t('settings.agent.argsReview')} hint={t('settings.agent.argsReviewHint')}>
        <input
          className={`${field} mono`}
          placeholder={t('settings.agent.argsPh')}
          value={agentEdit?.argsReview ?? ''}
          onChange={(e) => setAgentEdit((p) => p && { ...p, argsReview: e.target.value })}
        />
      </Field>

      <div className="flex items-center gap-2 pt-1">
        <button
          onClick={() => void saveAgent()}
          disabled={!agentDirty || agentBusy !== null}
          className="flex items-center gap-1 rounded-md bg-blue-600 px-3 py-1.5 text-cap font-bold text-white hover:bg-blue-700 disabled:opacity-40"
        >
          {agentBusy === 'save' ? <Loader2 size={11} className="animate-spin" /> : <Save size={11} />} 保存 agent 配置
        </button>
        {agentDirty && (
          <button onClick={() => void loadAgent()} className="rounded-md border border-slate-200 dark:border-slate-700 px-2.5 py-1.5 text-cap text-slate-400 dark:text-slate-500 hover:text-slate-600">
            放弃
          </button>
        )}
        <span className="ml-auto text-micro text-slate-300 dark:text-slate-600">
          {t('settings.agent.protocolLabel')}{agentStatus?.effective.type === 'claude' ? t('settings.agent.protocolStream') : t('settings.agent.protocolPlain')}
        </span>
      </div>
    </div>
  )
}
