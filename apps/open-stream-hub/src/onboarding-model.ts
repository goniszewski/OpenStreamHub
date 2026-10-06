// First-run guide state: which step is shown and whether the guide was finished or skipped before.
export const onboardingStorageKey = 'open-stream-hub.onboarding-done';
export const onboardingStepIds = ['try', 'profile', 'connect', 'safety'] as const;
export type OnboardingStepId = typeof onboardingStepIds[number];
export const lastOnboardingStep = onboardingStepIds.length - 1;

export const clampStep = (step: number) => Math.min(lastOnboardingStep, Math.max(0, Math.trunc(step) || 0));

// The guide opens by itself only when this returns true. If storage cannot be read the answer is
// unknown, so the guide stays closed rather than reappearing on every launch; Help still opens it.
export function shouldAutoOpenOnboarding(storage: Pick<Storage, 'getItem'> | null, desktop: boolean): boolean {
  if (!desktop || !storage) return false;
  try { return storage.getItem(onboardingStorageKey) === null; } catch { return false; }
}

// Called only for Done or Skip, never when the dialog is merely closed by something else.
export function markOnboardingDone(storage: Pick<Storage, 'setItem'> | null) {
  try { storage?.setItem(onboardingStorageKey, 'done'); } catch { /* The guide is simply offered again next launch if storage is blocked. */ }
}
