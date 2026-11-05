# MasterHUB 1.0.0.113: Discord, and whether OpenStreamHub can support it

Analysis date: 30 October 2026. Purpose: decide whether the nine official Discord actions can be added the way OBS and Streamlabs were. Static, text-only reading of the extracted sources, plus Discord's public RPC page (docs.discord.com/developers/topics/rpc). Nothing was executed and no Discord client or API was contacted. Provenance is in [app features](masterhub-1.0.0.113-app-features.md#provenance). Paths are relative to `sources/src/`.

**Verdict: feasible for personal use, not shippable as a turnkey feature.** The control channel is Discord's local IPC (a unix socket on this machine), which fits the app's no-network posture. The blocker is not the protocol but Discord's access policy: RPC is closed to unapproved apps except for up to 50 listed testers, so every user would have to register their own Discord application.

## What the official app does

- **Control channel: local IPC RPC.** `plugins/DiscordRpcSingleton.js` uses the `discord-rpc` package with `transport: 'ipc'`. This talks to the running Discord client over `${TMPDIR}/discord-ipc-{n}` (macOS and Linux). No Discord network call is needed for the actions themselves.
- **Authorization needs an OAuth2 application.** The app logs in with a client id taken from the build environment (`VUE_APP_DISCORD_CLIENT_ID`), requests the scopes `guilds.join rpc rpc.api rpc.voice.read rpc.voice.write rpc.notifications.read messages.read`, and keeps an access token per account.
- **Token exchange is remote and uses a secret.** `plugins/Discord.js` sends the OAuth code to `https://discord.com/api/v10/oauth2/token` with a `client_secret` also taken from the build environment (so the official app ships that secret to every user), with a `localhost:5173` redirect. It also lists guilds and channels through the REST API (`/users/@me/guilds`, `/guilds/{id}/channels`) to fill the editor pickers.
- **A hard-coded token sits in the source.** `discordAllChannels` in `plugins/Discord.js` contains a literal token-shaped string in a comment and in code. It is not reproduced here and must never be copied into this project. Whether it is live is unknown.
- **`plugins/DiscordSingleton.js` is unrelated to the actions.** It is a raw Gateway (`wss://gateway.discord.gg`) client for a bot token and voice-state updates. Nothing in the nine actions needs it.

## The 9 actions (`App.vue` executor)

| Official action | RPC call | Quirks |
| --- | --- | --- |
| `discord-mute` (l.2778) | `GET_VOICE_SETTINGS`, then `SET_VOICE_SETTINGS {mute, deaf:false}` | Modes toggle, mute, unmute. Every branch also sets `deaf:false`, so muting un-deafens. |
| `discord-deafen` (l.2823) | `GET_VOICE_SETTINGS`, then `SET_VOICE_SETTINGS` | Deaf sets `mute:true, deaf:true`; un-deafen sets both false (also un-mutes). |
| `discord-voice-channel` (l.2844) | `SELECT_VOICE_CHANNEL {channel_id, force:true}` | Channel id is stored in the profile. Discord says `force` is meant only for users who approved being moved. |
| `discord-text-channel` (l.2856) | `SELECT_TEXT_CHANNEL {channel_id}` | Id stored in the profile. |
| `discord-set-audio-device` (l.2866) | `SET_VOICE_SETTINGS {input.device, output.device}` | Device ids from the user's machine. |
| `discord-voice-input-mode` (l.2878) | `SET_VOICE_SETTINGS {mode.type}` | `VOICE_ACTIVITY` or `PUSH_TO_TALK`. |
| `discord-user-volume-control` (l.2893) | `SET_USER_VOICE_SETTINGS {user_id, volume, mute, pan}` | A per-user volume or mute, user id stored in the profile. Clamped 0 to 200. |
| `discord-output-volume-control` (l.2950) | `SET_VOICE_SETTINGS {output.volume}` | Clamped 0 to 200. Also used by knob, roller and dial. |
| `discord-input-volume-control` (l.2978) | `SET_VOICE_SETTINGS {input.volume}` | Clamped 0 to 200. |

