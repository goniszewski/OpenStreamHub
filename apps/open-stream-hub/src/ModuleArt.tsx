import { useId } from 'react';
import type { KnobOrientation } from './knob-orientation';
import { defaultFaderValues, fadersSvg, moduleGlass } from './module-art';
import { moduleImage } from './module-controls';
import type { ModuleId } from './ui-model';

// The module's artwork inside an image stage. The fader art is drawn inline, with each cap at its value, so a
// stage that tracks the app's fader state moves the caps. Every other module is a plain image.
export function ModuleArtImage({ module, orientation, faders }: { module: { id: ModuleId; art: string }; orientation?: KnobOrientation; faders?: readonly number[] }) {
  const prefix = `art${useId().replace(/\W/g, '')}-`;
  if (module.id === 'm005s') return <span className="module-art-inline" aria-hidden="true" dangerouslySetInnerHTML={{ __html: fadersSvg(faders ?? defaultFaderValues, prefix) }} />;
  return <img {...moduleImage(module, orientation)} alt="" draggable={false} />;
}

// The glass over a display module's display content. Put it after the display and the controls, so pictures
// sit under it. It ignores the pointer.
export function ModuleGlass({ module }: { module: ModuleId }) {
  const src = moduleGlass(module);
  return src ? <img className="module-glass" src={src} alt="" draggable={false} /> : null;
}
