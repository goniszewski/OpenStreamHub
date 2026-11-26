import { useMemo, useState, type ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { Copy, Download, FolderOpen, Map as MapIcon, X } from 'lucide-react';
import type { Profile, ServiceModule } from './contracts';
import { ModalDialog } from './controls';
import { useT } from './i18n-react';
import { clipKeyLabel, quickMap, wrapText, type LegendLine, type MapControl, type MapModule, type QuickMap } from './quick-map-model';

// A blueprint-style drawing of the open profile: line art of the modules it uses, a numbered callout on every bound control
// and the legend underneath, all in one SVG so that saving it keeps everything.
export interface Palette { ink: string; paper: string; muted: string; accent: string }
// On screen the drawing follows the app theme; a saved copy is always dark ink on white paper.
export const screenPalette: Palette = { ink: 'var(--text)', paper: 'var(--panel)', muted: 'var(--muted)', accent: 'var(--accent)' };
export const paperPalette: Palette = { ink: '#15151c', paper: '#ffffff', muted: '#5b6074', accent: '#4a24c9' };

const width = 1000;
const margin = 20;
const gap = 20;
const sizes: Record<ServiceModule, { w: number; h: number }> = {
  m4315: { w: 470, h: 262 }, m131e: { w: 210, h: 262 }, m003k: { w: 220, h: 262 }, m005s: { w: 470, h: 250 }, m032le: { w: 250, h: 250 },
};
const top = 78;
const font = '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';

interface Placed { module: MapModule; x: number; y: number }
// Modules flow left to right and wrap when the next one no longer fits.
export function placeModules(modules: MapModule[]): { placed: Placed[]; bottom: number } {
  const placed: Placed[] = [];
  let x = margin, y = top, rowHeight = 0;
  for (const module of modules) {
    const { w, h } = sizes[module.module];
    if (x > margin && x + w > width - margin) { x = margin; y += rowHeight + gap + 18; rowHeight = 0; }
    placed.push({ module, x, y });
    x += w + gap; rowHeight = Math.max(rowHeight, h);
  }
  return { placed, bottom: y + rowHeight };
}

const legendWrap = 62;
interface LegendLayout { columns: { line: LegendLine; text: string[]; y: number }[][]; height: number }
const legendItemHeight = (text: string[]) => 20 + text.length * 16;
export function layoutLegend(legend: LegendLine[]): LegendLayout {
  const items = legend.map(line => ({ line, text: wrapText(line.text, legendWrap) }));
  const total = items.reduce((sum, item) => sum + legendItemHeight(item.text), 0);
  const columns: LegendLayout['columns'] = [[], []];
  let used = 0;
  const heights = [0, 0];
  for (const item of items) {
    const column = used >= total / 2 && columns[0].length > 0 ? 1 : 0;
    columns[column].push({ ...item, y: heights[column] });
    heights[column] += legendItemHeight(item.text);
    used += legendItemHeight(item.text);
  }
  return { columns, height: Math.max(heights[0], heights[1]) };
}

function Callout({ n, x, y, palette }: { n: number | null; x: number; y: number; palette: Palette }) {
  if (n === null) return null;
  return <g aria-hidden="true"><circle cx={x} cy={y} r={11} fill={palette.paper} stroke={palette.accent} strokeWidth={1.6} />
    <text x={x} y={y + 4.2} textAnchor="middle" fontSize={12} fontWeight={700} fill={palette.accent}>{n}</text></g>;
}

// A thin line from a control to its callout, the way a patent drawing points at a part.
const Leader = ({ x1, y1, x2, y2, palette }: { x1: number; y1: number; x2: number; y2: number; palette: Palette }) =>
  <line x1={x1} y1={y1} x2={x2} y2={y2} stroke={palette.accent} strokeWidth={1} strokeDasharray="3 2" />;

function Frame({ x, y, module, palette, children }: { x: number; y: number; module: MapModule; palette: Palette; children: ReactNode }) {
  const { w, h } = sizes[module.module];
  return <g transform={`translate(${x} ${y})`}>
    <rect width={w} height={h} rx={10} fill="none" stroke={palette.ink} strokeWidth={1.4} />
    <rect x={5} y={5} width={w - 10} height={h - 10} rx={7} fill="none" stroke={palette.muted} strokeWidth={0.6} />
    <text x={12} y={-7} fontSize={12} fontWeight={700} letterSpacing={1.2} fill={palette.ink}>{module.title.toUpperCase()}</text>
    {children}
  </g>;
}

const find = (module: MapModule, control: number): MapControl | undefined => module.controls.find(item => item.control === control);

function Keypad({ module, palette }: { module: MapModule; palette: Palette }) {
  return <>{Array.from({ length: 15 }, (_, index) => {
    const control = find(module, index)!;
    const x = 26 + (index % 5) * 87, y = 22 + Math.floor(index / 5) * 78;
    const used = control.number !== null;
    return <g key={index}>
      <rect x={x} y={y} width={76} height={64} rx={7} fill="none" stroke={palette.ink} strokeWidth={used ? 1.4 : 0.8} strokeDasharray={used ? undefined : '4 3'} />
      {control.folder && <path d={`M${x + 8} ${y + 14} h14 l4 5 h18 v20 h-36 z`} fill="none" stroke={palette.ink} strokeWidth={1.2} />}
      {(control.keyLabel ?? '') && <text x={x + 38} y={y + 55} textAnchor="middle" fontSize={11} fill={palette.ink}>{clipKeyLabel(control.keyLabel!, 11)}</text>}
      <text x={x + 68} y={y + 12} textAnchor="end" fontSize={8} fill={palette.muted}>{index + 1}</text>
      <Callout n={control.number} x={x + 4} y={y + 4} palette={palette} />
    </g>;
  })}</>;
}

function Dial({ module, palette }: { module: MapModule; palette: Palette }) {
  const control = module.controls[0];
  return <>
    <circle cx={105} cy={125} r={76} fill="none" stroke={palette.ink} strokeWidth={1.4} />
    <circle cx={105} cy={125} r={58} fill="none" stroke={palette.ink} strokeWidth={0.8} />
    <circle cx={105} cy={125} r={34} fill="none" stroke={palette.muted} strokeWidth={0.8} />
    {Array.from({ length: 24 }, (_, index) => { const a = index * Math.PI / 12; return <line key={index} x1={105 + Math.cos(a) * 58} y1={125 + Math.sin(a) * 58} x2={105 + Math.cos(a) * 76} y2={125 + Math.sin(a) * 76} stroke={palette.ink} strokeWidth={0.7} />; })}
    <path d="M60 52 a60 60 0 0 1 90 0" fill="none" stroke={palette.accent} strokeWidth={1} markerEnd="url(#qm-arrow)" />
    <Leader x1={105} y1={125} x2={190} y2={30} palette={palette} />
    <Callout n={control.number} x={190} y={30} palette={palette} />
  </>;
}

function Knobs({ module, palette }: { module: MapModule; palette: Palette }) {
  // Seen from the front: Knob 3 on top, Knob 1 bottom-left, Knob 2 bottom-right.
  const spots = [{ x: 62, y: 190 }, { x: 158, y: 190 }, { x: 110, y: 78 }];
  return <>{spots.map((spot, index) => {
    const control = find(module, index)!;
    return <g key={index}>
      <circle cx={spot.x} cy={spot.y} r={34} fill="none" stroke={palette.ink} strokeWidth={1.4} />
      <circle cx={spot.x} cy={spot.y} r={24} fill="none" stroke={palette.muted} strokeWidth={0.8} />
      <line x1={spot.x} y1={spot.y - 24} x2={spot.x} y2={spot.y - 8} stroke={palette.ink} strokeWidth={1.6} />
      <text x={spot.x} y={spot.y + 46} textAnchor="middle" fontSize={9} fill={palette.muted}>{index + 1}</text>
      <Leader x1={spot.x + 20} y1={spot.y - 20} x2={spot.x + 36} y2={spot.y - 36} palette={palette} />
      <Callout n={control.number} x={spot.x + 38} y={spot.y - 38} palette={palette} />
    </g>;
  })}</>;
}

function Faders({ module, palette }: { module: MapModule; palette: Palette }) {
  return <>{Array.from({ length: 5 }, (_, index) => {
    const control = find(module, index)!;
    const x = 52 + index * 90;
    return <g key={index}>
      <rect x={x - 7} y={34} width={14} height={150} rx={7} fill="none" stroke={palette.ink} strokeWidth={1.2} />
      {Array.from({ length: 6 }, (_, tick) => <line key={tick} x1={x + 14} y1={34 + tick * 30} x2={x + 21} y2={34 + tick * 30} stroke={palette.muted} strokeWidth={0.7} />)}
      <rect x={x - 18} y={92} width={36} height={22} rx={4} fill={palette.paper} stroke={palette.ink} strokeWidth={1.4} />
      <line x1={x - 18} y1={103} x2={x + 18} y2={103} stroke={palette.ink} strokeWidth={0.8} />
      <text x={x} y={206} textAnchor="middle" fontSize={9} fill={palette.muted}>{index + 1}</text>
      <Callout n={control.number} x={x} y={20} palette={palette} />
    </g>;
  })}</>;
}

function Rollers({ module, palette }: { module: MapModule; palette: Palette }) {
  // Roller 1 is the right roller, Roller 2 the left one.
  const spots = [{ x: 170, index: 0 }, { x: 80, index: 1 }];
  return <>{spots.map(spot => {
    const control = find(module, spot.index)!;
    return <g key={spot.index}>
      <rect x={spot.x - 28} y={50} width={56} height={130} rx={14} fill="none" stroke={palette.ink} strokeWidth={1.4} />
      {Array.from({ length: 9 }, (_, line) => <line key={line} x1={spot.x - 28} y1={68 + line * 13} x2={spot.x + 28} y2={68 + line * 13} stroke={palette.muted} strokeWidth={0.7} />)}
      <path d={`M${spot.x} 40 l-6 8 h12 z`} fill={palette.ink} /><path d={`M${spot.x} 192 l-6 -8 h12 z`} fill={palette.ink} />
      <text x={spot.x} y={216} textAnchor="middle" fontSize={9} fill={palette.muted}>{spot.index + 1}</text>
      <Callout n={control.number} x={spot.x} y={24} palette={palette} />
    </g>;
  })}</>;
}

const drawings: Record<ServiceModule, (props: { module: MapModule; palette: Palette }) => ReactNode> = { m4315: Keypad, m131e: Dial, m003k: Knobs, m005s: Faders, m032le: Rollers };

export function QuickMapDrawing({ map, palette = screenPalette }: { map: QuickMap; palette?: Palette }) {
  const t = useT();
  const { placed, bottom } = placeModules(map.modules);
  const legend = layoutLegend(map.legend);
  const legendTop = bottom + 44;
  const height = legendTop + 28 + legend.height + 40;
  const label = t('quickMap.drawingLabel', { name: map.profileName });
  const description = map.legend.map(line => `${line.number}. ${line.name}: ${line.text}`).join(' ');
  const empty = map.modules.length === 0;
  return <svg xmlns="http://www.w3.org/2000/svg" role="img" aria-labelledby="qm-title qm-desc" viewBox={`0 0 ${width} ${empty ? 160 : height}`} width="100%" fontFamily={font}
    style={{ color: palette.ink, background: palette.paper, display: 'block' }}>
    <title id="qm-title">{label}</title><desc id="qm-desc">{empty ? t('quickMap.empty') : description}</desc>
    <defs><marker id="qm-arrow" viewBox="0 0 8 8" refX={6} refY={4} markerWidth={7} markerHeight={7} orient="auto"><path d="M0 0 L8 4 L0 8 z" fill={palette.accent} /></marker></defs>
    <rect x={8} y={8} width={width - 16} height={(empty ? 160 : height) - 16} rx={6} fill="none" stroke={palette.ink} strokeWidth={1.6} />
    <text x={margin + 6} y={42} fontSize={22} fontWeight={700} letterSpacing={2} fill={palette.ink}>{t('quickMap.title').toUpperCase()}</text>
    <text x={width - margin - 6} y={42} textAnchor="end" fontSize={16} fill={palette.ink}>{map.profileName}</text>
    <line x1={margin} y1={56} x2={width - margin} y2={56} stroke={palette.ink} strokeWidth={0.8} />
    {empty && <text x={width / 2} y={110} textAnchor="middle" fontSize={14} fill={palette.muted}>{t('quickMap.empty')}</text>}
    {placed.map(item => <Frame key={item.module.module} x={item.x} y={item.y} module={item.module} palette={palette}>{drawings[item.module.module]({ module: item.module, palette })}</Frame>)}
    {!empty && <g transform={`translate(${margin} ${legendTop})`}>
      <text y={0} fontSize={12} fontWeight={700} letterSpacing={1.2} fill={palette.ink}>{t('quickMap.legend').toUpperCase()}</text>
      <line x1={0} y1={8} x2={width - 2 * margin} y2={8} stroke={palette.ink} strokeWidth={0.8} />
      {legend.columns.map((column, columnIndex) => <g key={columnIndex} transform={`translate(${columnIndex * 490} 28)`}>
        {column.map(item => <g key={item.line.number} transform={`translate(0 ${item.y})`}>
          <circle cx={11} cy={0} r={11} fill={palette.paper} stroke={palette.accent} strokeWidth={1.6} />
          <text x={11} y={4.2} textAnchor="middle" fontSize={12} fontWeight={700} fill={palette.accent}>{item.line.number}</text>
          <text x={30} y={4} fontSize={13} fontWeight={700} fill={palette.ink}>{clipKeyLabel(item.line.name, 52)}{item.line.folder ? ' ▸' : ''}</text>
          {item.text.map((text, row) => <text key={row} x={30} y={21 + row * 16} fontSize={12.5} fill={palette.ink}>{text}</text>)}
        </g>)}
      </g>)}
    </g>}
    {!empty && <text x={width - margin - 6} y={(height) - 20} textAnchor="end" fontSize={10} fill={palette.muted}>{t('quickMap.footer', { count: map.legend.length })}</text>}
  </svg>;
}

// The standalone file: the same drawing on white paper, with its own dark ink so it reads the same anywhere.
export function quickMapSvg(map: QuickMap): string {
  return renderToStaticMarkup(<QuickMapDrawing map={map} palette={paperPalette} />);
}
export const quickMapFileName = (name: string) => `${name.replace(/[^\p{L}\p{N}]+/gu, '-').replace(/^-+|-+$/g, '').toLowerCase() || 'profile'}-quick-map.svg`;

export function QuickMapButton({ profile }: { profile: Profile }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  // Where the last save went, or why it failed.
  const [saved, setSaved] = useState<{ ok: boolean; text: string; file?: string } | null>(null);
  // App renders often (device feed, run state); the map only changes with the profile.
  const map = useMemo(() => quickMap(profile), [profile]);
  const save = async () => {
    // The desktop webview cannot download a blob, so the app writes the file (into its exports folder) and says where.
    if (isTauri()) {
      try {
        const result = await invoke<{ path: string }>('export_quick_map', { request: { svg: quickMapSvg(map), profileName: profile.name } });
        setSaved({ ok: true, text: t('quickMap.saved', { path: result.path }), file: result.path.split('/').pop() });
      } catch (reason) { setSaved({ ok: false, text: t('quickMap.saveFailed', { reason: reason instanceof Error ? reason.message : String(reason) }) }); }
      return;
    }
    const url = URL.createObjectURL(new Blob([quickMapSvg(map)], { type: 'image/svg+xml' }));
    const link = document.createElement('a');
    link.href = url; link.download = quickMapFileName(profile.name);
    document.body.appendChild(link); link.click(); link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  const reveal = (file: string) => { invoke('reveal_quick_map', { name: file }).catch((reason: unknown) => setSaved({ ok: false, text: t('quickMap.saveFailed', { reason: reason instanceof Error ? reason.message : String(reason) }) })); };
  const copy = () => { void navigator.clipboard?.writeText(quickMapSvg(map)).then(() => { setCopied(true); setTimeout(() => setCopied(false), 2000); }).catch(() => {}); };
  return <>
    <button type="button" className="button compact quick-map-button" aria-haspopup="dialog" title={t('quickMap.openTip')} onClick={() => setOpen(true)}><MapIcon size={16} aria-hidden="true" />{t('quickMap.open')}</button>
    <ModalDialog open={open} labelledBy="quick-map-title" onCancel={() => { setOpen(false); setSaved(null); }} className="quick-map-dialog">
      <div className="quick-map-bar">
        <h2 id="quick-map-title">{t('quickMap.title')}</h2>
        <button type="button" className="button compact" disabled={map.modules.length === 0} onClick={() => void save()}><Download size={15} aria-hidden="true" />{t('quickMap.save')}</button>
        <button type="button" className="button compact" disabled={map.modules.length === 0} onClick={copy}><Copy size={15} aria-hidden="true" />{copied ? t('quickMap.copied') : t('quickMap.copy')}</button>
        <span className="spacer" />
        <button type="button" className="button compact" autoFocus aria-label={t('quickMap.close')} onClick={() => { setOpen(false); setSaved(null); }}><X size={15} aria-hidden="true" />{t('quickMap.close')}</button>
      </div>
      {saved && <div className="quick-map-saved-row">
        <p className={saved.ok ? 'fine-print quick-map-saved' : 'field-error quick-map-saved'} role={saved.ok ? 'status' : 'alert'}>{saved.text}</p>
        {saved.ok && saved.file && <button type="button" className="button compact" onClick={() => reveal(saved.file!)}><FolderOpen size={15} aria-hidden="true" />{t('quickMap.reveal')}</button>}
      </div>}
      <div className="quick-map-sheet" role="region" aria-label={t('quickMap.title')} tabIndex={0}>{open && <QuickMapDrawing map={map} />}</div>
    </ModalDialog>
  </>;
}