Eight of nine are `SET_VOICE_SETTINGS` or `SELECT_*` calls. Discord's page lists no per-command scope for them, but the official app requests `rpc` and `rpc.voice.write`.

## Discord's access policy (the blocker)

From the public RPC page, quoted from a fetch of that page today:

- "We currently do not allow access to RPC for unapproved apps without being on the game's list of testers." Up to 50 testers can be invited. Unapproved apps are limited to 10 guilds and 10 channels. After approval the limits go and the app is open to everyone.
- The WebSocket transport is deprecated ("only available for old participants of private beta"). IPC remains.
- The page recommends the **Discord Social SDK for new projects**. That SDK is aimed at games and does not drive the user's own Discord client settings, so it is not a replacement for these actions.
- Only one app at a time may change voice settings; a second app connected at the same time loses the ability, and settings revert when the controlling app disconnects. A key press that changes mute is therefore reverted when our short-lived connection closes unless the connection stays open. This is the largest design finding: **the OBS/Streamlabs model of one short connection per action does not work for Discord.** A long-lived authenticated connection owned by the service would be required.
- The page does not state which scopes need approval, and I did not confirm the current rules on the developer portal. Third-party library docs mark `rpc`, `rpc.voice.write` and `messages.read` as approval-gated. Treat all of that as unverified.

## Options

1. **Bring-your-own Discord app (recommended if we build it).** The user creates an application in the Discord developer portal, adds themselves as a tester, and enters the client id (and secret, if the exchange needs it) in settings. Secrets go to the Keychain, like the OBS password. No application or secret is shipped by us. Setup is heavy for a stream-deck user, but it is honest and within the policy.
2. **Ship one OpenStreamHub Discord application.** Needs Discord's approval, which we cannot assume, and a shared client id (acceptable) and possibly a shared secret (not acceptable to embed). Out of scope until approval is confirmed.
3. **Reuse the official app's client id.** Not possible (it is a build-time value, not in the extracted source) and not appropriate.
4. **Drop Discord.** Mute, deafen and push-to-talk can already be bound as global shortcuts inside Discord (Discord keybinds, plus `shortcut` actions). The preset-pack route would cover the common cases without any Discord integration.

## What building option 1 would take

1. **Long-lived IPC session in the service** (new): the first persistent third-party connection. It needs reconnect, lifetime tied to the service, and a clean shutdown. This changes the service's current per-action model.
2. **IPC client**: unix socket `discord-ipc-0..9`, binary frames (opcode and length header, JSON body), `HANDSHAKE`, `AUTHORIZE`, `AUTHENTICATE`. Written by hand like the OBS WebSocket client, no new dependency.
3. **OAuth code exchange** against `discord.com`: the only remote call, once per authorization, plus token refresh. Whether a client secret is required for the exchange is unverified; a PKCE public client would avoid storing one.
4. **Settings and editor**: client id, Connect button (Discord shows its own authorization prompt), Keychain items, name lists for guilds, channels and devices (via `GET_GUILDS`, `GET_CHANNELS` over the same IPC connection, so no REST call).
5. **Actions and profile model**: the nine actions with the quirks above fixed (mute should not un-deafen; clamp 0 to 200), diagnostics redaction for channel and user ids, docs and fixtures.

Size is comparable to Streamlabs plus the long-lived connection work, with higher risk because it cannot be tested without a registered Discord app and tester access.

## Recommendation

Do not start Discord work yet. If you want it, the next step is a one-hour feasibility spike, run by hand on your Mac: register a throwaway Discord application, add yourself as a tester, and check from a short script that IPC accepts `AUTHORIZE` with `rpc rpc.voice.write` and that `SET_VOICE_SETTINGS` works, including whether a client secret is needed and what happens to the setting when the connection closes. Until that passes, everything above rests on documentation, not observation.
