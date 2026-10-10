# 34 · Data-driven app preset packs

**Priority:** P2 · **Effort:** M · **Hardware needed:** no
Extends [11](11-first-run-templates-and-new-module-prompt.md).

## Why
Reviewers missed presets for DaVinci Resolve and Audacity and for games; the official app ships only OBS, Twitch, Streamlabs and three Adobe apps. Tom's notes the competition's plugin marketplace.

## Scope
- Preset packs as JSON in our own format (actions, labels, icons), validated on load, imported as dry-run-only profiles.
- First packs: DaVinci Resolve, Final Cut Pro, Logic, Audacity, Zoom/Teams mute, Spotify/Music.
- Contributors can add a pack without code; pack licence and source recorded.
- Packs can include a focus rule suggestion ([13](13-surface-focus-rules-and-undo.md)).

## Acceptance
- Packs appear in the new-profile chooser, import cleanly, and invalid packs are rejected with a report.
