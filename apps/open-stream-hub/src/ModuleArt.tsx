import { useId } from 'react';
import type { KnobOrientation } from './knob-orientation';
import { knobRotation } from './knob-orientation';
import { defaultFaderValues, fadersSvg, knobSvg, moduleGlass } from './module-art';
import { moduleImage } from './module-controls';
import type { ModuleId } from './ui-model';

// The module's artwork inside an image stage. The fader art is drawn inline, with each cap at its value and
// each glow strip lit (`glows`), so a stage that tracks the app's fader state moves the caps. The knob art is
// drawn inline only when a ring is lit (`knobs`), so the plain picture stays an image. Every other module is a
// plain image.
export function ModuleArtImage({ module, orientation, faders, knobs, glows }: {
  module: { id: ModuleId; art: string };
  orientation?: KnobOrientation;
  faders?: readonly number[];
  knobs?: readonly (string | null)[];
  glows?: readonly (string | null)[];
}) {
  const prefix = `art${useId().replace(/\W/g, '')}-`;
  if (module.id === 'm005s') return <span className="module-art-inline" aria-hidden="true" dangerouslySetInnerHTML={{ __html: fadersSvg(faders ?? defaultFaderValues, prefix, 100, glows) }} />;
  if (module.id === 'm003k' && knobs?.some(colour => colour !== null)) {
    return <span className="module-art-inline" aria-hidden="true" style={orientation ? { transform: `rotate(${knobRotation(orientation)}deg)` } : undefined}
      dangerouslySetInnerHTML={{ __html: knobSvg(knobs, prefix) }} />;
  }
  return <img {...moduleImage(module, orientation)} alt="" draggable={false} />;
}

// The glass over a display module's display content. Put it after the display and the controls, so pictures
// sit under it. It ignores the pointer.
export function ModuleGlass({ module }: { module: ModuleId }) {
  const src = moduleGlass(module);
  return src ? <img className="module-glass" src={src} alt="" draggable={false} /> : null;
}
