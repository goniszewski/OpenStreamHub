//! The pictures plugins draw on M4315 keys (step 4c of `docs/plugins-design.md`).
//!
//! A keypad action tile is a plugin's to draw while its first binding runs a plugin action directly
//! ([`cooltek_sdk_service::KeypadTile::plugin_drawer`]). The display thread tells the app which of those keys are on
//! screen with `plugin_key` records (`shown`, `hidden`) as the drawn page changes, and the app hands back prepared
//! images on the control channel (`plugin_image`, `plugin_image_clear`). [`PluginImageTable`] is the small table
//! between the control channel and the display thread, as [`crate::timer_face::TimerFaces`] is for countdowns: a
//! change wakes the display thread, which redraws through the usual `DisplaySync` diff. Nothing here writes to the
//! device, and images live only for the run.

use std::{
    collections::HashMap,
    io::Cursor,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use base64::{Engine, engine::general_purpose::STANDARD};
use cooltek_sdk_masterhub::keypad;
use cooltek_sdk_service::{
    Action, DeviceContext, KeypadLayout, PluginBridge, PluginKey, Profile, SharedVariables,
    display_sync::{KeyContent, KeyImageProvider, KeyImageRequest},
    plugin::fill_in_plugin_params,
};
use image::{ImageDecoder, codecs::jpeg::JpegDecoder};

/// Largest plugin image accepted, decoded.
pub const MAX_PLUGIN_IMAGE_BYTES: usize = 32 * 1024;

/// Per (page, slot): the plugin that drew the image and its prepared JPEG.
type Images = HashMap<(String, u8), (String, Vec<u8>)>;

#[derive(Default)]
struct TableShared {
    images: Mutex<Images>,
    version: AtomicU64,
}

/// The images plugins drew, per key, for the display thread. Cloning shares the table.
#[derive(Clone, Default)]
pub struct PluginImageTable {
    shared: Arc<TableShared>,
}

impl PluginImageTable {
    fn images(&self) -> std::sync::MutexGuard<'_, Images> {
        self.shared
            .images
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `jpeg` as the picture `plugin` draws on `slot` of `page`.
    pub fn set(&self, page: &str, slot: u8, plugin: &str, jpeg: Vec<u8>) {
        let mut images = self.images();
        let entry = (plugin.to_string(), jpeg);
        if images.get(&(page.to_string(), slot)) != Some(&entry) {
            images.insert((page.to_string(), slot), entry);
            self.shared.version.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The picture a plugin drew on `slot` of `page`, if any.
    pub fn get(&self, page: &str, slot: u8) -> Option<Vec<u8>> {
        self.images()
            .get(&(page.to_string(), slot))
            .map(|(_, jpeg)| jpeg.clone())
    }

    /// Forgets every image for which `drop` holds, given its page, slot and plugin. Wakes the display thread only
    /// when something went.
    pub fn clear(&self, drop: impl Fn(&str, u8, &str) -> bool) {
        let mut images = self.images();
        let before = images.len();
        images.retain(|(page, slot), (plugin, _)| !drop(page, *slot, plugin));
        if images.len() != before {
            self.shared.version.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Changes whenever the table does.
    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::SeqCst)
    }
}

/// Wraps a provider so a key a plugin drew shows that picture; every other key comes from the inner provider. Sits
/// below the key faces and above the key's own artwork, so a face whose condition holds still wins. Animation timing
/// is the inner provider's.
pub struct PluginImages<P> {
    pub inner: P,
    pub table: PluginImageTable,
}

impl<P: KeyImageProvider> KeyImageProvider for PluginImages<P> {
    fn key_image(&self, request: &KeyImageRequest) -> Option<Vec<u8>> {
        if let KeyContent::Action { page_id, .. } = &request.content
            && let Some(jpeg) = self.table.get(page_id, request.key_index)
        {
            return Some(jpeg);
        }
        self.inner.key_image(request)
    }

    fn advance(&mut self, page_id: &str, elapsed: std::time::Duration) {
        self.inner.advance(page_id, elapsed);
    }

    fn next_frame_in(&self) -> Option<std::time::Duration> {
        self.inner.next_frame_in()
    }
}

/// Checks a plugin image as standard base64 of a 72×72 baseline JFIF of at most [`MAX_PLUGIN_IMAGE_BYTES`], with
/// its pixels decoded, and returns its bytes.
pub fn decode_plugin_image(base64: &str) -> Result<Vec<u8>, String> {
    if base64.len() > MAX_PLUGIN_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err("the plugin image is larger than 32 KiB".into());
    }
    let jpeg = STANDARD
        .decode(base64)
        .map_err(|_| "the plugin image is not valid base64".to_string())?;
    if jpeg.len() > MAX_PLUGIN_IMAGE_BYTES {
        return Err("the plugin image is larger than 32 KiB".into());
    }
    keypad::validate_jpeg(&jpeg)
        .map_err(|error| format!("the plugin image is not a key image: {error}"))?;
    let decoder = JpegDecoder::new(Cursor::new(&jpeg))
        .map_err(|error| format!("the plugin image is not a key image: {error}"))?;
    if decoder.dimensions() != (72, 72) || decoder.total_bytes() > 72 * 72 * 4 {
        return Err("the plugin image must be 72×72 pixels".into());
    }
    image::DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("the plugin image's pixels do not decode: {error}"))?;
    Ok(jpeg)
}

/// The keys on screen, as last told to the app.
#[derive(Default)]
struct Shown {
    page: Option<String>,
    keys: Vec<PluginKey>,
    /// Set at run end: nothing is shown after it.
    closed: bool,
}

struct KeysShared {
    layout: KeypadLayout,
    device: DeviceContext,
    table: PluginImageTable,
    bridge: PluginBridge,
    variables: SharedVariables,
    shown: Mutex<Shown>,
}

/// The keys plugins draw in one keypad run that draws its keys: which tiles are a plugin's, the images they drew,
/// and which are on screen. The control channel takes images through it, and the display thread tells it which page
/// it draws. Cloning shares it.
#[derive(Clone)]
pub struct PluginKeys {
    shared: Arc<KeysShared>,
}

impl PluginKeys {
    /// For the keypad pages of `profile` on `device`, with images kept in `table`, records written through
    /// `bridge`, and parameters filled in from `variables`. `None` for a profile without keypad pages.
    pub fn new(
        profile: &Profile,
        device: DeviceContext,
        table: PluginImageTable,
        bridge: PluginBridge,
        variables: SharedVariables,
    ) -> Option<Self> {
        Some(Self {
            shared: Arc::new(KeysShared {
                layout: profile.keypad.clone()?,
                device,
                table,
                bridge,
                variables,
                shown: Mutex::new(Shown::default()),
            }),
        })
    }

    /// The table the display draws from.
    pub fn table(&self) -> &PluginImageTable {
        &self.shared.table
    }

    fn shown(&self) -> std::sync::MutexGuard<'_, Shown> {
        self.shared
            .shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The plugin action that draws `slot` of `page`, or why there is none.
    fn drawer(&self, page: &str, slot: u8) -> Result<&Action, String> {
        let layout = &self.shared.layout;
        let found = layout
            .page(page)
            .ok_or_else(|| format!("the profile has no keypad page {page:?}"))?;
        let tile = found
            .slots
            .get(usize::from(slot))
            .ok_or_else(|| format!("slot {slot} is not a key: keys are 0 to 14"))?;
        tile.as_ref()
            .and_then(|tile| tile.plugin_drawer(&self.shared.device))
            .ok_or_else(|| format!("key {slot} on page {page:?} is not drawn by a plugin"))
    }

    /// Keeps the image (standard base64 of a 72×72 baseline JFIF) the plugin that draws `slot` of `page` sent, and
    /// redraws the key if it is on screen. A key of another page is kept for when its page opens.
    pub fn set_image(&self, page: &str, slot: u8, base64: &str) -> Result<(), String> {
        let Action::Plugin { plugin, .. } = self.drawer(page, slot)? else {
            unreachable!("a drawer is a plugin action");
        };
        let jpeg = decode_plugin_image(base64)?;
        // Held while it is kept, so an image never outlives `end`, which empties the table.
        let shown = self.shown();
        if shown.closed {
            return Err("the run has ended".into());
        }
        self.shared.table.set(page, slot, plugin, jpeg);
        drop(shown);
        Ok(())
    }

    /// Forgets images: of one key (`page` and `slot`), of one `plugin`, or every image (none given). The keys show
    /// their own face again.
    pub fn clear(
        &self,
        page: Option<&str>,
        slot: Option<u8>,
        plugin: Option<&str>,
    ) -> Result<(), String> {
        let table = &self.shared.table;
        match (page, slot, plugin) {
            (Some(page), Some(slot), None) => {
                self.drawer(page, slot)?;
                table.clear(|p, s, _| p == page && s == slot);
            }
            (None, None, Some(plugin)) => table.clear(|_, _, p| p == plugin),
            (None, None, None) => table.clear(|_, _, _| true),
            _ => {
                return Err(
                    "plugin_image_clear takes a page and a slot, a plugin, or nothing".into(),
                );
            }
        }
        Ok(())
    }

    /// The keys of `page` a plugin draws, with their parameters filled in now. A key whose parameters cannot be
    /// filled in (a value breaks the size or one-line rules) is left out, as an event would be.
    fn keys_of(&self, page: &str) -> Vec<PluginKey> {
        let shared = &self.shared;
        shared
            .layout
            .plugin_keys(page, &shared.device)
            .into_iter()
            .filter_map(|(slot, action)| {
                let Action::Plugin {
                    plugin,
                    min_version,
                    action,
                    params,
                    variables,
                } = action
                else {
                    return None;
                };
                let params = if variables.is_empty() {
                    params.clone()
                } else {
                    shared
                        .variables
                        .read(|lookup| fill_in_plugin_params(params, variables, lookup))
                        .ok()?
                };
                Some(PluginKey {
                    page: page.to_string(),
                    slot,
                    plugin: plugin.clone(),
                    min_version: min_version.clone(),
                    action: action.clone(),
                    params,
                    variables: variables.clone(),
                })
            })
            .collect()
    }

    /// Says `page` is the page drawn now: when it is another than before, `hidden` for each key of the old page and
    /// then `shown` for each key of the new one. Keys of different pages are different keys, even in the same slot.
    /// Does nothing once the run ended.
    pub fn show_page(&self, page: &str) {
        let mut shown = self.shown();
        if shown.closed || shown.page.as_deref() == Some(page) {
            return;
        }
        let next = self.keys_of(page);
        // A record the app cannot read is lost; the run goes on.
        for key in &shown.keys {
            let _ = self.shared.bridge.key(key, false);
        }
        for key in &next {
            let _ = self.shared.bridge.key(key, true);
        }
        shown.page = Some(page.to_string());
        shown.keys = next;
    }

    /// At run end: `hidden` for every key on screen, then every image is forgotten and nothing is shown again.
    pub fn end(&self) {
        let mut shown = self.shown();
        if shown.closed {
            return;
        }
        shown.closed = true;
        for key in std::mem::take(&mut shown.keys) {
            let _ = self.shared.bridge.key(&key, false);
        }
        drop(shown);
        self.shared.table.clear(|_, _, _| true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_artwork::{PreparedKeyArtwork, PressedKey, VariableFaces, encode};
    use crate::timer_face::{Countdown, CountdownKey};
    use cooltek_sdk_service::{ServiceModule, display_sync::TileIcon};
    use image::{Rgb, RgbImage};
    use serde_json::{Value, json};
    use std::time::{Duration, Instant};

    fn plain(colour: [u8; 3]) -> Vec<u8> {
        encode(&RgbImage::from_pixel(72, 72, Rgb(colour))).unwrap()
    }

    fn plugin(action: &str, channel: &str) -> Value {
        json!({"type": "plugin", "plugin": "com.x.now", "min_version": "1.0.0", "action": action,
            "params": {"channel": channel, "room": "{{room}}"}, "variables": ["room"]})
    }

    fn profile() -> Profile {
        let mut root: Vec<Value> = vec![Value::Null; 15];
        root[1] = json!({"type": "folder", "page_id": "more"});
        root[3] = json!({"type": "action", "label": "Now", "bindings": [{"gesture": "pressed", "action": plugin("now", "a")}],
            "faces": [{"when": {"variable_is": {"name": "live", "value": true}}, "label": "Live"}]});
        root[4] = json!({"type": "action", "label": "Mute", "bindings": [{"gesture": "pressed", "action": {"type": "toggle_mute"}}]});
        let mut more: Vec<Value> = vec![Value::Null; 15];
        more[3] = json!({"type": "action", "label": "Other", "bindings": [{"gesture": "pressed", "action": plugin("other", "b")}]});
        let profile = json!({"schema_version": 2, "name": "Draw", "bindings": [],
            "variables": [{"name": "room", "value": "desk"}, {"name": "live", "value": false}],
            "keypad": {"pages": [{"id": "root", "name": "Keys", "slots": root},
                {"id": "more", "name": "More", "slots": more}]}});
        Profile::from_json(profile.to_string().as_bytes()).unwrap()
    }

    fn keys_with(
        profile: &Profile,
        variables: SharedVariables,
    ) -> (PluginKeys, Arc<Mutex<Vec<Value>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&sent);
        let bridge = PluginBridge::new(Box::new(move |record| {
            log.lock().unwrap().push(record.clone());
            Ok(())
        }));
        let device = DeviceContext {
            module: ServiceModule::M4315,
            serial_number: None,
        };
        let keys = PluginKeys::new(
            profile,
            device,
            PluginImageTable::default(),
            bridge,
            variables,
        )
        .unwrap();
        (keys, sent)
    }

    fn variables(profile: &Profile) -> SharedVariables {
        let variables = SharedVariables::new();
        variables.declare(&profile.variables);
        variables
    }

    fn states(sent: &Arc<Mutex<Vec<Value>>>) -> Vec<String> {
        std::mem::take(&mut *sent.lock().unwrap())
            .into_iter()
            .map(|record| {
                assert_eq!(record["type"], "plugin_key");
                format!(
                    "{} {}/{} {}",
                    record["state"].as_str().unwrap(),
                    record["page"].as_str().unwrap(),
                    record["slot"],
                    record["action"].as_str().unwrap()
                )
            })
            .collect()
    }

    #[test]
    fn page_switches_hide_the_old_keys_then_show_the_new_ones_and_the_end_hides_the_rest() {
        let profile = profile();
        let variables = variables(&profile);
        let (keys, sent) = keys_with(&profile, variables.clone());
        keys.show_page("root");
        let first = sent.lock().unwrap()[0].clone();
        // Variables filled in when shown.
        assert_eq!(
            first,
            json!({"type": "plugin_key", "state": "shown", "page": "root", "slot": 3, "plugin": "com.x.now",
                "min_version": "1.0.0", "action": "now", "params": {"channel": "a", "room": "desk"},
                "variables": ["room"], "control": {"module": "m4315", "index": 3}})
        );
        assert_eq!(states(&sent), ["shown root/3 now"]);
        // The same page again says nothing.
        keys.show_page("root");
        assert!(states(&sent).is_empty());
        // The same slot on another page is another key.
        variables
            .set(
                "room",
                cooltek_sdk_service::VariableValue::Text("hall".into()),
            )
            .unwrap();
        keys.show_page("more");
        let sent_now = sent.lock().unwrap().clone();
        // The hidden record repeats what was shown.
        assert_eq!(sent_now[0]["params"]["room"], "desk");
        assert_eq!(sent_now[1]["params"]["room"], "hall");
        assert_eq!(states(&sent), ["hidden root/3 now", "shown more/3 other"]);
        keys.show_page("root");
        assert_eq!(states(&sent), ["hidden more/3 other", "shown root/3 now"]);
        keys.set_image("root", 3, &STANDARD.encode(plain([200, 0, 0])))
            .unwrap();
        keys.end();
        assert_eq!(states(&sent), ["hidden root/3 now"]);
        assert!(keys.table().get("root", 3).is_none());
        // Nothing after the end, and no image is kept any more.
        keys.show_page("more");
        keys.end();
        assert!(states(&sent).is_empty());
        assert_eq!(
            keys.set_image("root", 3, &STANDARD.encode(plain([0, 200, 0])))
                .unwrap_err(),
            "the run has ended"
        );
        assert!(keys.table().get("root", 3).is_none());
    }

    #[test]
    fn a_key_whose_parameters_cannot_be_filled_in_is_not_shown() {
        let profile = profile();
        let variables = variables(&profile);
        variables
            .set(
                "room",
                cooltek_sdk_service::VariableValue::Text("a\u{200b}b".into()),
            )
            .unwrap();
        let (keys, sent) = keys_with(&profile, variables);
        keys.show_page("root");
        assert!(states(&sent).is_empty());
    }

    #[test]
    fn images_are_taken_only_for_a_plugins_keys_and_checked() {
        let profile = profile();
        let (keys, _) = keys_with(&profile, variables(&profile));
        let red = plain([200, 0, 0]);
        // A key of a page not on screen is kept for later.
        keys.set_image("more", 3, &STANDARD.encode(&red)).unwrap();
        assert_eq!(keys.table().get("more", 3), Some(red.clone()));
        let error =
            |page: &str, slot: u8, base64: &str| keys.set_image(page, slot, base64).unwrap_err();
        let good = STANDARD.encode(&red);
        assert_eq!(
            error("gone", 3, &good),
            "the profile has no keypad page \"gone\""
        );
        assert_eq!(
            error("root", 15, &good),
            "slot 15 is not a key: keys are 0 to 14"
        );
        for slot in [0, 1, 4] {
            assert_eq!(
                error("root", slot, &good),
                format!("key {slot} on page \"root\" is not drawn by a plugin")
            );
        }
        assert_eq!(
            error("root", 3, "%%%"),
            "the plugin image is not valid base64"
        );
        assert!(
            error("root", 3, &STANDARD.encode(b"not a jpeg"))
                .starts_with("the plugin image is not a key image")
        );
        let wide = {
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
                .encode(&[0; 96 * 72 * 3], 96, 72, image::ExtendedColorType::Rgb8)
                .unwrap();
            out
        };
        assert!(error("root", 3, &STANDARD.encode(&wide)).starts_with("the plugin image"));
        assert_eq!(
            error("root", 3, &"A".repeat(44 * 1024)),
            "the plugin image is larger than 32 KiB"
        );
        assert!(keys.table().get("root", 3).is_none());
    }

    #[test]
    fn clearing_takes_one_key_one_plugin_or_everything() {
        let mut profile = profile();
        // A second plugin on root key 5.
        if let Some(layout) = &mut profile.keypad {
            let mut tile = layout.pages[0].slots[3].clone();
            if let Some(cooltek_sdk_service::KeypadTile::Action {
                bindings, faces, ..
            }) = &mut tile
            {
                faces.clear();
                if let Action::Plugin { plugin, .. } = &mut bindings[0].action {
                    *plugin = "com.y.other".into();
                }
            }
            layout.pages[0].slots[5] = tile;
        }
        let (keys, _) = keys_with(&profile, variables(&profile));
        let image = STANDARD.encode(plain([0, 200, 0]));
        let fill = || {
            for (page, slot) in [("root", 3), ("more", 3), ("root", 5)] {
                keys.set_image(page, slot, &image).unwrap();
            }
        };
        let present = || {
            [("root", 3), ("more", 3), ("root", 5)]
                .map(|(page, slot)| keys.table().get(page, slot).is_some())
        };
        fill();
        let version = keys.table().version();
        // Setting the same picture again changes nothing.
        keys.set_image("root", 3, &image).unwrap();
        assert_eq!(keys.table().version(), version);
        keys.clear(Some("root"), Some(3), None).unwrap();
        assert_eq!(present(), [false, true, true]);
        assert!(keys.table().version() > version);
        keys.clear(None, None, Some("com.x.now")).unwrap();
        assert_eq!(present(), [false, false, true]);
        fill();
        keys.clear(None, None, None).unwrap();
        assert_eq!(present(), [false, false, false]);
        assert!(keys.clear(Some("root"), Some(4), None).is_err());
        assert!(keys.clear(Some("gone"), Some(3), None).is_err());
        assert_eq!(
            keys.clear(Some("root"), None, None).unwrap_err(),
            "plugin_image_clear takes a page and a slot, a plugin, or nothing"
        );
        assert!(
            keys.clear(Some("root"), Some(3), Some("com.x.now"))
                .is_err()
        );
    }

    #[test]
    fn a_face_beats_the_plugin_image_which_beats_the_artwork_and_a_timer_and_a_press_sit_on_top() {
        let profile = profile();
        let variables = variables(&profile);
        let table = PluginImageTable::default();
        let artwork = PreparedKeyArtwork::prepare_profile(&profile).unwrap();
        let own = artwork.key_image(&request(3)).unwrap();
        let chain = VariableFaces::prepare(
            &profile,
            PluginImages {
                inner: artwork,
                table: table.clone(),
            },
            variables.clone(),
        )
        .unwrap();
        // Nothing drawn: the key's own artwork.
        assert_eq!(chain.key_image(&request(3)), Some(own.clone()));
        let red = plain([200, 0, 0]);
        table.set("root", 3, "com.x.now", red.clone());
        assert_eq!(chain.key_image(&request(3)), Some(red.clone()));
        // Another page's image is not this key's.
        table.set("more", 4, "com.x.now", plain([0, 0, 200]));
        assert_ne!(chain.key_image(&request(4)), Some(plain([0, 0, 200])));
        // A face whose condition holds wins over the plugin.
        variables
            .set("live", cooltek_sdk_service::VariableValue::Bool(true))
            .unwrap();
        let face = chain.key_image(&request(3)).unwrap();
        assert_ne!(face, red);
        assert_ne!(face, own);
        variables
            .set("live", cooltek_sdk_service::VariableValue::Bool(false))
            .unwrap();
        // A countdown and a press are drawn over the plugin's picture.
        let timers = [Countdown {
            control: 3,
            page_id: Some("root".into()),
            deadline_micros: 1,
            ends: Instant::now() + Duration::from_secs(30),
        }];
        let counted = CountdownKey {
            inner: &chain,
            timers: &timers,
            now: Instant::now(),
        };
        let timed = counted.key_image(&request(3)).unwrap();
        assert_ne!(timed, red);
        let pressed = PressedKey {
            inner: &chain,
            pressed: Some(3),
        };
        assert_eq!(
            pressed.key_image(&request(3)),
            crate::display_artwork::dimmed(&red)
        );
    }

    fn request(key_index: u8) -> KeyImageRequest {
        KeyImageRequest {
            key_index,
            content: KeyContent::Action {
                page_id: "root".into(),
                label: "Now".into(),
                icon: TileIcon::Application,
            },
        }
    }
}
