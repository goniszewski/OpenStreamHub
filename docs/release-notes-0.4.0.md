# OpenStreamHub 0.4.0

Answer your AI coding agent's prompts from the deck, and the two-roller module finally works on macOS. An unofficial macOS app for Cooler Master MasterHUB modules, independent of and not endorsed by Cooler Master.

> [!IMPORTANT]
> This build is **not signed or notarized**, like earlier releases. On first launch, use **System Settings → Privacy & Security → Open Anyway**. See [Install](https://github.com/goniszewski/OpenStreamHub#install). Unsigned builds count as a new app after each update, so macOS may ask for Accessibility, Automation and Input Monitoring again.

## What is new

- **Two-roller module (M032LE) works on macOS.** Its firmware sends nothing until it receives a HID SET_IDLE request, which macOS never sends to this device. The SDK now sends it every time the module is opened, so the rollers report input. After unplugging and replugging the module the app starts a new session by itself and the rollers work again. Idea and fix by KamilKobylarek ([issue #4](https://github.com/goniszewski/OpenStreamHub/issues/4)). A new `masterhub-diag roller-set-idle` command tests it by hand, and `capture` sends the request too.
- **Answer agent prompts from the deck.** Claude Code and Codex hooks can ask for permission on the deck and take Accept or Deny from a key. Several agent sessions can share the deck. The bundled `osh` tool talks to the app, and a Terminal and AI agents starter profile, session keys and an Open terminal and run action are included.
- **AI agents panel with a first-run checklist.** The panel shows hook snippets to paste and an optional `osh` link, and a checklist that reads real state: whether `osh` is found, whether your Claude Code or Codex settings hold the hooks (read only; the app never edits those files), and whether the deck has answered a prompt this session. It collapses to one line when `osh`, at least one agent and an answered prompt are in place.
- **Variables and key faces.** Outside tools can read and set variables through a local socket and `osh`. Key faces can follow a variable, are edited in the inspector, and variable values survive the restart a profile save causes.
- **UX polish.** Changes across Workspace, header, sidebar, Settings and About, including a table of contents in Settings, a Fit to window option for the base layout, a clearer run control, and About links that open this project and the SDK it builds on.

## Fixes

- Turning a knob, roller or dial quickly no longer makes volume lag behind: steps that queue up while the volume is being set are summed, and each change is one `osascript` run instead of two.
- Select lists no longer add a scrollbar to the sidebar.
- App key labels follow edits and app names learned later.

## Checked on hardware

Roller input on one real M032LE: both rollers and both directions, one event per click, and recovery after an unplug and replug in the desktop app. Nothing else in this release was newly checked on hardware.

## Known limits

- **Two-roller module:** sleep and wake and base power loss are not verified, and only one unit has been tested.
- **Agent hooks, the deck prompts and the conditional flows** are covered by tests, not by a hardware run of every path.
- **Intel build:** cross-compiled on an Apple Silicon Mac, ad-hoc signed, and not launched on Intel hardware or under Rosetta.
- Interface language is English only.

## Requirements

macOS 12.3 or later, Apple Silicon or Intel. Builds are per architecture; pick the matching `.dmg`.
