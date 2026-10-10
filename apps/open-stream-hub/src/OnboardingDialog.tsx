import type { ReactNode } from 'react';
import { ModalDialog } from './controls';
import { lastOnboardingStep, onboardingStepIds } from './onboarding-model';
import type { OnboardingStepId } from './onboarding-model';

const steps = (mod: string): Record<OnboardingStepId, { title: string; body: ReactNode }> => ({
  starters: {
    title: 'Start from a starter profile',
    body: <>
      <p id="onboarding-body">The first launch offers starter profiles — Mac basics, Streaming (OBS) and Editing and media. Picking one creates a saved profile with working controls; starting with an empty profile is fine too.</p>
      <ul><li>When a module is connected later, OpenStreamHub offers the starters that use it. “New from a starter” in the profile manager (Manage profiles, next to the profile switcher) opens the same chooser any time.</li><li>Starters only set volume and playback, take screenshots, run a timer and drive OBS. Nothing is typed into apps, and no app or web page is opened.</li><li>A run starts as a dry run that only logs what would happen; executing actions always asks for confirmation first.</li><li>Profiles from someone else can do much more than the starters. Only run profiles you wrote or reviewed.</li></ul>
    </>,
  },
  test: {
    title: 'Edit, test and save a profile',
    body: <>
      <p id="onboarding-body">Pick a module in the catalog and select a control on its image to edit what it does. Your changes stay unsaved until the profile is saved; the saved file is not touched before that.</p>
      <ul><li>Changes are saved automatically a moment after you stop typing, and each saved version is kept so you can restore one from the profile manager. Undo with {mod}+Z, redo with Shift+{mod}+Z.</li><li>The inspector's Test button shows what one binding would do. “Test with sample input” under the module canvas runs a built-in sample through the profile.</li><li>Tests and sample tests only show what would happen. Nothing is executed on your Mac.</li><li>If the app quits unexpectedly, your unsaved changes are offered back the next time you open it. That recovery copy stays on this Mac.</li></ul>
    </>,
  },
  connect: {
    title: 'Connect modules',
    body: <>
      <p id="onboarding-body">Plug in a module and it appears in the rig on its own (auto-detect can be turned off). The live view is read-only: it shows input and runs no actions.</p>
      <ul><li>Press Run in the header to run the open profile on a module. It defaults to a dry run, which only logs what would happen.</li><li>Execute actions really runs everything in the profile: it changes volume, sends shortcuts and media keys, opens apps and web pages, pastes text into the focused app (and can press Return after it), captures the screen to the clipboard and runs multi-step sequences. A user-started Execute run asks you to confirm and lists what the profile will do. If you turn on Run actions automatically in Settings, app-started sessions can run actions without a prompt; this option is off by default.</li><li>Only run profiles you wrote or reviewed. A profile from someone else can type and run commands in a terminal, or paste your clipboard into a web page.</li><li>macOS may ask for Accessibility or Automation permission the first time shortcuts, media keys, pasted text, app launches or volume changes execute. Allow it in System Settings only if you want that.</li></ul>
    </>,
  },
  safety: {
    title: 'Safety and privacy',
    body: <>
      <p id="onboarding-body">OpenStreamHub is an unofficial app, not affiliated with Cooler Master. Behaviour on real hardware is not yet verified.</p>
      <ul><li>Nothing is uploaded. The app sends no data over the network.</li><li>The diagnostic bundle in Settings is built locally. You decide whether to share it.</li><li>While the app is open it controls the 15-key module, the encoder dial and the knobs, and draws on the displays of the first two: a run starts on them by itself, as a dry run unless you turn on Run actions automatically in Settings. When you quit, the app gives them back their built-in animation (so far checked on the 15-key module only). To keep their built-in animation instead, set “Let OpenStreamHub drive the display” to Built-in animation in Settings.</li></ul>
    </>,
  },
});

export function OnboardingDialog({ open, step, mac = true, onStep, onSkip, onDone }: {
  open: boolean; mac?: boolean; step: number; onStep(step: number): void; onSkip(): void; onDone(): void;
}) {
  const id = onboardingStepIds[step] ?? onboardingStepIds[0];
  const all = steps(mac ? 'Cmd' : 'Ctrl');
  const content = all[id];
  const last = step === lastOnboardingStep;
  return <ModalDialog open={open} labelledBy="onboarding-title" describedBy="onboarding-body" onCancel={onSkip} className="onboarding-dialog">
    <p className="onboarding-count">Getting started · Step {step + 1} of {onboardingStepIds.length}</p>
    <ol className="onboarding-steps" aria-label="Guide progress">{onboardingStepIds.map((stepId, index) =>
      <li key={stepId} className={index < step ? 'done' : index === step ? 'current' : ''} aria-current={index === step ? 'step' : undefined}><span className="sr-only">{`Step ${index + 1}: ${all[stepId].title}${index === step ? ' (current)' : ''}`}</span></li>)}</ol>
    <h2 id="onboarding-title">{content.title}</h2>
    {content.body}
    <div className="dialog-actions">
      {!last && <button className="button" onClick={onSkip}>Skip guide</button>}
      <span className="spacer" />
      {/* Back stays rendered (disabled on the first step) so the buttons do not shift. When Back
          is disabled under focus, Next remounts on the first step and its autoFocus takes focus. */}
      <button className="button" disabled={step === 0} onClick={() => onStep(step - 1)}>Back</button>
      {last ? <button key="done" className="button primary" autoFocus onClick={onDone}>Done</button>
        : <button key={step === 0 ? 'next-first' : 'next'} className="button primary" autoFocus onClick={() => onStep(step + 1)}>Next</button>}
    </div>
  </ModalDialog>;
}
