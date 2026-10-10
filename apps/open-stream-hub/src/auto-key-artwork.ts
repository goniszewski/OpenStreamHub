// Key artwork made without the studio: assigning "Open application" draws the
// application's own icon, and "Open URL" draws a globe with the site's host.
// Nothing is fetched from the network (a site icon would be), and artwork the
// user has not asked for is never replaced: the plan applies only to a key
// with no artwork, or whose artwork is still the one this module drew.
import type { BindingAction, KeyArtwork, Profile } from './contracts';
import { setKeyArtwork } from './key-artwork';
import { buildKeyArtworkDesignRequest, defaultKeyArtworkDesign, designTextIssue, appIconId, storedKeyArtworkDesign } from './key-artwork-design';
import type { KeyArtworkDesign } from './key-artwork-design';
import { stableStringify } from './draft-model';

export type AutoPlan = { kind: 'app'; bundleId: string } | { kind: 'url'; host: string };

/** The host of an http(s) URL as a short key label, or null for anything else. */
export function urlHost(url: string): string | null {
  try {
    const parsed = new URL(url.trim());
    if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return null;
    const host = parsed.hostname.replace(/^www\./, '');
    return host && designTextIssue(host) === null ? host : null;
  } catch { return null; }
}

/** What to draw for a key's action, or null when nothing is drawn for it. */
export function planAutoArtwork(action?: BindingAction | null): AutoPlan | null {
  if (action?.type === 'launch_app') {
    const bundleId = action.bundle_id.trim();
    return bundleId ? { kind: 'app', bundleId } : null;
  }
  if (action?.type === 'open_url') {
    const host = urlHost(action.url);
    return host ? { kind: 'url', host } : null;
  }
  return null;
}

/** Same plan, same drawing: used to tell that nothing changed. */
export const planKey = (plan: AutoPlan | null): string | null => plan && (plan.kind === 'app' ? `app:${plan.bundleId}` : `url:${plan.host}`);

export function autoDesign(plan: AutoPlan): KeyArtworkDesign {
  return plan.kind === 'app'
    ? { ...defaultKeyArtworkDesign, iconId: appIconId(plan.bundleId) }
    : { ...defaultKeyArtworkDesign, iconId: 'globe', text: plan.host, textSize: 0 };
}

/** Whether artwork is exactly what an automatic drawing makes: its stored
    design is the plan's design for the application or host it names. This
    survives a reload and a restart, so the artwork keeps following the
    action; a design changed in any field no longer matches and stays. */
export function isAutoDrawn(artwork: KeyArtwork | undefined): boolean {
  const design = artwork?.design;
  if (!design) return false;
  const icon = design.icon_id ?? '';
  const plan: AutoPlan | null = icon.startsWith('app:') && icon.length > 4 ? { kind: 'app', bundleId: icon.slice(4) }
    : icon === 'globe' && design.text ? { kind: 'url', host: design.text } : null;
  return plan !== null && stableStringify(design) === stableStringify(storedKeyArtworkDesign(autoDesign(plan)));
}

/** Whether a key's artwork may be replaced by an automatic one: it has none,
    or it is exactly what an earlier automatic drawing made (the application
    changed since). Anything the user designed or uploaded stays. */
export function mayReplaceArtwork(artwork: KeyArtwork | undefined): boolean {
  return !artwork || isAutoDrawn(artwork);
}

export interface AutoDeps {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  /** The design as PNG (base64), as the studio paints it. */
  render(design: KeyArtworkDesign, appIconPng?: string): Promise<string>;
  /** The application's icon as PNG (base64); rejects when it cannot be read. */
  appIcon(bundleId: string): Promise<string>;
}

/** The prepared artwork for a plan, carrying its design so it reopens in the studio. */
export async function buildAutoArtwork(plan: AutoPlan, deps: AutoDeps): Promise<KeyArtwork> {
  const design = autoDesign(plan);
  const png = await deps.render(design, plan.kind === 'app' ? await deps.appIcon(plan.bundleId) : undefined);
  const built = buildKeyArtworkDesignRequest(design, png);
  if (!built.ok) throw new Error(built.error);
  const prepared = await deps.invoke<KeyArtwork>('prepare_key_artwork', { request: built.request });
  return { ...prepared, design: storedKeyArtworkDesign(design) };
}

/** What the editor knows at the moment the drawing finishes. */
export interface AutoContext {
  /** The plan the key's action has now. */
  planKey(): string | null;
  /** The key's artwork now. */
  artwork(): KeyArtwork | undefined;
  /** False once a newer request, or leaving the editor, superseded this one. */
  current(): boolean;
}

export interface AutoSession {
  getState(): { draftGeneration: number; draft: { profile: Profile } };
  edit(profile: Profile, key?: string): void;
}

/** Draws the plan and puts it on the key as one undoable edit. Returns whether
    the artwork was set. A key whose artwork or action changed meanwhile, or a
    draft that was replaced, is left alone; a failure leaves the key as it is
    (the studio is still there for the user). */
export async function applyAutoArtwork(plan: AutoPlan, deps: AutoDeps, session: AutoSession, path: readonly string[], control: number, context: AutoContext): Promise<boolean> {
  if (!mayReplaceArtwork(context.artwork())) return false;
  const generation = session.getState().draftGeneration;
  try {
    const artwork = await buildAutoArtwork(plan, deps);
    if (!context.current() || context.planKey() !== planKey(plan)) return false;
    if (!mayReplaceArtwork(context.artwork())) return false;
    const state = session.getState();
    if (state.draftGeneration !== generation) return false;
    session.edit(setKeyArtwork(state.draft.profile, path, control, artwork), 'key-artwork-auto');
    return true;
  } catch { return false; }
}
