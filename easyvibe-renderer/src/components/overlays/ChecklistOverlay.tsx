import { OnboardingChecklist } from './OnboardingChecklist'
import type { loadOnboarding } from '@/lib/onboarding'

type OnboardingState = ReturnType<typeof loadOnboarding>

/** 新手引导：事件驱动 checklist（Linear 式；完成/关闭后不再打扰） */
export function ChecklistOverlay({
  onboarding,
  onGo,
  onDismiss,
}: {
  onboarding: OnboardingState
  onGo: (key: string) => void
  onDismiss: () => void
}) {
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-40">
      <OnboardingChecklist state={onboarding} onGo={onGo} onDismiss={onDismiss} />
    </div>
  )
}
