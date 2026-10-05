import { useCallback, useEffect, useRef, useState } from 'react'
import type { CodeMap } from '@/types/map'
import { toast } from '@/lib/toast'

export type Repo = { id: string; name: string }

/**
 * 数据源装配域（后端探测 / 仓库管理 / 地图拉取 / 版本锚点）。
 * 从 App 抽出，行为与依赖数组逐处保持：
 * - 探测每 5s 重试（后端晚启动可达）；
 * - 404 = 尚未归纳（合法，走 MapGate 引导），非错误；
 * - serverVersionRef 跨 WS 重连保留（Y7 版本感知）。
 */
export function useBackendConnection() {
  const [map, setMap] = useState<CodeMap | null>(null)
  const [error, setError] = useState<string | null>(null)
  // 后端模式：探测 /api/repos 成功且仓库列表非空则启用；失败降级静态 demo 数据
  const [backendRepo, setBackendRepo] = useState<string | null>(null)
  const [repos, setRepos] = useState<Repo[]>([])
  // 三态显式化：null=探测中 / true=在线 / false=离线（演示数据模式）
  const [backendOnline, setBackendOnline] = useState<boolean | null>(null)
  const [reloadTick, setReloadTick] = useState(0)
  const serverVersionRef = useRef<string | null>(null)

  useEffect(() => {
    let cancelled = false
    let timer: number | undefined
    // 后端探测：/api/repos 取仓库列表；探测成功前每 5 秒重试，永不死心
    const probe = () => {
      fetch('/api/repos')
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error('no backend'))))
        .then((d: { data?: Repo[] }) => {
          if (!d.data || d.data.length === 0) {
            // 在线但零仓库：与离线显式区分（"尚未挂载"是真实状态，不是探测失败）
            setBackendOnline(true)
            if (!cancelled) timer = window.setTimeout(probe, 5000)
            return
          }
          setBackendOnline(true)
          setRepos(d.data)
          setBackendRepo((cur) => cur ?? d.data![0].id) // 保留当前选择（切换器驱动）
          // 版本感知（Y7）：仅取 version，不参与后端模式判定
          fetch('/api/health')
            .then((r) => (r.ok ? r.json() : null))
            .then((h: { data?: { version?: string } } | null) => {
              if (!cancelled && h?.data?.version) serverVersionRef.current = h.data.version
            })
            .catch(() => {})
        })
        .catch(() => {
          if (cancelled) return
          setBackendRepo(null)
          setBackendOnline(false)
          timer = window.setTimeout(probe, 5000)
        })
    }
    probe()
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [])

  useEffect(() => {
    const url = backendRepo ? `/api/repos/${backendRepo}/map` : '/data/map.json'
    fetch(url)
      .then((r) => {
        // 404 = 仓库尚未归纳——合法状态，走 MapGate 的"尚未生成"引导（开始归纳），不是错误
        if (r.status === 404 && backendRepo) return null
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        return r.json() as Promise<CodeMap>
      })
      .then((m) => {
        setError(null) // 实弹#4 前端根因：成功后必须清错误态，否则 (error && backendRepo) 恒真永远白屏等待
        if (m) setMap(m)
      })
      .catch((e) => setError(String(e)))
  }, [backendRepo, reloadTick])

  // D5 仓库管理：刷新列表（添加/移除后）；后端事实源是 /api/repos
  const refreshRepos = useCallback(() => {
    fetch('/api/repos')
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: Repo[] } | null) => {
        if (!d?.data) return
        setRepos(d.data)
        setBackendRepo((cur) => (cur && d.data!.some((r) => r.id === cur) ? cur : d.data![0]?.id ?? null))
      })
      .catch(() => {})
  }, [])

  // 添加本地仓库：桌面壳走系统目录选择器（Tauri dialog），浏览器降级为路径输入
  const addRepo = useCallback(async (): Promise<boolean> => {
    let path: string | null = null
    try {
      if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
        const { open } = await import('@tauri-apps/plugin-dialog')
        const sel = await open({ directory: true, title: '选择本地仓库目录' })
        path = typeof sel === 'string' ? sel : null
      } else {
        path = window.prompt('输入本地仓库目录的绝对路径')
      }
    } catch {
      path = window.prompt('目录选择器不可用，输入本地仓库目录的绝对路径')
    }
    if (!path?.trim()) return false
    try {
      const r = await fetch('/api/repos', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: path.trim() }),
      })
      const d = await r.json().catch(() => null)
      if (!r.ok) {
        toast(d?.error ?? '添加失败（目录不可读或已挂载）', 'error')
        return false
      }
      toast('已添加仓库，正在归纳…')
      refreshRepos()
      setBackendRepo(d.data.id)
      return true
    } catch {
      toast('添加失败（需要后端在线）', 'error')
      return false
    }
  }, [refreshRepos])

  // 重审 P1：移除仓库会杀活动会话；wipe=true 额外抹掉该仓库在本地库的全部痕迹
  const removeRepo = useCallback(async (id: string, wipe: boolean): Promise<boolean> => {
    try {
      const r = await fetch(`/api/repos/${encodeURIComponent(id)}${wipe ? '?wipe=true' : ''}`, { method: 'DELETE' })
      if (!r.ok) {
        toast('移除失败', 'error')
        return false
      }
      toast(wipe ? '已移除仓库并清除其数据' : '已移除仓库（数据保留，重新添加后可见）')
      if (id === backendRepo) setMap(null)
      refreshRepos()
      return true
    } catch {
      toast('移除失败（需要后端在线）', 'error')
      return false
    }
  }, [backendRepo, refreshRepos])

  const switchRepo = useCallback(
    (id: string) => {
      if (id === backendRepo) return
      setMap(null)
      setError(null)
      setBackendRepo(id)
    },
    [backendRepo],
  )

  return {
    map,
    error,
    backendRepo,
    repos,
    backendOnline,
    reloadTick,
    setReloadTick,
    serverVersionRef,
    refreshRepos,
    addRepo,
    removeRepo,
    switchRepo,
  }
}
