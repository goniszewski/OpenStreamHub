# 35 · What the device shows when the app is not running

**Priority:** P2 · **Effort:** S (check) to M · **Hardware needed:** yes

## Why
APH Networks: the official hardware sits in a continuous loading loop and does nothing unless the utility runs, because the base and modules keep no configuration. We should know and state what ours does.

## Scope
- Test on hardware: normal quit, force quit, crash, sleep, unplug. Record what keys and dial show and whether presses do anything.
- Leave the last artwork showing on quit if the protocol allows, and make a login item / background helper easy to enable (`auto-start` exists).
- Tell the user plainly in the app that keys need the app running and what happens on quit.

## Acceptance
- Findings in `docs/masterhub-hardware-results.md`; app copy matches; any helper behaviour tested.
