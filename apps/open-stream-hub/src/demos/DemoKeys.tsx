import type { CSSProperties } from 'react';
import { ModuleArtImage, ModuleGlass } from '../ModuleArt';
import { moduleArtwork } from '../module-controls';
import { modules } from '../ui-model';

// The 15-key module as a demo draws it: the module's own artwork with each key showing the picture the module shows,
// so the page reflects what is on the hardware. A key can be clicked as if pressed.
export function DemoKeys({ images, label, onPress }: { images: readonly (string | null)[]; label(key: number): string; onPress?(key: number): void }) {
  const info = modules.find(entry => entry.id === 'm4315')!;
  const art = moduleArtwork.m4315;
  return <div className="rig-stage demo-keys" style={{ '--art-aspect': art.aspect } as CSSProperties} role="group" aria-label={info.name}>
    <ModuleArtImage module={info} />
    {art.controls.map(area => <button key={area.control} type="button" className="image-control key-display demo-key" disabled={!onPress}
      style={{ left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` }}
      aria-label={label(area.control)} onClick={() => onPress?.(area.control)}>
      {images[area.control] && <img src={`data:image/png;base64,${images[area.control]}`} alt="" draggable={false} />}
    </button>)}
    <ModuleGlass module="m4315" />
  </div>;
}
