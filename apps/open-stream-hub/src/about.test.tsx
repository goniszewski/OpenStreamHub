import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { AboutPage } from './InfoPages';

describe('AboutPage', () => {
  const html = renderToStaticMarkup(createElement(AboutPage));

  it('credits the maintainer and the SDK author this project builds on', () => {
    expect(html).toContain('Robert Goniszewski');
    expect(html).toContain('Josiah Egner (projectzero7)');
  });

  it('shows the version, commit and repository this build came from', () => {
    expect(html).toContain(__BUILD_INFO__.version);
    expect(html).toContain('github.com/goniszewski/OpenStreamHub');
    expect(html).toContain('d996ede');
  });

  it('discloses the use of generative AI in reverse engineering, development and testing', () => {
    expect(html).toContain('Generative AI disclosure');
    expect(html).toContain('reverse engineering, development and testing');
  });
});
