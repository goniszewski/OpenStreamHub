// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { moduleAssignments } from './binding-model';
import type { Profile } from './contracts';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import { ModuleAssignmentsChip } from './ModuleAssignments';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; });

const profile = {
  schema_version: 5, name: 'p',
  bindings: [
    { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
    { module: 'm003k', control: 2, gesture: 'pressed', action: { type: 'toggle_mute' } },
  ],
} as unknown as Profile;
const chip = () => view!.$<HTMLButtonElement>('.assignments-chip');

describe('the assignments popup', () => {
  it('opens on press and lists each trigger with its action', () => {
    view = render(<ModuleAssignmentsChip assignments={moduleAssignments(profile, 'm003k')} moduleName="3-knob module" />);
    expect(view.$$('[role="dialog"]')).toHaveLength(0);
    view.click(chip());
    expect(chip().getAttribute('aria-expanded')).toBe('true');
    expect(view.$('[role="dialog"]').getAttribute('aria-label')).toBe('Actions assigned on 3-knob module');
    expect(view.$$('.assignments-list li')).toHaveLength(2);
  });
  it('closes on a press outside, but not on one inside', () => {
    view = render(<ModuleAssignmentsChip assignments={moduleAssignments(profile, 'm003k')} moduleName="m" />);
    view.click(chip());
    view.pointerDown(view.$('.assignments-list'));
    expect(view.$$('[role="dialog"]')).toHaveLength(1);
    view.pointerDown(document);
    expect(view.$$('[role="dialog"]')).toHaveLength(0);
  });
  it('closes on Escape and returns the focus to the chip', () => {
    view = render(<ModuleAssignmentsChip assignments={moduleAssignments(profile, 'm003k')} moduleName="m" />);
    view.click(chip());
    view.key(view.$('[role="dialog"]'), 'Escape');
    expect(view.$$('[role="dialog"]')).toHaveLength(0);
    expect(document.activeElement).toBe(chip());
  });
  it('says so when nothing is assigned', () => {
    view = render(<ModuleAssignmentsChip assignments={moduleAssignments(profile, 'm032le')} moduleName="m" />);
    view.click(chip());
    expect(view.$('[role="dialog"]').textContent).toContain('Nothing is assigned');
  });
  it('starts closed again for another module or keypad page (the parent keys the chip by it)', () => {
    view = render(<ModuleAssignmentsChip key="a" assignments={moduleAssignments(profile, 'm003k')} moduleName="m" />);
    view.click(chip());
    view.rerender(<ModuleAssignmentsChip key="b" assignments={moduleAssignments(profile, 'm003k')} moduleName="m" />);
    expect(view.$$('[role="dialog"]')).toHaveLength(0);
  });
});
