// Crash-recovery copies of unsaved profile drafts. Pure logic over an injected
// Storage so it can be tested without a browser; every storage access is
// guarded because storage may be unavailable, full or cleared at any time.
import type { Profile } from './contracts';
import { stableStringify } from './draft-model';

export const draftKeyPrefix = 'openstreamhub.draft.';
export const newDraftKey = 'new';
export const lastOpenedKey = 'openstreamhub.last-profile';
export const maximumDraftBytes = 1024 * 1024;
export const draftWriteDelayMs = 400;

const canonicalUuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

// source_id: the saved profile the draft was opened from (kept after that profile is deleted), so a recovered draft of an
// imported dry-run-only profile still names it.
export interface StoredDraft { profile: Profile; base_modified_at_ms: number | null; saved_at: number; source_id?: string | null }

export class DraftStore {
  constructor(private readonly storage: Storage | null, private readonly now: () => number = Date.now) {}

  private access<T>(action: (storage: Storage) => T, fallback: T): T {
    if (!this.storage) return fallback;
    try { return action(this.storage); } catch { return fallback; }
  }

  read(key: string): StoredDraft | null {
    const raw = this.access(storage => storage.getItem(draftKeyPrefix + key), null);
    if (raw === null) return null;
    try {
      const value = JSON.parse(raw) as Partial<StoredDraft> | null;
      const profile = value?.profile as Profile | undefined;
      const valid = profile && typeof profile === 'object' && typeof profile.name === 'string' && Array.isArray(profile.bindings) && profile.schema_version === 2
        && (value!.base_modified_at_ms === null || typeof value!.base_modified_at_ms === 'number') && typeof value!.saved_at === 'number';
      if (valid) {
        // Only a canonical lowercase UUID can name a saved profile; anything else is dropped, never passed on as a source.
        if ('source_id' in value! && !(typeof value!.source_id === 'string' && canonicalUuid.test(value!.source_id))) delete value!.source_id;
        return value as StoredDraft;
      }
    } catch { /* Corrupt entries are dropped below. */ }
    this.clear(key);
    return null;
  }

  // Returns false when the draft was not stored (too large or storage unavailable).
  write(key: string, profile: Profile, baseModifiedAtMs: number | null, sourceId: string | null = null): 'written' | 'too-large' | 'unavailable' {
    const text = JSON.stringify({ profile, base_modified_at_ms: baseModifiedAtMs, saved_at: this.now(), ...(sourceId ? { source_id: sourceId } : {}) } satisfies StoredDraft);
    if (text.length > maximumDraftBytes) { this.clear(key); return 'too-large'; }
    return this.access(storage => { storage.setItem(draftKeyPrefix + key, text); return 'written' as const; }, 'unavailable' as const);
  }

  clear(key: string) { this.access(storage => storage.removeItem(draftKeyPrefix + key), undefined); }

  // The saved profile the editor last had open, so a refresh or a relaunch comes back to it.
  lastOpened(): string | null {
    const id = this.access(storage => storage.getItem(lastOpenedKey), null);
    return id !== null && canonicalUuid.test(id) ? id : null;
  }

  setLastOpened(id: string) {
    if (canonicalUuid.test(id) && this.lastOpened() !== id) this.access(storage => storage.setItem(lastOpenedKey, id), undefined);
  }

  keys(): string[] {
    return this.access(storage => {
      const found: string[] = [];
      for (let index = 0; index < storage.length; index += 1) {
        const name = storage.key(index);
        if (name?.startsWith(draftKeyPrefix)) found.push(name.slice(draftKeyPrefix.length));
      }
      return found;
    }, []);
  }

  // Removes drafts of profiles that no longer exist. The never-saved draft is kept.
  purge(existingIds: Iterable<string>) {
    const keep = new Set(existingIds);
    for (const key of this.keys()) if (key !== newDraftKey && !keep.has(key)) this.clear(key);
  }
}

export type RestoreDecision = { kind: 'none' } | { kind: 'offer'; changedOnDisk: boolean };

// A stored draft equal to the saved profile is stale and is not offered.
export function restoreDecision(stored: StoredDraft | null, saved: { profile: Profile; modifiedAtMs: number | null } | null): RestoreDecision {
  if (!stored) return { kind: 'none' };
  if (saved && stableStringify(stored.profile) === stableStringify(saved.profile)) return { kind: 'none' };
  return { kind: 'offer', changedOnDisk: saved !== null && stored.base_modified_at_ms !== saved.modifiedAtMs };
}

export type Cancel = () => void;
export type Schedule = (action: () => void, delayMs: number) => Cancel;
export const timeoutSchedule: Schedule = (action, delay) => { const timer = setTimeout(action, delay); return () => clearTimeout(timer); };

export interface DraftObservation { key: string; profile: Profile; baseModifiedAtMs: number | null; dirty: boolean; sourceId?: string | null }

// Write-through persistence of the current draft: debounced writes, immediate
// clears. A held key is left alone while a restore decision is pending.
export class DraftPersister {
  private current: string | null = null;
  private pending: { key: string; profile: Profile; base: number | null; source: string | null; cancel: Cancel } | null = null;
  private readonly held = new Set<string>();
  private oversizeReported = false;

  onOversize: () => void = () => {};

  constructor(readonly store: DraftStore, private readonly schedule: Schedule = timeoutSchedule) {}

  hold(key: string) { this.held.add(key); }
  release(key: string) { this.held.delete(key); }

  private drop(key: string) {
    if (this.pending?.key === key) { this.pending.cancel(); this.pending = null; }
    if (!this.held.has(key)) this.store.clear(key);
  }

  observe(next: DraftObservation) {
    if (this.current !== null && this.current !== next.key) this.drop(this.current);
    this.current = next.key;
    if (this.held.has(next.key)) return;
    if (!next.dirty) { this.drop(next.key); return; }
    this.pending?.cancel();
    const pending = { key: next.key, profile: next.profile, base: next.baseModifiedAtMs, source: next.sourceId ?? null, cancel: () => {} };
    pending.cancel = this.schedule(() => { if (this.pending === pending) this.flush(); }, draftWriteDelayMs);
    this.pending = pending;
  }

  // Writes any pending draft now (visibilitychange, pagehide, beforeunload).
  flush() {
    const pending = this.pending;
    if (!pending) return;
    pending.cancel();
    this.pending = null;
    if (this.held.has(pending.key)) return;
    if (this.store.write(pending.key, pending.profile, pending.base, pending.source) === 'too-large' && !this.oversizeReported) {
      this.oversizeReported = true;
      this.onOversize();
    }
  }

  // The user explicitly discarded the current draft.
  discardCurrent() { if (this.current !== null) this.drop(this.current); }
}
