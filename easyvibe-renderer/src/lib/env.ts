// 运行环境探测：Tauri 桌面壳 vs 浏览器（dev/preview）
export function isTauriRuntime() {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}
