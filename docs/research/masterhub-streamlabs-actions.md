# MasterHUB 1.0.0.113: official Streamlabs actions

Analysis date: 27 October 2026. Purpose: pin down what the seven official Streamlabs Desktop actions send, so OpenStreamHub can add them the way the OBS actions were added ([OBS actions](masterhub-obs-actions.md)). Static, text-only reading of the extracted sources; nothing was executed and no Streamlabs instance was contacted. Provenance is in [app features](masterhub-1.0.0.113-app-features.md#provenance).

Paths are relative to `sources/src/`. `App.vue` line numbers are the executor (`case "<action>":`); `plugins/StreamlabsSingleton.js` is the connection.

## Connection model of the official app

- Streamlabs Desktop's Remote Control API, over a **SockJS** client (`sockjs-client`) at `http://<ip>:<port>/api` (default `127.0.0.1:59650`). The host and port are user settings (`streamlabsIp`, `streamlabsPort`) together with an API token (`streamlabsToken`) copied from Streamlabs Desktop's Remote Control settings.
- JSON-RPC 2.0 messages: `{"jsonrpc":"2.0","id":N,"method":<method>,"params":{"resource":<resourceId>,"args":[...]}}`. Replies carry the same `id` and `result` or `error`.
- First request after open is authentication: resource `TcpServerService`, method `auth`, one arg, the token.
- The official app keeps one long-lived connection and subscribes to events (`sceneSwitched`, `recordingStatusChange`, `streamingStatusChange`, `studioModeChanged`, `collectionSwitched`, `itemUpdated`, `sourceUpdated` and more) to mirror state for key glyphs. Our client should do what the OBS client does: one short connection per action, no subscriptions, so no live key state.
- A reply `result` can be a resource object with a `resourceId` (for example a scene item); methods on it are then called with that id as `resource`.

## The 7 actions

| # | Official action | Requests sent (resource.method) | Profile `func` fields | Semantics and quirks | `App.vue` |
| --- | --- | --- | --- | --- | --- |
| 1 | `streamlabs-record` | `StreamingService.toggleRecording` | none | Pure toggle. | l.2568 |
| 2 | `streamlabs-stream` | `StreamingService.toggleStreaming` | none | Pure toggle: goes live or ends the stream. | l.2578 |
| 3 | `streamlabs-studio-mode` | `TransitionsService.getModel`, then `enableStudioMode` or `disableStudioMode` | none | Read `studioMode`, write the opposite. | l.2589 |
| 4 | `streamlabs-transition` | `TransitionsService.executeStudioModeTransition` | none | Triggers the studio-mode transition; no mode check. | l.2611 |
| 5 | `streamlabs-scenes` | `SceneCollectionsService.activeCollection`, optional `SceneCollectionsService.load`, then `ScenesService.makeSceneActive` | `streamlabSceneCollectionId`, `streamlabsSceneId` | If the wanted collection is not active, it is loaded first and the scene switch is sent after a fixed 500 ms timer (a race: loading a collection can take longer). Ids are Streamlabs ids, not names. | l.2621 |
| 6 | `streamlabs-source-visibility` | `SceneCollectionsService.activeCollection`, then `<sceneItem resourceId>.setVisibility` (true/false) | `streamlabsSourceVsbCollectionId`, `streamlabsSourceVsbSceneId`, `streamlabsSourceVsbSourceId` | Acts only when the wanted collection is active (otherwise silent no-op). Toggles from a **cached** schema (`fetchSceneCollectionsSchema`), not a fresh read, so the cache can drift. A folder toggles all its children: visible if any is hidden. | l.2652 |
| 7 | `streamlabs-audio-mixer` | `<audio source resourceId>.getModel`, then `setDeflection` or `setMuted` | `streamlabsAudioTypeId` (`Set`, `Adjust`, `Mute`), `streamlabsAudioSourceId` (a resource id), `streamlabsAudioDeflection`, `streamlabsAudioStepSize`, `streamlabsAudioMuteMode` | `Set`: `setDeflection(d)`, a 0 to 1 fader position (not dB). `Adjust`: `getModel().fader.deflection + step`, **no clamp**. `Mute`: mode 1 toggle (read `muted`, write the opposite), 2 mute, 3 unmute. Also used by knob, roller and dial (l.3744, 4004, 4309). | l.2702 |

## Notes for our implementation

- **Names, not opaque ids.** Streamlabs ids are internal and unstable across installs (and a profile exported from one machine would not carry over). The official profile stores them anyway. Our actions should address scenes and sources by name and resolve ids at run time with read requests (`ScenesService.getScenes`, `SourcesService.getSources`, `AudioService.getSources`) on the same connection.
- **Clamp the fader.** `Adjust` should clamp to 0 to 1; the official code does not.
- **No fixed 500 ms.** For a scene in another collection, load then poll `activeCollection` until it matches (bounded by the action deadline), instead of sleeping.
- **Auth token.** Treat it exactly like the OBS password: macOS Keychain, no `Display`, never in a profile or log.
- **Loopback only**, as for OBS. The official host field is free text; ours would not be.
- **Transport is unverified.** SockJS servers expose a raw WebSocket endpoint at `/api/websocket` next to the framed transports. I believe Streamlabs Desktop's server (Node `sockjs`) does, which would let the existing hand-written client reuse its framing with a new path and no SockJS client. This has not been confirmed against a real Streamlabs Desktop; the first implementation step is a connection check, and if the raw endpoint is absent the SockJS XHR-streaming/polling framing would be needed instead.
- Action parity target: record, stream, studio mode, transition, scene, source visibility, audio (set, adjust, mute). Only toggles are known from the official code; whether `StreamingService` has separate start and stop methods is unchecked.
