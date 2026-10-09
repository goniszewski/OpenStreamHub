import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { FakeStorage } from './fake-storage';
import { OnboardingDialog } from './OnboardingDialog';
import { SettingsPage } from './InfoPages';
import { clampStep, lastOnboardingStep, markOnboardingDone, onboardingStepIds, onboardingStorageKey, shouldAutoOpenOnboarding } from './onboarding-model';

const render = (step: number, mac = true) => renderToStaticMarkup(<OnboardingDialog open step={step} mac={mac} onStep={() => {}} onSkip={() => {}} onDone={() => {}} />);
const headings = ['Try it without hardware', 'Edit and save a profile', 'Connect modules', 'Safety and privacy'];

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
  it('states the permission and privacy facts', () => {
    expect(render(2)).toContain('Accessibility or Automation');
    expect(render(2)).toContain('app-started sessions can run actions without a prompt');
    expect(render(2)).toContain('this option is off by default');
    expect(render(2)).toContain('pastes text into the focused app (and can press Return after it)');
    expect(render(2)).toContain('Only run profiles you wrote or reviewed.');
    expect(render(3)).toContain('Nothing is uploaded');
    expect(render(3)).toContain('set Module control to Built-in animation in Settings');
    expect(render(1, false)).toContain('Ctrl+S');
  });
  it('keeps step numbers in range', () => {
    expect(onboardingStepIds).toHaveLength(4);
    expect([clampStep(-3), clampStep(2), clampStep(9), clampStep(NaN)]).toEqual([0, 2, 3, 0]);
  });
});

describe('onboarding persistence', () => {
  it('opens on first launch, and not again once Done or Skip stored the flag', () => {
    const storage = new FakeStorage();
    expect(shouldAutoOpenOnboarding(storage, true)).toBe(true);
    expect(shouldAutoOpenOnboarding(storage, true)).toBe(true); // merely viewing does not store anything
    markOnboardingDone(storage);
    expect(storage.getItem(onboardingStorageKey)).toBe('done');
    expect(shouldAutoOpenOnboarding(storage, true)).toBe(false);
  });
  it('never opens by itself in the browser preview', () => {
    expect(shouldAutoOpenOnboarding(new FakeStorage(), false)).toBe(false);
  });
  it('does not auto-open or throw when storage is unavailable', () => {
    const broken = { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); } };
    expect(shouldAutoOpenOnboarding(broken, true)).toBe(false);
    expect(shouldAutoOpenOnboarding(null, true)).toBe(false);
    expect(() => markOnboardingDone(broken)).not.toThrow();
    expect(() => markOnboardingDone(null)).not.toThrow();
  });
});

describe('Settings entries', () => {
  it('offers the guide and the connection notices toggle', () => {
    const markup = renderToStaticMarkup(<SettingsPage desktop themePicker={null} storagePath="" noticesEnabled onNoticesEnabled={() => {}} onShowGuide={() => {}} />);
    expect(markup).toContain('Show the getting-started guide');
    expect(markup).toContain('Show connection notices');
    expect(markup).toMatch(/type="checkbox"[^>]*checked/);
    expect(renderToStaticMarkup(<SettingsPage desktop themePicker={null} storagePath="" noticesEnabled={false} />)).not.toMatch(/type="checkbox"[^>]*checked/);
  });
});
