// @vitest-environment jsdom
import { act, useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type { KeyFace, KeypadSlot, Profile, Variable } from './contracts';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import { describeEdit } from './edit-labels';
import { KeyFacesSection } from './KeyFacesSection';
import { VariablesContext } from './variables-context';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; document.body.innerHTML = ''; });

type ActionSlot = Extract<KeypadSlot, { type: 'action' }>;
const declared: Variable[] = [{ name: 'state', value: 'idle' }, { name: 'count', value: 3 }];
const working: KeyFace = { when: { variable_is: { name: 'state', value: 'working' } }, label: 'Working', background: '#d98e04' };
const done: KeyFace = { when: { variable_is: { name: 'state', value: 'done' } }, label: 'Done' };
const profileWith = (faces: KeyFace[] | undefined, variables: Variable[] = declared): Profile => ({
  schema_version: 5, name: 'p', bindings: [], ...(variables.length ? { variables } : {}),
  keypad: { pages: [{ id: 'root', name: 'Root', slots: [{ type: 'action', label: 'Accept', bindings: [{ gesture: 'pressed', action: { type: 'toggle_mute' } }], ...(faces ? { faces } : {}) }, ...Array<null>(14).fill(null)] }] },
});

interface Edit { before: Profile; after: Profile; key?: string }

// Keeps the draft the way the inspector does: every edit replaces the profile and is recorded.
function Harness({ initial, edits }: { initial: Profile; edits: Edit[] }) {
  const [profile, setProfile] = useState(initial);
  const slot = profile.keypad!.pages[0].slots[0] as ActionSlot;
  const session = { edit(next: Profile, key?: string) { edits.push({ before: profile, after: next, key }); setProfile(next); } };
  return <VariablesContext.Provider value={profile.variables ?? []}>
    <KeyFacesSection profile={profile} session={session} path={['root']} control={0} slot={slot} disabled={false} />
  </VariablesContext.Provider>;
}

const facesOf = (profile: Profile) => (profile.keypad!.pages[0].slots[0] as ActionSlot).faces;
const last = (edits: Edit[]) => edits[edits.length - 1];

