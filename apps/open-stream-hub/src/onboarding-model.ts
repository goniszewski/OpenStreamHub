// First-run guide state: which step is shown. The guide never opens by itself
// any more — the starter chooser is the first-run experience — and stays
// reachable from Help in the sidebar and from Settings.
export const onboardingStepIds = ['starters', 'test', 'connect', 'safety'] as const;
export type OnboardingStepId = typeof onboardingStepIds[number];
export const lastOnboardingStep = onboardingStepIds.length - 1;

export const clampStep = (step: number) => Math.min(lastOnboardingStep, Math.max(0, Math.trunc(step) || 0));
