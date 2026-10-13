import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { PermissionsPanel, permissionLabels, permissionState } from './PermissionsPanel';

describe('permissions panel', () => {
  it('reports what macOS says and never claims a state for Automation', () => {
    expect(permissionState('post_events', { post_events: true, listen_events: false })).toBe('granted');
    expect(permissionState('listen_events', { post_events: true, listen_events: false })).toBe('missing');
    expect(permissionState('post_events', null)).toBe('unknown');
    expect(permissionState('automation', { post_events: false, listen_events: false })).toBe('ask');
    expect(permissionLabels.ask).toBe('Asked when used');
  });

  it('lists the three permissions with their buttons on the desktop', () => {
    const markup = renderToStaticMarkup(<PermissionsPanel desktop invoke={(async () => undefined) as never} />);
    for (const title of ['Accessibility', 'Input Monitoring', 'Automation']) expect(markup).toContain(title);
    expect(markup.match(/>Open</g)).toHaveLength(3);
    expect(markup).toContain('Check again');
  });

  it('says the check needs the desktop app in the browser preview', () => {
    const markup = renderToStaticMarkup(<PermissionsPanel desktop={false} />);
    expect(markup).toContain('desktop app only');
    expect(markup).not.toContain('Check again');
    expect(markup).not.toContain('permission-list');
  });
});