// Types into a React-controlled input as a person would.
function type(input: HTMLInputElement, text: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, text);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('the Faces section in the DOM', () => {
  it('explains itself and adds a face as one labelled edit', () => {
    const edits: Edit[] = [];
    view = render(<Harness initial={profileWith(undefined)} edits={edits} />);
    expect(view.$('.key-faces').textContent).toContain('The tile shows the first face whose variable test holds');
    expect(view.$$('.key-faces-list > li')).toHaveLength(0);
    view.click(view.$('#key-face-add'));
    expect(edits).toHaveLength(1);
    expect(facesOf(last(edits).after)).toEqual([{ when: { variable_is: { name: 'state', value: 'idle' } }, background: '#c81e1e' }]);
    expect(describeEdit(last(edits).before, last(edits).after)).toBe('Changed the faces of Key 1');
    expect(view.$$('.key-faces-list > li')).toHaveLength(1);
  });

  it('edits the label of one face, coalescing the typing under one field key', () => {
    const edits: Edit[] = [];
    view = render(<Harness initial={profileWith([working, done])} edits={edits} />);
    type(view.$<HTMLInputElement>('#key-face-1-label'), 'Finished');
    type(view.$<HTMLInputElement>('#key-face-1-label'), 'Finished!');
    expect(facesOf(last(edits).after)?.[1].label).toBe('Finished!');
    expect(new Set(edits.map(edit => edit.key))).toEqual(new Set(['key-faces:root:0:1:label']));
    // Emptying the label leaves it out, so the tile's own label shows.
    type(view.$<HTMLInputElement>('#key-face-1-label'), '');
    expect(facesOf(last(edits).after)?.[1]).toEqual({ when: done.when });
    expect(view.$('#key-face-1-error').textContent).toContain('needs a label or a background');
  });

  it('switches the variable, offers "is at least" for a number, and compares with it', () => {
    const edits: Edit[] = [];
    view = render(<Harness initial={profileWith([working])} edits={edits} />);
    view.click(view.$('#key-face-0-comparison'));
    expect(view.$$('#key-face-0-comparison-list [data-value]').map(item => item.getAttribute('data-value'))).toEqual(['is']);
    view.click(view.$('#key-face-0-comparison'));
    view.click(view.$('#key-face-0-variable'));
    view.click(view.$('#key-face-0-variable-list [data-value="count"]'));
    expect(facesOf(last(edits).after)?.[0].when).toEqual({ variable_is: { name: 'count', value: 3 } });
    view.click(view.$('#key-face-0-comparison'));
    view.click(view.$('#key-face-0-comparison-list [data-value="at_least"]'));
    expect(facesOf(last(edits).after)?.[0].when).toEqual({ variable_at_least: { name: 'count', value: 3 } });
    expect(view.$$('#key-face-0-error')).toHaveLength(0);
  });

  it('turns the background off and on', () => {
    const edits: Edit[] = [];
    view = render(<Harness initial={profileWith([working])} edits={edits} />);
    expect(view.$$('.key-faces-list .colour-picker')).toHaveLength(1);
    view.click(view.$('#key-face-0-background-on'));
    expect(facesOf(last(edits).after)?.[0]).toEqual({ when: working.when, label: 'Working' });
    expect(view.$$('.key-faces-list .colour-picker')).toHaveLength(0);
    view.click(view.$('#key-face-0-background-on'));
    expect(facesOf(last(edits).after)?.[0].background).toBe('#c81e1e');
  });

  it('moves and removes faces, and keeps the field off the tile once none is left', () => {
    const edits: Edit[] = [];
    view = render(<Harness initial={profileWith([working, done])} edits={edits} />);
    expect(view.$<HTMLButtonElement>('[aria-label="Move face 1 up"]').disabled).toBe(true);
    expect(view.$<HTMLButtonElement>('[aria-label="Move face 2 down"]').disabled).toBe(true);
    view.click(view.$('[aria-label="Move face 2 up"]'));
    expect(facesOf(last(edits).after)).toEqual([done, working]);
    expect(last(edits).key).toBeUndefined();
    view.click(view.$('[aria-label="Remove face 1"]'));
    expect(facesOf(last(edits).after)).toEqual([working]);
    view.click(view.$('[aria-label="Remove face 1"]'));
    expect(last(edits).after.keypad!.pages[0].slots[0]).not.toHaveProperty('faces');
  });

  it('stops adding at eight faces', () => {
    const edits: Edit[] = [];
    view = render(<Harness initial={profileWith(Array(7).fill(working))} edits={edits} />);
    expect(view.$<HTMLButtonElement>('#key-face-add').disabled).toBe(false);
    view.click(view.$('#key-face-add'));
    expect(facesOf(last(edits).after)).toHaveLength(8);
    expect(view.$<HTMLButtonElement>('#key-face-add').disabled).toBe(true);
    expect(view.$('.key-faces').textContent).toContain('A key holds at most 8 faces.');
  });

  it('keeps a face that tests a group, and offers the variables link when none is declared', () => {
    const grouped: KeyFace = { when: { all: [{ variable_is: { name: 'state', value: 'a' } }, { variable_at_least: { name: 'count', value: 1 } }] }, label: 'Both' };
    view = render(<Harness initial={profileWith([grouped])} edits={[]} />);
    expect(view.$('.key-faces-list [role="note"]').textContent).toContain('tests several conditions together');
    expect(view.$$('#key-face-0-variable')).toHaveLength(0);
    view.unmount();
    view = render(<Harness initial={profileWith([{ when: { variable_is: { name: '', value: false } }, label: 'X' }], [])} edits={[]} />);
    expect(view.$('.key-faces-list [role="note"]').textContent).toContain('No variable is declared yet.');
    expect(view.$('.key-faces-list .link-button').textContent).toBe('Add one under Variables');
    expect(view.$('#key-face-0-error').textContent).toContain('not declared');
  });
});
