// The logo's square eye, drawn in the current text colour: a rounded square outline around a rounded square
// pupil with a small highlight. The menu bar icon (src-tauri/src/menubar.rs) uses the same proportions.
const box = (x: number, y: number, size: number, r: number) =>
  `M${x + r} ${y}h${size - 2 * r}a${r} ${r} 0 0 1 ${r} ${r}v${size - 2 * r}a${r} ${r} 0 0 1 ${-r} ${r}h${2 * r - size}a${r} ${r} 0 0 1 ${-r} ${-r}v${2 * r - size}a${r} ${r} 0 0 1 ${r} ${-r}z`;

export function SquareEye({ size = 23 }: { size?: number }) {
  return <svg width={size} height={size} viewBox="0 0 44 44" fill="currentColor" fillRule="evenodd" aria-hidden="true">
    <path d={`${box(2, 2, 40, 9)}${box(5.5, 5.5, 33, 6.5)}`} />
    <path d={`${box(15.5, 15.5, 13, 4)}${box(19.1, 19.1, 5.6, 1.8)}`} />
  </svg>;
}
