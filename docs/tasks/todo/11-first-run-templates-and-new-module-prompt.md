# 11 · First-run templates and "new module detected" prompt

**Priority:** P2 · **Effort:** M · **Hardware needed:** no

## Why
Official first run offers Default / Photoshop (bundled `.mhz`) / New cards, and hot-plugging a module asks which saved setting to apply. Ours has a text onboarding dialog and starts with an empty profile.

## Scope
- First-run chooser with starter profiles (Mac basics, Streaming/OBS, Editing/Media) authored in our own JSON format; offered per detected module.
- "Module connected" toast/sheet when the device feed sees a new module: apply a template, or ignore.
- Replace/trim the text onboarding (`OnboardingDialog.tsx`) around the new flow; keep it reachable from Help.
- Templates ship dry-run-safe; Execute still needs confirmation.

## Acceptance
- A fresh data directory shows the chooser; choosing a template creates a valid saved profile.
- Hot-plug prompt tested with a simulated inventory (`device-feed.ts`).
