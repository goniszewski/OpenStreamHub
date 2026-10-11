import { bytesToBase64 } from './artwork-model';

// Mirrors DialSource::MAX_IMAGE_BYTES in the service profile.
export const maxDialSourceBytes = 256 * 1024;

const decodedBytes = (base64: string) => Math.floor(base64.length * 3 / 4) - (base64.endsWith('==') ? 2 : base64.endsWith('=') ? 1 : 0);

// The picture to keep in the profile so its fit can be changed later: the original when it is small enough,
// otherwise a smaller JPEG drawn through a canvas. Null when it cannot be made small enough (or there is no
// canvas), in which case the picture stays adjustable only until the app closes.
export async function sourceToKeep(base64: string, mime: string, background: string): Promise<{ image: string } | null> {
  if (decodedBytes(base64) <= maxDialSourceBytes && (mime === 'image/png' || mime === 'image/jpeg')) return { image: base64 };
  try {
    const bitmap = await createImageBitmap(await (await fetch(`data:${mime};base64,${base64}`)).blob());
    for (const side of [1024, 800, 600, 480]) {
      const scale = Math.min(1, side / Math.max(bitmap.width, bitmap.height));
      const canvas = document.createElement('canvas');
      canvas.width = Math.max(1, Math.round(bitmap.width * scale));
      canvas.height = Math.max(1, Math.round(bitmap.height * scale));
      const context = canvas.getContext('2d');
      if (!context) return null;
      context.fillStyle = background;
      context.fillRect(0, 0, canvas.width, canvas.height);
      context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
      const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, 'image/jpeg', 0.85));
      if (!blob) return null;
      const encoded = bytesToBase64(new Uint8Array(await blob.arrayBuffer()));
      if (decodedBytes(encoded) <= maxDialSourceBytes) return { image: encoded };
    }
  } catch { /* no way to shrink it here */ }
  return null;
}
