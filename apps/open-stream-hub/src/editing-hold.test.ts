import { describe, expect, it } from 'vitest';
import { beginEditingHold, endEditingHold, isEditing, restoreNeedsConfirm } from './editing-hold';

describe('the editing predicate', () => {
  it('holds while anything is being edited, and only then', () => {
    expect(isEditing({ unsavedChanges: false, pickerOpen: false, drawerOpen: false })).toBe(false);
    for (const signals of [
      { unsavedChanges: true, pickerOpen: false, drawerOpen: false },
      { unsavedChanges: false, pickerOpen: true, drawerOpen: false },
      { unsavedChanges: false, pickerOpen: false, drawerOpen: true },
      { unsavedChanges: true, pickerOpen: true, drawerOpen: true },
    ]) {
      expect(isEditing(signals)).toBe(true);
    }
  });
});

describe('the hold steps', () => {
  it('begins by saving Test over the Live the user chose, once', () => {
    expect(beginEditingHold(false, 'live', true)).toEqual({ held: true, save: 'test' });
    // Already held: no second save.
    expect(beginEditingHold(true, 'live', true)).toEqual({ held: true, save: null });
    expect(beginEditingHold(true, 'test', true)).toEqual({ held: true, save: null });
  });
  it('never begins with the setting off or while the mode is already Test', () => {
    expect(beginEditingHold(false, 'live', false)).toEqual({ held: false, save: null });
    expect(beginEditingHold(false, 'test', true)).toEqual({ held: false, save: null });
    expect(beginEditingHold(false, 'test', false)).toEqual({ held: false, save: null });
  });
  it('ends by restoring Live, and does nothing without a hold', () => {
    expect(endEditingHold(true)).toEqual({ held: false, save: 'live' });
    expect(endEditingHold(false)).toEqual({ held: false, save: null });
  });
});

describe('the restore rule', () => {
  it('restores in silence when every dry run was already executing at hold start', () => {
    const wereExecuting = new Set(['s1', 's2']);
    expect(restoreNeedsConfirm(wereExecuting, [])).toBe(false);
    expect(restoreNeedsConfirm(wereExecuting, [{ serviceId: 's1' }, { serviceId: 's2' }])).toBe(false);
  });
  it('asks when a dry run started during the hold would begin executing', () => {
    const wereExecuting = new Set(['s1']);
    expect(restoreNeedsConfirm(wereExecuting, [{ serviceId: 's1' }, { serviceId: 'new' }])).toBe(true);
    expect(restoreNeedsConfirm(new Set(), [{ serviceId: 'new' }])).toBe(true);
  });
});
