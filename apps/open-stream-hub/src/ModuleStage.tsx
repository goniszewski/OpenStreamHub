import type { CSSProperties, DragEvent, PointerEvent } from 'react';
import { ChevronDown, ChevronUp } from 'lucide-react';
import { DialFace, KeyDisplayContent } from './DisplayContent';
import { keypadDepth, keypadIsBack } from './keypad-model';
import type { KnobOrientation } from './knob-orientation';
import type { StoredLighting } from './lighting-state';
import { litLedColours } from './lighting-state';
import type { ModuleLive } from './live-state';
import { ModuleArtImage, ModuleGlass } from './ModuleArt';
import { dialScreen, knobControls, moduleArtwork } from './module-controls';
import type { Profile } from './contracts';
import type { DeviceEditor } from './useDeviceEditor';
import { controlAction, faderFromKey } from './rig-model';
import { controlLabel, faderMaximum, modules } from './ui-model';
import type { ModuleId } from './ui-model';

// Read-only live overlays, shared by every module stage and the standalone module image.
export const livePressed = (live: ModuleLive | undefined, control: number) => live?.keysDown.includes(control) ? ' pressed' : '';
// `screen` fits the needle inside the encoder's glass: the dial's control area also covers the black ring around the screen,
// so a full-radius needle would stick out over it.
export const livePointer = (live: ModuleLive | undefined, control: number, screen = false) => live?.angles[control] === undefined ? null :
  <span className={screen ? 'live-pointer screen' : 'live-pointer'} style={{ transform: `rotate(${live.angles[control]}deg)` }} aria-hidden="true" />;
export function livePulse(live: ModuleLive | undefined, control: number) {
  const pulse = live?.pulses[control];
  return pulse && <span key={pulse.tick} className={`live-pulse ${pulse.direction}`} aria-hidden="true">{pulse.direction === 'up' ? <ChevronUp size={16} /> : <ChevronDown size={16} />}</span>;
}

export interface DropHandlers { onDragOver?: (event: DragEvent<HTMLElement>) => void; onDrop?: (event: DragEvent<HTMLElement>) => void }

// One module drawn from the draft profile with its hit areas: knob caps, fader caps (dragging one moves the
// simulated value), key displays with their artwork, the dial and its press, the roller halves. Live input is
// read-only and overrides what the art shows. Every area is a named button (or slider), so the stage works by
// keyboard; a picked stage outlines the selected control. `where` completes each area's name ("in rig",
// "on the base"). The caller owns what a pick means: this component reports it and draws the result.
export function ModuleStage({ module, where, editor, profile, live, stored, knobOrientation, rotation = 0, picked, faderValue, onSelect, onFaderPointer, onFaderValue, dropHandlers }: {
  module: ModuleId; where: string; editor: DeviceEditor; profile: Profile; live: ModuleLive | undefined; stored: StoredLighting | undefined;
  knobOrientation: KnobOrientation;
  // Degrees clockwise the base is turned: the whole stage, controls and displays included, turns with it. The 3-knob
  // module's art turns through `knobOrientation` instead.
  rotation?: 0 | 90 | 180 | 270;
  // This stage is the picked unit, so its selected control is outlined.
  picked: boolean;
  faderValue(control: number): number;
  onSelect(control: number): void;
  onFaderPointer(control: number, event: PointerEvent<HTMLElement>): void;
  onFaderValue(control: number, value: number): void;
  dropHandlers?(control: number): DropHandlers;
}) {
  const info = modules.find(entry => entry.id === module)!;
  const art = moduleArtwork[module];
  const isKnob = module === 'm003k';
  const controls = isKnob ? knobControls(knobOrientation) : art.controls;
  const selected = editor.selection.control;
  const turn = rotation !== 0 && !isKnob;
  const sideways = rotation === 90 || rotation === 270;
  // The module's screens draw upright whatever its turn (the service turns what it sends the other way), so the
  // pictures on a turned stage are turned back.
  const upright = turn ? { transform: `rotate(${360 - rotation}deg)` } : undefined;
  const content = <>
    <ModuleArtImage module={info} orientation={knobOrientation} faders={module === 'm005s' ? [0, 1, 2, 3, 4].map(faderValue) : undefined}
      knobs={isKnob ? litLedColours(stored, 3) : undefined}
      glows={module === 'm005s' ? litLedColours(stored, 5) : undefined} />
    {module === 'm131e' && <div className="single-display" style={{ left: `${dialScreen.left}%`, top: `${dialScreen.top}%`, width: `${dialScreen.width}%`, height: `${dialScreen.height}%`, ...upright }}><DialFace profile={profile} /></div>}
    {controls.map(area => {
      const action = controlAction(profile, module, area.control);
      const style = { left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` };
      const isPicked = picked && selected === area.control;
      const label = `Select ${controlLabel(module, area.control)} ${where}`;
      const drop = dropHandlers?.(area.control) ?? {};
      if (module === 'm131e') return <button key={area.control} type="button" className={`image-control round${isPicked ? ' selected' : ''}${livePressed(live, area.control)}`} style={style} aria-label={label} aria-pressed={isPicked} onClick={() => onSelect(area.control)} {...drop}>{livePointer(live, area.control, true)}</button>;
      if (module === 'm4315') {
        const page = editor.currentKeypadPage;
        return <button key={area.control} type="button" className={`image-control key-display${isPicked ? ' selected' : ''}${livePressed(live, area.control)}`} style={style} aria-label={label} aria-pressed={isPicked} onClick={() => onSelect(area.control)} {...drop}>
          <span className="key-upright" style={upright}>
            <KeyDisplayContent device key={page.id} artwork={page.artwork?.[String(area.control)]} slot={page.slots[area.control]} pages={profile.keypad?.pages ?? []} depth={keypadDepth(editor.keypadPath)} back={keypadIsBack(editor.keypadPath, area.control)} />
          </span>
        </button>;
      }
      if (module === 'm005s') {
        const value = faderValue(area.control);
        return <div key={area.control} role="slider" tabIndex={0} className={`image-control fader${action ? ' bound' : ''}${isPicked ? ' selected' : ''}${live?.faders[area.control] !== undefined ? ' live' : ''}`} style={style}
          aria-label={controlLabel(module, area.control)} aria-orientation="vertical" aria-valuemin={0} aria-valuemax={faderMaximum} aria-valuenow={value}
          onPointerDown={event => onFaderPointer(area.control, event)} onPointerMove={event => onFaderPointer(area.control, event)}
          onKeyDown={event => { const next = faderFromKey(event.key, value, faderMaximum); if (next !== null) { event.preventDefault(); onSelect(area.control); onFaderValue(area.control, next); } }} {...drop}>
        </div>;
      }
      return <button key={area.control} type="button" className={`image-control${area.round ? ' round' : ''}${action ? ' bound' : ''}${isPicked ? ' selected' : ''}${livePressed(live, area.control)}`} style={style} aria-label={label} aria-pressed={isPicked} onClick={() => onSelect(area.control)} {...drop}>
        {livePointer(live, area.control)}
        {livePulse(live, area.control)}
      </button>;
    })}
    <ModuleGlass module={module} />
  </>;
  return <div className={`rig-stage${isKnob ? ' knob-stage' : ''}${turn ? ' turned' : ''}`} style={{ '--art-aspect': turn && sideways ? 1 / art.aspect : art.aspect, '--turn-aspect': art.aspect } as CSSProperties} role="group" aria-label={`${info.name} ${where}`}>
    {turn ? <div className={`rig-stage-turn r${rotation}`}>{content}</div> : content}
  </div>;
}
