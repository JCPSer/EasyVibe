import { WelcomePage } from './WelcomePage'

/** 新手引导：首启欢迎工作台（零仓库首启自动出现；顶栏 ? 可重看） */
export function WelcomeOverlay({
  hasRepo,
  onAddRepo,
  onClose,
}: {
  hasRepo: boolean
  onAddRepo: () => Promise<boolean>
  onClose: () => void
}) {
  return <WelcomePage hasRepo={hasRepo} onAddRepo={onAddRepo} onClose={onClose} />
}
