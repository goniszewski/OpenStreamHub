// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import { SettingsToc } from './SettingsToc';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; });

const page = (headings: string[]) => <section>{headings.map(text => <section key={text} className="panel"><h2>{text}</h2></section>)}<SettingsToc label="On this page" defaultGroup="Device" groups={[{ label: 'Integrations', headings: ['OBS Studio'] }, { label: 'App', headings: ['Updates'] }]} /></section>;

describe('the settings contents list', () => {
  it('groups the sections under labels, keeping page order and sending unlisted ones to the default group', () => {
    view = render(page(['Current mode', 'OBS Studio', 'Appearance', 'Updates']));
    expect(view.$$('.settings-toc-label').map(node => node.textContent)).toEqual(['Device', 'Integrations', 'App']);
    const groups = view.$$('.settings-toc-group').map(group => [...group.querySelectorAll('a')].map(link => link.textContent));
    expect(groups).toEqual([['Current mode', 'Appearance'], ['OBS Studio'], ['Updates']]);
  });
  it('leaves out a group with no section and marks the clicked entry active', () => {
    view = render(page(['Current mode', 'Updates']));
    expect(view.$$('.settings-toc-label').map(node => node.textContent)).toEqual(['Device', 'App']);
    const link = view.$$<HTMLAnchorElement>('.settings-toc a')[1];
    view.click(link);
    expect(link.getAttribute('aria-current')).toBe('location');
  });
});
