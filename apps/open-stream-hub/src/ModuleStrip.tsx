import { useSyncExternalStore } from 'react';
import { lightingCapability, lightingDeviceLabel, lightingKind, lightingModuleInfo } from './lighting-model';
import type { LightingDevice } from './lighting-model';
import { resolveSelected } from './module-selection';
import type { ModuleSelection } from './module-selection';

// The module picker shared by the pages that act on one module: a photo tile
// per connected module (the base first), with its name, a serial tail and what
// it can do. The pick lives in the selection store, so it follows the user from
// page to page instead of being chosen again on each one.
export function ModuleStrip({ devices, selection, caption = device => lightingCapability(device.module), usable = device => lightingKind(device.module) !== null }: {
  devices: LightingDevice[]; selection: ModuleSelection;
  // The line under each tile, and whether the current page can act on the module (an unusable tile is dimmed, still pickable).
  caption?: (device: LightingDevice) => string; usable?: (device: LightingDevice) => boolean;
}) {
  const state = useSyncExternalStore(selection.subscribe, selection.getState, selection.getState);
  const selected = resolveSelected(devices, state.path);
  return <div className="module-strip-bar"><div className="module-strip" role="group" aria-label="Module">
    {devices.length === 0 && <p className="fine-print">No modules are connected.</p>}
    {devices.map(device => {
      const info = lightingModuleInfo(device.module);
      const picked = device.path_hex === selected?.path_hex;
      const marked = state.marked.includes(device.path_hex);
      const lit = usable(device);
      return <button key={device.path_hex} type="button" aria-pressed={picked} aria-label={lightingDeviceLabel(device)}
        className={`module-tile${picked ? ' picked' : ''}${marked ? ' marked' : ''}${lit ? '' : ' unlit'}`}
        onClick={() => selection.select(device.path_hex)}>
        <span className="module-tile-photo">{info.image && <img {...info.image} alt="" draggable={false} />}</span>
        <span className="module-tile-name">{info.name}</span>
        {device.serial_number && <span className="module-tile-serial">…{device.serial_number.slice(-4)}</span>}
        <span className="module-tile-capability">{marked ? 'Included in Whole kit' : caption(device)}</span>
      </button>;
    })}
  </div></div>;
}
