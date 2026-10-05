import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { EncoderDisplayContent, KeyDisplayContent } from './DisplayContent';

// Verify the previews reflect saved content, rather than placeholder artwork.
describe('display content', () => {
  it('renders the current action label and its mute icon', () => {
    const markup = renderToStaticMarkup(<KeyDisplayContent slot={{type:'action', label:'Quiet mode', bindings:[{gesture:'pressed', action:{type:'toggle_mute'}}]}} pages={[]} depth={0} back={false} />);
    expect(markup).toContain('Quiet mode');
    expect(markup).toContain('lucide-volume-x');
  });
  it('uses the destination folder name and generated Back depth', () => {
    expect(renderToStaticMarkup(<KeyDisplayContent slot={{type:'folder', page_id:'tools'}} pages={[{id:'tools', name:'Tools', slots:[]}]} depth={1} back={false} />)).toContain('Tools');
    const back = renderToStaticMarkup(<KeyDisplayContent slot={null} pages={[]} depth={3} back />);
    expect(back).toContain('L3');
    expect(back).toContain('lucide-arrow-left');
  });
  it('shows the encoder binding and an honest unbound state', () => {
    expect(renderToStaticMarkup(<EncoderDisplayContent action={{type:'volume', delta:5}} />)).toContain('Volume +5');
    expect(renderToStaticMarkup(<EncoderDisplayContent />)).toContain('No binding');
  });
});
