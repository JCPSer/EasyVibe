import { useCallback, useEffect, useState } from 'react'
import { toast } from '@/runtime/toast'
import { t } from '@/runtime/i18n'
import { agentStatus, agentTest } from '@/api/system'
import { putSetting } from '@/api/settings'

export type AgentDetected = { command: string; path: string; version: string | null }
export type AgentState = { found: boolean | null; detected: AgentDetected[] }

/**
 * 执行 agent 状态域（M2 引导与降级）：探测 + 30s 轮询 + 「采用」端点序列 + 安装命令复制。
 * 从 App 抽出；所有时变值经参数传入，依赖数组与拆分前一致。
 */
export function useAgentState(backendOnline: boolean | null) {
  // found=null 探测中/离线——按可用处理，只有显式 false 才拦截
  const [agentState, setAgentState] = useState<AgentState>({ found: null, detected: [] })

  const loadAgentState = useCallback(() => {
    agentStatus()
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { effective?: { found: boolean }; detected?: AgentDetected[] } } | null) => {
        if (d?.data?.effective) {
          setAgentState({ found: d.data.effective.found, detected: d.data.detected ?? [] })
        }
      })
      .catch(() => {})
  }, [])

  // 后端在线后加载 agent 状态 + 30s 轮询（装好后自然恢复，无需手动刷新）
  useEffect(() => {
    if (backendOnline !== true) return
    loadAgentState()
    const t = window.setInterval(loadAgentState, 30000)
    return () => window.clearInterval(t)
  }, [backendOnline, loadAgentState])

  // M2"采用"动作的端点序列（方案 §5.2）：settings/set → test → status；任一步失败保留横幅并报原因
  const adoptAgent = useCallback(
    async (command: string) => {
      try {
        const presetId = ['claude', 'codex', 'opencode'].includes(command) ? command : 'custom'
        const put = (key: string, value: unknown) => putSetting({ scope: 'global', key, value })
        const [r1, r2] = await Promise.all([put('agent.command', command), put('agent.preset', presetId)])
        if (!r1.ok || !r2.ok) throw new Error(t('hooks.agent.writeFail'))
        const r3 = await agentTest()
        const d3 = await r3.json().catch(() => null)
        toast(
          d3?.data?.ok
            ? t('hooks.agent.adoptedOk', { command, ms: d3.data.latencyMs })
            : t('hooks.agent.adoptedFail', { command, protocol: d3?.data?.protocol ?? t('hooks.agent.unknown') }),
          d3?.data?.ok ? 'info' : 'error',
        )
        loadAgentState()
      } catch (e) {
        toast(e instanceof Error ? e.message : t('hooks.agent.adoptFail'), 'error')
      }
    },
    [loadAgentState],
  )

  const CLAUDE_INSTALL_CMD = 'npm install -g @anthropic-ai/claude-code'
  const copyInstallCmd = useCallback(() => {
    void navigator.clipboard?.writeText(CLAUDE_INSTALL_CMD)
    toast(t('hooks.agent.installCopied'))
  }, [])

  return { agentState, adoptAgent, copyInstallCmd, reload: loadAgentState }
}
