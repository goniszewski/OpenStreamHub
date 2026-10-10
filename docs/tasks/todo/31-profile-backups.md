# 31 · Automatic profile backups and restore previous version

**Priority:** P1 · **Effort:** S–M · **Hardware needed:** no

## Why
APH Networks lost his whole profile when the official software launched and had to export by hand as a workaround. Autosave ([14](../done/14-autosave.md)) prevents loss, but cannot undo a bad edit, a bad import or a bad save from an earlier session.

## Scope
- Keep a bounded history of snapshots per profile (on app start, before import and replace, and at most every N minutes while editing); cap by count and size.
- "Restore previous version" in the profile menu: list with time and change summary, preview, restore as a new draft.
- Never silently drop a snapshot that is the only copy; clear message when the store is full.

## Acceptance
- Break a profile, restore a snapshot, undo works; store limits unit-tested; backups survive an app crash.
