# MasterHUB 1.0.0.113: official Twitch actions

Analysis date: 23 November 2026. Purpose: pin down what the twelve official "Twitch Studio" actions send, and whether OpenStreamHub can add them the way the OBS, Streamlabs and Discord actions were considered ([OBS actions](masterhub-obs-actions.md), [Streamlabs actions](masterhub-streamlabs-actions.md), [Discord](masterhub-discord-feasibility.md)). Static, text-only reading of the extracted sources; nothing was executed and Twitch was never contacted. Provenance is in [app features](masterhub-1.0.0.113-app-features.md#provenance).

The official ZIP was downloaded again on 23 November from the recorded link. Its SHA-256 matched `cf9aef83…0d83f`, and the renderer map (`79da9c6d…`) and `background.js` (`d3139e72…`) hashes matched the recorded values. The renderer sources were rebuilt from `js/app.c762092d.js.map` into a scratch folder outside the repo.

Paths are relative to `sources/src/`. `App.vue` line numbers are the executor (`case "<action>":`); `plugins/Twitch.js` holds the Helix calls; the sign-in server is in `background.js` (minified, so no line numbers).

**Verdict: feasible for personal use, but the first action that touches the network changes the app's posture.** Every Twitch action is a call to a remote service with a user token, unlike OBS and Streamlabs (loopback) and Discord (a local socket). Nothing in the official protocol is hard to reproduce. The blocker is policy and trust, not protocol: the official app ships a Twitch client secret to every user, and we cannot do that. See [Design for OpenStreamHub](#design-for-openstreamhub).

## Connection model of the official app

- **Sign-in.** `openTwitchLoginPage` (`plugins/Twitch.js` l.1-26) builds an `https://id.twitch.tv/oauth2/authorize` URL (authorization-code flow) with the client id from the build environment, the redirect `VUE_APP_TWITCH_CLIENT_REDIRECT_URI`, a **constant `state` value** (`Some32BitsHashNumber`, so it gives no CSRF protection) and eight scopes:
  `channel:edit:commercial channel:manage:broadcast chat:edit chat:read clips:edit moderator:manage:chat_messages moderator:manage:chat_settings moderator:manage:shield_mode`.
- **Code exchange runs in the main process.** `background.js` starts a local Express server and serves `GET /auth/callback/:sns`. For `twitch` it posts the code to `https://id.twitch.tv/oauth2/token` with a hard-coded client id, a **hard-coded client secret** and the redirect `http://localhost:4040/auth/callback/twitch`, then reads the user from Twitch and sends `TWITCH_LOGIN` (`id`, `username` = `login`, `access_token`, `refresh_token`) to the renderer. The Discord callback is in the same handler and also has a hard-coded secret. Neither is reproduced here and neither must be copied into this project. Whether they are live is unknown.
- **Storage.** The renderer keeps accounts in the Vuex store at `sns.twitchAccounts` (`store/index.js` l.182; `App.vue` l.7391 handler), with plain `access_token` and `refresh_token` fields. Whether that store is written to disk, and where, was not checked.
- **Per action: validate, then maybe refresh.** Every function in `plugins/Twitch.js` first calls `GET https://id.twitch.tv/oauth2/validate` (`Authorization: OAuth <token>`). Only if the status is 401 it refreshes with `grant_type=refresh_token` plus the client id **and the client secret** from the build environment (`VUE_APP_TWITCH_CLIENT_SECRET`, l.40-56), and stores the new tokens through `refreshTokenFunc`. A failed refresh is not checked: the next request just goes out with `undefined`.
- **Helix calls** carry `Authorization: Bearer <token>` and `Client-Id`. Every moderator call passes `moderator_id` equal to `broadcaster_id`, both the signed-in user's own id, so the actions only work on the user's own channel.
- **Chat messages do not use Helix.** `plugins/TwitchChatSingleton.js` opens a long-lived `tmi.js` client (IRC over WebSocket) for the signed-in user's own channel, with the access token as the IRC password. It is created when an account is selected (`mixins/twitchMixin.js`). If sending fails, it refreshes the token, reconnects and sends once more.
- **One connection, state in the key glyph.** The toggle actions return the new state and the executor passes it to `changeOnImage` for every key bound to the same action. Our client would not keep a connection or subscribe to anything, so a glyph would show only what the last press set.

## The 12 actions

All take `display: ["15keys", "15keys-multi-tasking"]` (keypad keys and multi-task steps only, no dial, knob or roller; `store/module/masterHub.js` l.714-905).

| # | Official action | Request sent | Profile `func` fields | Semantics and quirks | `App.vue` |
| --- | --- | --- | --- | --- | --- |
| 1 | `chat-message` | IRC `PRIVMSG` to `#<own username>` through `tmi.js` | `twitchMessage` | Own channel only. Throws if no account has been selected yet, because `mainApp.twitchChatPlugin` does not exist until then. | l.2234 |
| 2 | `clear-chat` | `DELETE /helix/moderation/chat?broadcaster_id&moderator_id` | none | No `message_id`, so it clears the whole chat. No confirmation, no result check. | l.2240 |
| 3 | `emote-chat` | `GET`, then `PATCH /helix/chat/settings` `{emote_mode: !current}` | none | Read-then-write toggle. Glyph shows the new state. | l.2248 |
| 4 | `followers-chat` | `GET`, then `PATCH /helix/chat/settings` `{follower_mode: !current, follower_mode_duration}` | `followersChatDuration` (minutes: 0, 10, 30, 60, 1440, 10080, 43200, 129600) | Toggle. The duration is sent on every call, including when turning the mode off. | l.2281 |
| 5 | `play-ad` | `POST /helix/channels/commercial` `{broadcaster_id, length}` | `adDuration` (seconds, a list starting at 30, 60, 90) | Fire and forget. On failure it logs the **access token** to the console (`plugins/Twitch.js` l.410, 414). | l.2318 |
| 6 | `slow-chat` | `GET`, then `PATCH /helix/chat/settings` `{slow_mode: !current, slow_mode_wait_time}` | `slowChatDuration` (seconds, options start at 3, 5, 10; default 30) | Toggle, as for followers. | l.2329 |
| 7 | `stream-game-title` | `GET /helix/games?igdb_id=<selectedGameId>`, then `PATCH /helix/channels` `{game_id, title}` | `streamGameInfoTitle`, `selectedGameId` (an IGDB id) | The editor searches games through `GET https://api.igdb.com/v4/games?search=` with the Twitch token, which is a second host. If the game lookup finds nothing it still sends the title with `game_id: ""`, which clears the category. | l.2365 |
| 8 | `sub-chat` | `GET`, then `PATCH /helix/chat/settings` `{subscriber_mode: !current}` | none | Toggle. | l.2393 |
| 9 | `create-stream-marker` | `POST /helix/streams/markers` `{user_id, description}` | `desc` | Fails if the channel is not live. The response is not checked. | l.2426 |
| 10 | `create-clip` | `POST /helix/clips?broadcaster_id` | none | Stores the returned clip id in `sns.lastClipId`. Reads `resp.data` without a check, so an error reply (for example while offline) throws. | l.2437 |
| 11 | `open-last-clip` | none: opens `https://clips.twitch.tv/<lastClipId>/edit` in the browser through `ipc-open-browser` | none | Uses only the in-memory id from the last `create-clip`. The version that asked Helix for the clip is commented out. Does nothing before a first clip. | l.2452 |
| 12 | `shield-mode` | `GET`, then `PUT /helix/moderation/shield_mode` `{is_active: !current}` | none | Toggle. Reads `data[0].is_active` without a check. | l.2471 |

Five of the toggles (emote, followers, slow, subscriber, shield) read the state and then write the opposite. Two quick presses, or a change made in the Twitch UI between the read and the write, can leave the setting where the user did not want it.

## Design for OpenStreamHub

- **Do not reuse the official sign-in.** We have no client id of our own, and the official secret is not ours to use. The official refresh path needs a secret on the machine. A new Twitch application would be registered by the project, and the flow should be one that needs **no client secret**. Twitch documents a Device Code Grant Flow for public clients; I believe it fits (the user opens a short URL and types a code), but I did not check Twitch's current page for this note, so confirm it before building. A fixed localhost redirect on port 4040 would also be avoidable.
- **Tokens like the OBS password.** macOS Keychain, no `Display`, never in a profile, a log or a state dump. Never log a token, which the official `play-ad` does.
- **Real `state`/PKCE where a redirect is used.** The official constant `state` should not be copied.
- **Minimum scopes per action.** The official app asks for all eight at sign-in. Ours should list the scope each action needs and request only those the user's profile uses. From memory (verify on Twitch's scope page): `user:write:chat` (or the IRC scopes) for a chat message, `moderator:manage:chat_messages` for clearing chat, `moderator:manage:chat_settings` for the four chat modes, `moderator:manage:shield_mode`, `channel:edit:commercial`, `channel:manage:broadcast` for the title and markers, `clips:edit` for clips.
- **Chat message over Helix, not IRC.** One short HTTPS call fits the one-shot model better than a long-lived IRC socket. Twitch has a "Send Chat Message" Helix endpoint; I believe it needs `user:write:chat`, but I did not check it for this note.
- **Set modes, do not toggle blindly.** Offer `on`, `off` and `toggle`, as the official audio and Discord actions do for mute. For toggle, say plainly that it is read-then-write.
- **Check results.** Every official call ignores the HTTP status. Ours should report a failed refresh, an offline channel (markers, ads) and a rate limit as an action error.
- **Clips.** Keep the last clip id for `open-last-clip`, and decide whether it survives a restart.
- **Game category.** Search through Helix `GET /search/categories` instead of IGDB, so there is one host and one token. This has not been checked either.
- **Own channel only.** The official app uses the user's own id as both broadcaster and moderator. Acting on another channel as a moderator is out of scope.
- **Network posture.** This is the first integration that sends data off the machine (a chat message or a stream title is user text). It should be opt-in per profile, off by default, and say in the first-enable dialog which host it talks to (`id.twitch.tv`, `api.twitch.tv`).

## Not verified

- Nothing here was run: no account, no token, no Helix call. All request shapes are read from the source.
- Whether the Vuex store holding the tokens is written to disk, and how, was not checked.
- The Twitch details marked "from memory" in the design section.
- `open-last-clip` after a restart, and how the official app behaves when `refreshToken` fails, were not exercised.
- Native addons and the MasterCTRL worker were not inspected (as in the other notes).
