# 36 · OBS connection status and troubleshooting

**Priority:** P2 · **Effort:** S · **Hardware needed:** no
Follows [04](../in-review/04-obs-live-pickers.md).

## Why
Noisy Pixel had trouble connecting the official software to OBS, and OBS actions fail silently when the connection is wrong.

## Scope
- Status chip (Connected, Not running, WebSocket off, Wrong password, Wrong port) in the OBS settings and the action editor.
- "Test connection" button and one-line fix hints for each state.
- Offer the default port and detect OBS running; password remains memory-only.

## Acceptance
- Each failure state produced against a fake server shows its own message; chip updates when OBS starts or stops.
