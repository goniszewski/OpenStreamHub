# 15 · Localisation scaffolding

**Priority:** P3 · **Effort:** M · **Hardware needed:** no

## Why
Official app ships 18 languages (with some hard-coded English). All our strings are inline in TSX.

## Scope
- Extract UI strings to message catalogues (English first) with a small i18n layer; no runtime CDN.
- Language picker in Settings (System default).
- Start with Polish as the second language, then community contributions.
- Do after tasks 01, 02, 06 so copy stabilises first.

## Acceptance
- No string literals in components for user-visible text (lint rule or test).
- Switching language needs no restart.
