// @vitest-environment jsdom
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { render } from './dom-test';
import { QuickMapButton } from './QuickMap';
import { browserProfile } from './ui-model';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: vi.fn() }));

const flush = () => act(async () => { await Promise.resolve(); });
const saveButton = (view: ReturnType<typeof render>) => view.$$('button').find(button => button.textContent === 'Save as SVG')!;

describe('Quick map save in the desktop app', () => {
  beforeEach(() => {
    HTMLDialogElement.prototype.showModal = function showModal(this: HTMLDialogElement) { this.setAttribute('open', ''); };
    HTMLDialogElement.prototype.close = function close(this: HTMLDialogElement) { this.removeAttribute('open'); };
    vi.mocked(invoke).mockReset();
  });
  afterEach(() => { document.body.innerHTML = ''; });

  it('hands the paper SVG to the app and shows where it was written', async () => {
    vi.mocked(invoke).mockResolvedValue({ path: '/data/exports/terminal-quick-map-20261126-120000.svg', bytes: 10 });
    const view = render(<QuickMapButton profile={browserProfile} />);
    view.click(view.$('.quick-map-button'));
    view.click(saveButton(view));
    await flush();
    const [command, args] = vi.mocked(invoke).mock.calls[0] as [string, { request: { svg: string; profileName: string } }];
    expect(command).toBe('export_quick_map');
    expect(args.request.profileName).toBe(browserProfile.name);
    expect(args.request.svg.startsWith('<svg')).toBe(true);
    expect(args.request.svg).toContain('#ffffff');
    expect(view.$('[role="status"]').textContent).toBe('Saved to /data/exports/terminal-quick-map-20261126-120000.svg');
    vi.mocked(invoke).mockResolvedValue(undefined);
    view.click(view.$$('button').find(button => button.textContent === 'Show in Finder')!);
    expect(invoke).toHaveBeenLastCalledWith('reveal_quick_map', { name: 'terminal-quick-map-20261126-120000.svg' });
  });

  it('says why a save failed', async () => {
    vi.mocked(invoke).mockRejectedValue('disk full');
    const view = render(<QuickMapButton profile={browserProfile} />);
    view.click(view.$('.quick-map-button'));
    view.click(saveButton(view));
    await flush();
    expect(view.$('[role="alert"]').textContent).toBe('Could not save the Quick map: disk full');
  });
});
