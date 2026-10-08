# Per-key static images and GIF playback

## Goal and evidence

The owner requested independent GIF or static artwork per M4315 key on 8 October 2026. Existing hardware tests establish static 72×72 baseline JPEG uploads and page redraws. Animation throughput and smoothness are not yet hardware-validated. The first version streams prepared JPEG frames from the host using the existing `02 36` image command; it does not assume a native GIF command or persistent firmware storage.

The roller module is a separate issue: the owner confirmed it works in the official Windows app. Its missing macOS input needs Windows startup/input traffic evidence, not guessed initialization writes.

## Contract and limits

- Add optional `artwork` to each schema-2 keypad page: canonical wire-index strings `"0"` through `"14"` map to `{frames: [{jpeg_base64, delay_ms}], loop_count}`. Zero loop count means infinite playback; positive values mean total plays. Existing profiles omit the field and retain generated tiles. Child-page Back (wire 0) remains generated.
- Prepared frames live inside the profile so save, export, import, recovery and session snapshots remain portable. Keep the existing 1 MiB serialized profile limit. Reject artwork that exceeds it rather than silently dropping frames.
- Accept PNG/JPEG and GIF sources up to 8 MiB. GIF canvas dimensions are at most 512×512, with at most 120 composited frames and 32 MiB cumulative decoded RGBA pixel work. Prepare at most 32 KiB of JPEG bytes per frame, 512 KiB per asset and 700 KiB across a profile; the JSON limit can be tighter.
- Frame delays are 100–60,000 ms. Clamp shorter GIF delays to 100 ms (conservative 10 FPS ceiling); reject longer ones. Preserve loop metadata. Static artwork has one frame and one play. Preserve transparency/disposal through the GIF decoder, then apply the existing fit/matte/rotation pipeline to each full frame.

## Implementation

1. Validate artwork in Rust profiles, including canonical key IDs, reserved Back, frame structure, base64, JPEG framing and aggregate limits. Decode before HID access.
2. Add bounded offline GIF preparation and a Tauri preparation command using the existing serialized artwork worker.
3. Add a per-key inspector file picker, preview, remove action and profile-size feedback. Preserve artwork while editing bindings, folders and names. Protect async file completion against a changed draft or selection.
4. Extend the keypad artwork provider and existing display thread with a monotonic, bounded timer. Draw only the active page, restart timelines on page entry, skip obsolete frames when transfers fall behind, diff unchanged JPEGs, and use the existing per-report write gate and Stop ordering. Static-only profiles retain request-driven rendering.
5. Test malformed/oversized GIFs, disposal and timing, profile validation/round trips, editor persistence/races, frame selection, page transitions and shutdown gating with fake transports.

## Acceptance

Implementation and offline checks do not establish animation smoothness. A separately consented keypad hardware test must check one animated key, mixed static/animated keys, folder navigation, input responsiveness and Stop restoring the built-in animation. Multiple animated keys and observed transfer throughput determine whether the initial FPS cap needs adjustment. No hardware writes are authorized by this design document alone.

## Sources

- Repository: `crates/cooltek-sdk-cli/src/image_prep.rs`, `display_artwork.rs`, `live_runtime.rs`; service profile/keypad/display-sync models; `docs/masterhub-display-sync.md`.
- [`image` 0.25.10 GIF decoder](https://docs.rs/image/0.25.10/image/codecs/gif/index.html) and [AnimationDecoder](https://docs.rs/image/0.25.10/image/trait.AnimationDecoder.html): frame decoding and loop metadata. Enable its `gif` feature; enforce application bounds explicitly rather than relying only on decoder allocation hints.
