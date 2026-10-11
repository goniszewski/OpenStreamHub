import { ChevronDown, ChevronUp } from 'lucide-react';
import { lightingDeviceLabel, lightingModuleInfo } from './lighting-model';
import type { DrawerTile } from './module-drawer-model';

// The one module picker, a drawer under the header on every page that acts on a module: a photo tile per
// module with its name, serial tail and a page-specific caption. Closed it shrinks to a single line naming
// the pick, so the page keeps its height. What a pick means is the page's business (`onPick`).
export function ModuleDrawer({ tiles, open, onToggle, onPick }: {
  tiles: DrawerTile[]; open: boolean; onToggle(): void; onPick(tile: DrawerTile): void;
}) {
  const picked = tiles.find(tile => tile.picked);
  const pickedName = picked ? lightingModuleInfo(picked.module).name : 'No module';
  const connected = tiles.filter(tile => tile.path !== null).length;
  return <section className={`module-drawer${open ? ' open' : ''}`} aria-label="Modules">
    <button type="button" className="drawer-bar" aria-expanded={open} aria-controls="module-drawer-tiles"
      data-tip={open ? 'Hide the module list' : 'Show the module list'} onClick={onToggle}>
      {open ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}
      <span className="drawer-title">Modules</span>
      <span className="drawer-summary">{open ? `${connected} connected` : `${pickedName}${picked?.serial ? ` · …${picked.serial.slice(-4)}` : ''}`}</span>
    </button>
    {open && <div id="module-drawer-tiles" className="module-strip" role="group" aria-label="Module">
      {tiles.length === 0 && <p className="fine-print">No modules are connected.</p>}
      {tiles.map(tile => {
        const info = lightingModuleInfo(tile.module);
        const label = tile.path ? lightingDeviceLabel({ path_hex: tile.path, module: tile.module, serial_number: tile.serial }) : info.name;
        return <button key={tile.key} type="button" aria-pressed={tile.picked} aria-label={label}
          className={`module-tile${tile.picked ? ' picked' : ''}${tile.marked ? ' marked' : ''}${tile.dimmed ? ' unlit' : ''}`}
          onClick={() => onPick(tile)}>
          <span className="module-tile-photo">{info.image && <img {...info.image} alt="" draggable={false} />}</span>
          <span className="module-tile-name">{info.name}</span>
          {tile.serial && <span className="module-tile-serial">…{tile.serial.slice(-4)}</span>}
          <span className="module-tile-capability">{tile.marked ? 'Included in Whole kit' : tile.caption}</span>
        </button>;
      })}
    </div>}
  </section>;
}
