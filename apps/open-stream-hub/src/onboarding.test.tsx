import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { OnboardingDialog } from './OnboardingDialog';
import { clampStep, lastOnboardingStep, onboardingStepIds } from './onboarding-model';
import { SettingsPage } from './InfoPages';

const render = (step: number, mac = true) => renderToStaticMarkup(<OnboardingDialog open step={step} mac={mac} onStep={() => {}} onSkip={() => {}} onDone={() => {}} />);
const headings = ['Start from a starter profile', 'Edit, test and save a profile', 'Connect modules', 'Safety and privacy'];

describe('onboarding steps', () => {
  it('renders each step heading with an accessible step indicator', () => {
    headings.forEach((heading, step) => {
      const markup = render(step);
      expect(markup).toContain(`<h2 id="onboarding-title">${heading}</h2>`);
      expect(markup).toContain(`Step ${step + 1} of 4`);
      expect(markup.match(/aria-current="step"/g)).toHaveLength(1);
      expect(markup).toContain('aria-labelledby="onboarding-title"');
    });
  });
  it('shows Next and Skip on early steps, Back disabled only on the first, and Done only on the last', () => {
    expect(render(0)).toContain('>Next<');
    expect(render(0)).toContain('>Skip guide<');
    expect(render(0)).toContain('<button class="button" disabled="">Back</button>');
    expect(render(1)).toContain('<button class="button">Back</button>');
    expect(render(lastOnboardingStep)).toContain('<button class="button">Back</button>');
    const last = render(lastOnboardingStep);
    expect(last).toContain('>Done<');
    expect(last).not.toContain('>Next<');
    expect(last).not.toContain('>Skip guide<');
  });
  it('describes the starter flow on the first step', () => {
    const markup = render(0);
    expect(markup).toContain('Mac basics, Streaming (OBS) and Editing and media');
    expect(markup).toContain('offers the starters that use it');
    expect(markup).toContain('New from a starter');
    expect(markup).toContain('Nothing is typed into apps, and no app or web page is opened');
    expect(markup).toContain('executing actions always asks for confirmation first');
    expect(markup).toContain('Only run profiles you wrote or reviewed.');
  });
  it('merges editing, testing and saving on the second step', () => {
    const markup = render(1);
    expect(markup).toContain('saved automatically a moment after you stop typing');
    expect(markup).toContain('“Test with sample input”');
    expect(markup).toContain('Nothing is executed on your Mac.');
    expect(markup).toContain('offered back the next time you open it');
    expect(render(1, false)).toContain('Ctrl+Z');
  });
  it('states the permission and privacy facts', () => {
    expect(render(2)).toContain('Accessibility or Automation');
    expect(render(2)).toContain('app-started sessions can run actions without a prompt');
    expect(render(2)).toContain('this option is off by default');
    expect(render(2)).toContain('pastes text into the focused app (and can press Return after it)');
    expect(render(2)).toContain('Only run profiles you wrote or reviewed.');
    expect(render(3)).toContain('Nothing is uploaded');
    expect(render(3)).toContain('set “Let OpenStreamHub drive the display” to Built-in animation in Settings');
  });
  it('keeps step numbers in range', () => {
    expect(onboardingStepIds).toHaveLength(4);
    expect([clampStep(-3), clampStep(2), clampStep(9), clampStep(NaN)]).toEqual([0, 2, 3, 0]);
  });
});

describe('Settings entries', () => {
  it('offers the guide, the connection notices toggle and the automatic-saving toggle', () => {
    const markup = renderToStaticMarkup(<SettingsPage desktop themePicker={null} storagePath="" noticesEnabled onNoticesEnabled={() => {}} onShowGuide={() => {}} />);
    expect(markup).toContain('Show the getting-started guide');
    expect(markup).toContain('Show connection notices');
    expect(markup).toContain('Save changes automatically');
    expect(markup).toMatch(/type="checkbox"[^>]*checked/);
    expect(renderToStaticMarkup(<SettingsPage desktop themePicker={null} storagePath="" noticesEnabled={false} autosave={false} />)).not.toMatch(/type="checkbox"[^>]*checked/);
    // The toggle exists only in the desktop app.
    expect(renderToStaticMarkup(<SettingsPage desktop={false} themePicker={null} storagePath="" />)).not.toContain('Save changes automatically');
  });
});
