use crate::{
    Action, Binding, BindingEngine, Condition, DeviceContext, DialFunction, DialFunctionNavigation,
    Gesture, InputEvent, KeyArtwork, Profile, ServiceModule,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A pressed or released action associated with one keypad tile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeypadBinding {
    /// Button transition; only `pressed` and `released` are supported.
    pub gesture: Gesture,
    /// Exact serial selector, or absent for any serial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// Complete host action; input-dependent fader templates are unsupported.
    pub action: Action,
}

/// An ordinary action tile or a link to a child keypad page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeypadTile {
    /// A labelled key containing its transition and serial selectors.
    Action {
        /// Human-readable label of at most 128 bytes.
        label: String,
        /// Unique gesture/serial bindings for this tile.
        bindings: Vec<KeypadBinding>,
        /// Alternative faces chosen by variable values; the first whose
        /// `when` holds is shown, and none holding shows the normal face.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        faces: Vec<KeyFace>,
    },
    /// Enter a child page on a press; its page name supplies the folder label.
    Folder {
        /// ID of a page with exactly one parent.
        page_id: String,
    },
}

/// A face an action tile shows while a variable condition holds.
///
/// It replaces the tile's label and/or background (and any prepared artwork)
/// for as long as `when` holds; the icon and bindings stay the tile's own.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyFace {
    /// Variable-only condition (`variable_is`, `variable_at_least`, and `all`/`any` of them).
    pub when: Condition,
    /// Label shown instead of the tile label; absent keeps the tile label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Background hex colour such as `#ff0000`; absent keeps the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

impl KeyFace {
    /// Most faces one tile can carry.
    pub const MAX_PER_TILE: usize = 8;

    /// Checks the condition shape, label and colour; declared variables are checked by the profile.
    pub fn validate(&self) -> Result<(), String> {
        self.when.validate()?;
        if !self.when.is_variable_only() {
            return Err("a key face condition can only test variables".into());
        }
        if self.label.is_none() && self.background.is_none() {
            return Err("a key face needs a label or a background".into());
        }
        if let Some(label) = &self.label {
            validate_label(label, "key face label")?;
        }
        if let Some(background) = &self.background {
            crate::key_artwork::validate_hex_colour(background).map_err(|_| {
                "key face background must be a hex colour such as #000000 or #fff".to_string()
            })?;
        }
        Ok(())
    }
}

/// One fixed fifteen-key page in a keypad folder tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeypadPage {
    /// Unique ASCII identifier of 1 through 64 bytes.
    pub id: String,
    /// Human-readable page name of at most 128 bytes.
    pub name: String,
    /// Exactly fifteen slots in wire order; child slot 0 is reserved for Back.
    pub slots: Vec<Option<KeypadTile>>,
    /// Portable prepared artwork keyed by canonical decimal wire index.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artwork: BTreeMap<String, KeyArtwork>,
}

/// A bounded page tree rooted at the literal ID `root`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeypadLayout {
    /// Reachable pages with unique IDs and one parent per non-root page.
    pub pages: Vec<KeypadPage>,
}

impl KeypadLayout {
    /// Wire control count of the M4315 keypad.
    pub const KEY_COUNT: usize = 15;
    /// Required root identifier; root slot 0 remains an ordinary slot.
    pub const ROOT_PAGE_ID: &'static str = "root";
    /// Maximum pages in a profile.
    pub const MAX_PAGES: usize = 64;
    /// Maximum child level: root is 0 and children may extend through L8.
    pub const MAX_DEPTH: usize = 8;

    /// Counts actions on every page, including pages outside the current path.
    pub fn binding_count(&self) -> usize {
        self.pages
            .iter()
            .flat_map(|page| &page.slots)
            .filter_map(|slot| match slot {
                Some(KeypadTile::Action { bindings, .. }) => Some(bindings.len()),
                _ => None,
            })
            .sum()
    }

    /// Returns a page by its exact identifier.
    pub fn page(&self, id: &str) -> Option<&KeypadPage> {
        self.pages.iter().find(|page| page.id == id)
    }

    /// Checks all shape, binding, tree, and depth constraints.
    pub fn validate(&self) -> Result<(), String> {
        if self.pages.is_empty() || self.pages.len() > Self::MAX_PAGES {
            return Err(format!(
                "keypad must contain 1 through {} pages",
                Self::MAX_PAGES
            ));
        }
        if self.binding_count() > Profile::MAX_BINDINGS {
            return Err(format!("keypad exceeds {} bindings", Profile::MAX_BINDINGS));
        }
        let mut artwork_bytes = 0usize;
        let mut pages = HashMap::new();
        for (index, page) in self.pages.iter().enumerate() {
            validate_id(&page.id)?;
            validate_label(&page.name, "page name")?;
            if pages.insert(page.id.as_str(), index).is_some() {
                return Err(format!("duplicate keypad page ID {:?}", page.id));
            }
            if page.slots.len() != Self::KEY_COUNT {
                return Err(format!(
                    "page {:?} requires exactly {} slots",
                    page.id,
                    Self::KEY_COUNT
                ));
            }
            if page.id != Self::ROOT_PAGE_ID && page.slots[0].is_some() {
                return Err(format!(
                    "page {:?} slot 0 is reserved for Back and must be null",
                    page.id
                ));
            }
            for (key, artwork) in &page.artwork {
                let index = key
                    .parse::<usize>()
                    .ok()
                    .filter(|index| *index < Self::KEY_COUNT && index.to_string() == *key)
                    .ok_or_else(|| {
                        format!(
                            "page {:?}: artwork key must be canonical 0 through 14",
                            page.id
                        )
                    })?;
                if page.id != Self::ROOT_PAGE_ID && index == 0 {
                    return Err(format!("page {:?}: Back key artwork is reserved", page.id));
                }
                artwork_bytes = artwork_bytes
                    .checked_add(artwork.validate()?)
                    .ok_or("artwork byte count overflow")?;
                if artwork_bytes > KeyArtwork::MAX_PROFILE_BYTES {
                    return Err("profile prepared artwork exceeds 700 KiB".into());
                }
            }
            for (slot, tile) in page.slots.iter().enumerate() {
                if let Some(KeypadTile::Action {
                    label,
                    bindings,
                    faces,
                }) = tile
                {
                    validate_label(label, "tile label")?;
                    if faces.len() > KeyFace::MAX_PER_TILE {
                        return Err(format!(
                            "page {:?} slot {slot}: at most {} key faces",
                            page.id,
                            KeyFace::MAX_PER_TILE
                        ));
                    }
                    for face in faces {
                        face.validate()
                            .map_err(|error| format!("page {:?} slot {slot}: {error}", page.id))?;
                    }
                    let mut selectors = HashSet::new();
                    for binding in bindings {
                        if !matches!(binding.gesture, Gesture::Pressed | Gesture::Released) {
                            return Err(format!(
                                "page {:?} slot {slot}: keypad bindings require pressed or released",
                                page.id
                            ));
                        }
                        crate::profile::validate_serial_number(binding.serial_number.as_deref())?;
                        if !selectors.insert((binding.gesture, binding.serial_number.as_deref())) {
                            return Err(format!(
                                "page {:?} slot {slot}: duplicate keypad selector",
                                page.id
                            ));
                        }
                        binding.action.validate_executable()?;
                    }
                }
            }
        }
        let root = *pages
            .get(Self::ROOT_PAGE_ID)
            .ok_or("keypad requires a root page")?;
        let mut parents = vec![0; self.pages.len()];
        for page in &self.pages {
            for tile in page.slots.iter().flatten() {
                if let KeypadTile::Folder { page_id } = tile {
                    validate_id(page_id)?;
                    if page_id == Self::ROOT_PAGE_ID {
                        return Err("keypad folders cannot target the root page".into());
                    }
                    let target = *pages
                        .get(page_id.as_str())
                        .ok_or_else(|| format!("folder targets missing page {page_id:?}"))?;
                    parents[target] += 1;
                    if parents[target] > 1 {
                        return Err(format!("page {page_id:?} has multiple parents"));
                    }
                }
            }
        }
        let mut visited = HashSet::new();
        self.visit(root, 0, &pages, &mut visited)?;
        if visited.len() != self.pages.len() {
            return Err("keypad contains unreachable pages or a disconnected cycle".into());
        }
        Ok(())
    }

    fn visit(
        &self,
        index: usize,
        depth: usize,
        pages: &HashMap<&str, usize>,
        visited: &mut HashSet<usize>,
    ) -> Result<(), String> {
        if depth > Self::MAX_DEPTH {
            return Err(format!("keypad nesting exceeds L{}", Self::MAX_DEPTH));
        }
        if !visited.insert(index) {
            return Err("keypad contains a cycle".into());
        }
        for tile in self.pages[index].slots.iter().flatten() {
            if let KeypadTile::Folder { page_id } = tile {
                self.visit(pages[page_id.as_str()], depth + 1, pages, visited)?;
            }
        }
        Ok(())
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Err("keypad page IDs must contain 1 through 64 ASCII letters, digits, underscores, or hyphens".into())
    } else {
        Ok(())
    }
}

fn validate_label(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(format!(
            "{field} must be nonempty, free of control characters, and at most 128 bytes"
        ))
    } else {
        Ok(())
    }
}

/// A completed folder or Back transition, derived from the validated page tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KeypadNavigation {
    /// Page shown before this press.
    pub from_page: String,
    /// Page shown after this press.
    pub to_page: String,
    /// Full path from root through the new current page.
    pub path: Vec<String>,
    /// New child level, with root equal to zero.
    pub depth: usize,
}

/// The pure outcome of resolving one normalized input transition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeypadResolution {
    /// A concrete operation that may be previewed or explicitly dispatched.
    Action {
        /// Resolved host action, including any input-dependent value.
        action: Action,
        /// Original keypad page for schema-2 keys; absent for flat bindings.
        page_id: Option<String>,
    },
    /// A page change with no host action.
    Navigation {
        /// Completed folder or Back transition.
        navigation: KeypadNavigation,
    },
    /// No selected action, including duplicate presses or unmatched releases.
    Unbound,
}

/// The complete outcome of [`ProfileSession::route`], including dial function switches.
///
/// [`KeypadResolution`] remains the compatibility view returned by
/// [`ProfileSession::resolve`]; it reports a dial function switch as `Unbound`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProfileResolution {
    /// A concrete operation that may be previewed or explicitly dispatched.
    Action {
        /// Resolved host action, including any input-dependent value.
        action: Action,
        /// Original keypad page for schema-2 keys; absent otherwise.
        page_id: Option<String>,
        /// Current dial function for M131E bindings selected through `dial_functions`.
        dial_function: Option<String>,
    },
    /// A keypad page change with no host action.
    Navigation {
        /// Completed folder or Back transition.
        navigation: KeypadNavigation,
    },
    /// A short-click dial function switch with no host action.
    DialFunction {
        /// Previous and new current function.
        navigation: DialFunctionNavigation,
    },
    /// No selected action, including duplicate presses or unmatched releases.
    Unbound,
}

impl From<ProfileResolution> for KeypadResolution {
    fn from(resolution: ProfileResolution) -> Self {
        match resolution {
            ProfileResolution::Action {
                action, page_id, ..
            } => Self::Action { action, page_id },
            ProfileResolution::Navigation { navigation } => Self::Navigation { navigation },
            ProfileResolution::DialFunction { .. } | ProfileResolution::Unbound => Self::Unbound,
        }
    }
}

#[derive(Clone, Debug)]
struct HeldKey {
    page_id: String,
    action_tile: bool,
}

/// Stateful profile routing for one physical keypad interface.
///
/// Non-key modules and schema-1 profiles keep ordinary flat binding behavior.
/// Schema-2 keypad presses capture the original page and serial before navigating;
/// their release can resolve only on that original action tile. Navigation keys,
/// empty keys, duplicate presses, and unmatched releases never invoke a child
/// action. Held keys are tracked per `(serial, key)` so a transition from one
/// keypad never clears or completes another keypad's held key. Profiles with
/// `dial_functions` keep one current M131E function index, starting at the
/// first function and shared by every dial routed through this session. Reset
/// after disconnect, suspension, or replacing the selected profile.
#[derive(Clone, Debug)]
pub struct ProfileSession {
    /// Unique per [`ProfileSession::new`] in this process (clones share it), so
    /// work planned by one profile can be told from work planned by the next.
    id: u64,
    engine: BindingEngine,
    path: Vec<String>,
    held: HashMap<(Option<String>, usize), HeldKey>,
    dial_function: usize,
}

impl ProfileSession {
    /// Validates and owns an immutable profile, starting at root with no held keys.
    pub fn new(profile: Profile) -> Result<Self, String> {
        profile.validate()?;
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Ok(Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            engine: BindingEngine::from_validated(profile),
            path: vec![KeypadLayout::ROOT_PAGE_ID.into()],
            held: HashMap::new(),
            dial_function: 0,
        })
    }

    /// This session's id, never 0 and never shared with another session made
    /// by [`ProfileSession::new`] in this process.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the immutable validated profile.
    pub fn profile(&self) -> &Profile {
        self.engine.profile()
    }

    /// Returns the original flat selector, or `None` for schema-2 keypad or dial function routing.
    pub fn binding_for(&self, device: &DeviceContext, event: &InputEvent) -> Option<&Binding> {
        if device.module == ServiceModule::M4315 && self.profile().keypad.is_some()
            || device.module == ServiceModule::M131E && self.profile().dial_functions.is_some()
        {
            None
        } else {
            self.engine.binding_for(device, event)
        }
    }

    /// Returns the explicit root-to-current-page path.
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// Returns the current child level; root is zero.
    pub fn depth(&self) -> usize {
        self.path.len() - 1
    }

    /// Returns the displayed page, or `None` for a schema-1 profile.
    pub fn current_page(&self) -> Option<&KeypadPage> {
        self.profile().keypad.as_ref()?.page(self.path.last()?)
    }

    /// Selects an existing root-to-page path and clears held keys without actions.
    ///
    /// Each path edge must be an actual folder link. An invalid path leaves both
    /// navigation and held-key state unchanged.
    pub fn set_path(&mut self, path: &[String]) -> Result<(), String> {
        if path.is_empty()
            || path.len() > KeypadLayout::MAX_DEPTH + 1
            || path[0] != KeypadLayout::ROOT_PAGE_ID
        {
            return Err("keypad path must start at root and remain within L8".into());
        }
        if let Some(layout) = &self.profile().keypad {
            for pair in path.windows(2) {
                let parent = layout
                    .page(&pair[0])
                    .ok_or("keypad path contains a missing page")?;
                if !parent.slots.iter().flatten().any(
                    |tile| matches!(tile, KeypadTile::Folder { page_id } if page_id == &pair[1]),
                ) {
                    return Err("keypad path contains an invalid folder edge".into());
                }
            }
        } else if path.len() != 1 {
            return Err("flat profiles do not contain keypad child pages".into());
        }
        self.path = path.to_vec();
        self.clear_held();
        Ok(())
    }

    /// Returns the current dial function index, or `None` without `dial_functions`.
    pub fn dial_function_index(&self) -> Option<usize> {
        self.profile()
            .dial_functions
            .as_ref()
            .map(|_| self.dial_function)
    }

    /// Returns the current dial function, or `None` without `dial_functions`.
    pub fn current_dial_function(&self) -> Option<&DialFunction> {
        self.profile()
            .dial_functions
            .as_ref()?
            .get(self.dial_function)
    }

    /// Selects a dial function by index without running any action.
    ///
    /// An out-of-range index or a profile without functions leaves state unchanged.
    pub fn set_dial_function(&mut self, index: usize) -> Result<(), String> {
        let count = self.profile().dial_functions.as_ref().map_or(0, Vec::len);
        if index >= count {
            return Err(format!(
                "dial function index {index} is outside the profile's {count} functions"
            ));
        }
        self.dial_function = index;
        Ok(())
    }

    /// Returns to root and the first dial function, and forgets held keys
    /// without synthetic releases or actions.
    pub fn reset(&mut self) {
        self.path.truncate(1);
        self.dial_function = 0;
        self.clear_held();
    }

    fn clear_held(&mut self) {
        self.held.clear();
    }

    /// The plugin actions that hear `event` as a plugin event: those bound directly on its control, as the key's own
    /// action, when `event`'s gesture has no binding of its own there. Ask before [`ProfileSession::route`], which
    /// changes held keys and the dial function.
    ///
    /// A keypad key belongs to the tile on the page shown at the press, and its release to the tile that took the
    /// press (a duplicate press or an unmatched release goes to nobody). The dial belongs to the current function;
    /// a short press, which switches functions, and the raw pressed / released of a click are never events there. An exact serial's binding hides a wildcard's
    /// for the same gesture, as when routing. A plugin action inside a `multi_action` or a `conditional` owns
    /// nothing. Each distinct action is listed once; whether its manifest takes events is the app's to check.
    pub fn plugin_event_targets(&self, device: &DeviceContext, event: &InputEvent) -> Vec<Action> {
        let Ok(device) = device.normalized() else {
            return Vec::new();
        };
        if event.validate_for(device.module).is_err() {
            return Vec::new();
        }
        if device.module == ServiceModule::M131E
            && let Some(functions) = &self.profile().dial_functions
        {
            // A short press switches functions, and raw pressed / released are the parts of a short or long press.
            if matches!(
                event.gesture,
                Gesture::ShortPress | Gesture::Pressed | Gesture::Released
            ) {
                return Vec::new();
            }
            let bindings = functions[self.dial_function]
                .bindings
                .iter()
                .map(|b| (b.gesture, b.serial_number.as_deref(), &b.action));
            return owning_plugin_actions(bindings, &device, event.gesture);
        }
        if device.module == ServiceModule::M4315
            && let Some(layout) = &self.profile().keypad
        {
            let index = event.control as usize;
            let key = (device.serial_number.clone(), index);
            let page_id = match event.gesture {
                Gesture::Pressed if !self.held.contains_key(&key) => self.path.last(),
                Gesture::Released => self
                    .held
                    .get(&key)
                    .filter(|held| held.action_tile)
                    .map(|held| &held.page_id),
                _ => None,
            };
            let Some(KeypadTile::Action { bindings, .. }) = page_id
                .and_then(|page_id| layout.page(page_id))
                .and_then(|page| page.slots.get(index)?.as_ref())
            else {
                return Vec::new();
            };
            let bindings = bindings
                .iter()
                .map(|b| (b.gesture, b.serial_number.as_deref(), &b.action));
            return owning_plugin_actions(bindings, &device, event.gesture);
        }
        let bindings = self
            .profile()
            .bindings
            .iter()
            .filter(|b| b.module == device.module && b.control == event.control)
            .map(|b| (b.gesture, b.serial_number.as_deref(), &b.action));
        owning_plugin_actions(bindings, &device, event.gesture)
    }

    /// Resolves one normalized input without hardware or operating-system I/O.
    ///
    /// This is the compatibility view of [`ProfileSession::route`]: a dial
    /// function switch still advances state but is reported as `Unbound`, so
    /// existing callers never dispatch anything for a short click.
    pub fn resolve(
        &mut self,
        device: &DeviceContext,
        event: &InputEvent,
    ) -> Result<KeypadResolution, String> {
        self.route(device, event).map(Into::into)
    }

    /// Resolves one normalized input, reporting dial function switches explicitly.
    ///
    /// With `dial_functions`, an M131E `short_press` advances to the next
    /// function (wrapping) and returns [`ProfileResolution::DialFunction`]
    /// without a host action. `right`, `left`, and `long_press` select the
    /// current function's binding, preferring an exact serial over a wildcard.
    /// Raw `pressed` and `released` transitions are always unbound.
    pub fn route(
        &mut self,
        device: &DeviceContext,
        event: &InputEvent,
    ) -> Result<ProfileResolution, String> {
        event.validate_for(device.module)?;
        let device = &device.normalized()?;
        if device.module == ServiceModule::M131E
            && let Some(functions) = &self.profile().dial_functions
        {
            let count = functions.len();
            let current = &functions[self.dial_function];
            return Ok(match event.gesture {
                Gesture::ShortPress => {
                    let from_function = current.name.clone();
                    let index = (self.dial_function + 1) % count;
                    let to_function = functions[index].name.clone();
                    self.dial_function = index;
                    ProfileResolution::DialFunction {
                        navigation: DialFunctionNavigation {
                            from_function,
                            to_function,
                            index,
                            count,
                        },
                    }
                }
                Gesture::Right | Gesture::Left | Gesture::LongPress => {
                    match crate::dial::select_action(current, device, event.gesture) {
                        Some(action) => ProfileResolution::Action {
                            action,
                            page_id: None,
                            dial_function: Some(current.name.clone()),
                        },
                        None => ProfileResolution::Unbound,
                    }
                }
                _ => ProfileResolution::Unbound,
            });
        }
        if device.module != ServiceModule::M4315 || self.profile().keypad.is_none() {
            return Ok(match self.engine.action_for(device, event)? {
                Some(action) => ProfileResolution::Action {
                    action,
                    page_id: None,
                    dial_function: None,
                },
                None => ProfileResolution::Unbound,
            });
        }
        let index = event.control as usize;
        match event.gesture {
            Gesture::Pressed => {
                let key = (device.serial_number.clone(), index);
                if self.held.contains_key(&key) {
                    return Ok(ProfileResolution::Unbound);
                }
                let page = self.current_page().expect("validated current keypad page");
                let page_id = page.id.clone();
                let tile = page.slots[index].as_ref();
                let action_tile = matches!(tile, Some(KeypadTile::Action { .. }));
                let back = index == 0 && self.depth() != 0;
                let folder = match tile {
                    Some(KeypadTile::Folder { page_id }) => Some(page_id.clone()),
                    _ => None,
                };
                let action = match tile {
                    Some(KeypadTile::Action { bindings, .. }) => {
                        select_action(bindings, device, Gesture::Pressed)
                    }
                    _ => None,
                };
                self.held.insert(
                    key,
                    HeldKey {
                        page_id: page_id.clone(),
                        action_tile,
                    },
                );
                if back || folder.is_some() {
                    if back {
                        self.path.pop();
                    } else if let Some(folder) = folder {
                        self.path.push(folder);
                    }
                    return Ok(ProfileResolution::Navigation {
                        navigation: KeypadNavigation {
                            from_page: page_id,
                            to_page: self.path.last().expect("root path cannot be empty").clone(),
                            path: self.path.clone(),
                            depth: self.depth(),
                        },
                    });
                }
                Ok(action.map_or(ProfileResolution::Unbound, |action| {
                    ProfileResolution::Action {
                        action,
                        page_id: Some(page_id),
                        dial_function: None,
                    }
                }))
            }
            Gesture::Released => {
                // Only the pressing serial's own release clears or completes its
                // held key; another keypad's release on the same slot is unmatched.
                let Some(held) = self.held.remove(&(device.serial_number.clone(), index)) else {
                    return Ok(ProfileResolution::Unbound);
                };
                if !held.action_tile {
                    return Ok(ProfileResolution::Unbound);
                }
                let page = self
                    .profile()
                    .keypad
                    .as_ref()
                    .expect("keypad routing selected")
                    .page(&held.page_id)
                    .expect("immutable validated origin page");
                let Some(KeypadTile::Action { bindings, .. }) = &page.slots[index] else {
                    return Ok(ProfileResolution::Unbound);
                };
                Ok(select_action(bindings, device, Gesture::Released).map_or(
                    ProfileResolution::Unbound,
                    |action| ProfileResolution::Action {
                        action,
                        page_id: Some(held.page_id),
                        dial_function: None,
                    },
                ))
            }
            _ => unreachable!("keypad selector validation accepts only button transitions"),
        }
    }
}

/// The plugin actions that own a control whose `bindings` (gesture, serial, action) are given, for an input of
/// `gesture` (see [`ProfileSession::plugin_event_targets`]).
fn owning_plugin_actions<'a>(
    bindings: impl Iterator<Item = (Gesture, Option<&'a str>, &'a Action)> + Clone,
    device: &DeviceContext,
    gesture: Gesture,
) -> Vec<Action> {
    let serial = device.serial_number.as_deref();
    // The bindings that apply to this device: an exact serial's, or a wildcard's for a gesture without one.
    let applies = |(bound, selector, _): &(Gesture, Option<&str>, &Action)| match selector {
        Some(_) => *selector == serial,
        None => !bindings.clone().any(|(other, other_serial, _)| {
            other == *bound && other_serial.is_some() && other_serial == serial
        }),
    };
    let applying: Vec<_> = bindings.clone().filter(applies).collect();
    // The gesture runs a binding of its own, and no plugin hears it.
    if applying.iter().any(|(bound, ..)| *bound == gesture) {
        return Vec::new();
    }
    let mut owners: Vec<Action> = Vec::new();
    for (_, _, action) in applying {
        if matches!(action, Action::Plugin { .. }) && !owners.contains(action) {
            owners.push(action.clone());
        }
    }
    owners
}

fn select_action(
    bindings: &[KeypadBinding],
    device: &DeviceContext,
    gesture: Gesture,
) -> Option<Action> {
    let mut wildcard = None;
    for binding in bindings.iter().filter(|binding| binding.gesture == gesture) {
        match binding.serial_number.as_deref() {
            Some(serial) if device.serial_number.as_deref() == Some(serial) => {
                return Some(binding.action.clone());
            }
            None => wildcard = Some(binding.action.clone()),
            Some(_) => {}
        }
    }
    wildcard
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> Profile {
        Profile::from_json(include_bytes!(
            "../../../examples/profiles/keypad-folders.json"
        ))
        .unwrap()
    }
    fn event(control: u8, gesture: Gesture) -> InputEvent {
        InputEvent {
            timestamp_micros: 0,
            control,
            gesture,
            value: None,
        }
    }
    fn device() -> DeviceContext {
        DeviceContext {
            module: ServiceModule::M4315,
            serial_number: None,
        }
    }
    #[test]
    fn navigation_release_never_triggers_new_page_and_back_moves_one_level() {
        let mut session = ProfileSession::new(fixture()).unwrap();
        for expected in [1, 2] {
            assert!(matches!(
                session
                    .resolve(&device(), &event(1, Gesture::Pressed))
                    .unwrap(),
                KeypadResolution::Navigation { .. }
            ));
            assert_eq!(session.depth(), expected);
            assert_eq!(
                session
                    .resolve(&device(), &event(1, Gesture::Released))
                    .unwrap(),
                KeypadResolution::Unbound
            );
        }
        assert_eq!(
            session
                .resolve(&device(), &event(2, Gesture::Released))
                .unwrap(),
            KeypadResolution::Unbound
        );
        session
            .resolve(&device(), &event(2, Gesture::Pressed))
            .unwrap();
        session
            .resolve(&device(), &event(0, Gesture::Pressed))
            .unwrap();
        assert_eq!(session.depth(), 1);
        assert!(
            matches!(session.resolve(&device(), &event(2, Gesture::Released)).unwrap(), KeypadResolution::Action { page_id: Some(id), .. } if id == "git-tools")
        );
        assert_eq!(
            session
                .resolve(&device(), &event(0, Gesture::Released))
                .unwrap(),
            KeypadResolution::Unbound
        );
        session
            .resolve(&device(), &event(0, Gesture::Pressed))
            .unwrap();
        assert_eq!(session.depth(), 0);
    }
    #[test]
    fn duplicate_press_wrong_serial_release_and_reset_preserve_pairing() {
        let mut session = ProfileSession::new(fixture()).unwrap();
        let unit = DeviceContext {
            serial_number: Some("unit".into()),
            ..device()
        };
        assert!(matches!(
            session.resolve(&unit, &event(0, Gesture::Pressed)).unwrap(),
            KeypadResolution::Action { .. }
        ));
        assert_eq!(
            session.resolve(&unit, &event(0, Gesture::Pressed)).unwrap(),
            KeypadResolution::Unbound
        );
        assert_eq!(
            session
                .resolve(&device(), &event(0, Gesture::Released))
                .unwrap(),
            KeypadResolution::Unbound
        );
        session.reset();
        assert_eq!(
            session
                .resolve(&unit, &event(0, Gesture::Released))
                .unwrap(),
            KeypadResolution::Unbound
        );
        assert_eq!(session.depth(), 0);
    }
    #[test]
    fn release_from_another_serial_keeps_the_original_held_key() {
        let mut profile = fixture();
        let Some(KeypadTile::Action { bindings, .. }) =
            &mut profile.keypad.as_mut().unwrap().pages[0].slots[0]
        else {
            panic!("root slot 0 action missing")
        };
        bindings.push(KeypadBinding {
            gesture: Gesture::Released,
            serial_number: None,
            action: Action::ToggleMute,
        });
        let mut session = ProfileSession::new(profile).unwrap();
        let a = DeviceContext {
            serial_number: Some("keypad-a".into()),
            ..device()
        };
        let b = DeviceContext {
            serial_number: Some("keypad-b".into()),
            ..device()
        };
        assert!(matches!(
            session.resolve(&a, &event(0, Gesture::Pressed)).unwrap(),
            KeypadResolution::Action { .. }
        ));
        // B never pressed slot 0, so its release is unmatched and leaves A held.
        assert_eq!(
            session.resolve(&b, &event(0, Gesture::Released)).unwrap(),
            KeypadResolution::Unbound
        );
        // B's own press on the same slot is independent of A's held key.
        assert!(matches!(
            session.resolve(&b, &event(0, Gesture::Pressed)).unwrap(),
            KeypadResolution::Action { .. }
        ));
        // A's release still fires its Released binding.
        assert_eq!(
            session.resolve(&a, &event(0, Gesture::Released)).unwrap(),
            KeypadResolution::Action {
                action: Action::ToggleMute,
                page_id: Some("root".into()),
            }
        );
        // And A's key is no longer held: a duplicate release is unmatched.
        assert_eq!(
            session.resolve(&a, &event(0, Gesture::Released)).unwrap(),
            KeypadResolution::Unbound
        );
        assert!(matches!(
            session.resolve(&b, &event(0, Gesture::Released)).unwrap(),
            KeypadResolution::Action { .. }
        ));
    }
    #[test]
    fn blank_serial_in_struct_literal_pairs_with_absent_serial() {
        let mut session = ProfileSession::new(fixture()).unwrap();
        session
            .set_path(&["root".into(), "development".into(), "git-tools".into()])
            .unwrap();
        let blank = DeviceContext {
            serial_number: Some(" ".into()),
            ..device()
        };
        assert_eq!(
            session
                .resolve(&blank, &event(1, Gesture::Pressed))
                .unwrap(),
            KeypadResolution::Unbound
        );
        assert!(matches!(
            session
                .resolve(&device(), &event(1, Gesture::Released))
                .unwrap(),
            KeypadResolution::Action { .. }
        ));
    }
    #[test]
    fn invalid_tree_shapes_are_rejected() {
        let original = serde_json::to_value(fixture()).unwrap();
        for mutation in 0..8 {
            let mut value = original.clone();
            let pages = value["keypad"]["pages"].as_array_mut().unwrap();
            match mutation {
                0 => pages[1]["id"] = json!("root"),
                1 => pages[0]["slots"][1]["page_id"] = json!("missing"),
                2 => pages[1]["slots"][1]["page_id"] = json!("root"),
                3 => pages[0]["slots"][2] = json!({"type":"folder","page_id":"git-tools"}),
                4 => pages[1]["slots"][0] = pages[0]["slots"][0].clone(),
                5 => pages[0]["slots"][1] = Value::Null,
                6 => {
                    pages[0]["slots"].as_array_mut().unwrap().pop();
                }
                _ => pages[2]["slots"][2]["bindings"][0]["gesture"] = json!("clockwise"),
            }
            assert!(
                Profile::from_json(&serde_json::to_vec(&value).unwrap()).is_err(),
                "mutation {mutation}"
            );
        }
    }
    #[test]
    fn depth_limit_and_invalid_path_do_not_change_current_state() {
        let mut profile = fixture();
        let layout = profile.keypad.as_mut().unwrap();
        let mut id = "git-tools".to_owned();
        for depth in 3..=8 {
            let next = format!("level-{depth}");
            layout.pages.iter_mut().find(|p| p.id == id).unwrap().slots[3] =
                Some(KeypadTile::Folder {
                    page_id: next.clone(),
                });
            layout.pages.push(KeypadPage {
                id: next.clone(),
                name: next.clone(),
                slots: vec![None; 15],
                artwork: Default::default(),
            });
            id = next;
        }
        profile.validate().unwrap();
        let mut session = ProfileSession::new(profile.clone()).unwrap();
        session
            .set_path(&["root".into(), "development".into()])
            .unwrap();
        let path = session.path().to_vec();
        assert!(
            session
                .set_path(&["root".into(), "git-tools".into()])
                .is_err()
        );
        assert_eq!(session.path(), path);
        let layout = profile.keypad.as_mut().unwrap();
        layout.pages.last_mut().unwrap().slots[1] = Some(KeypadTile::Folder {
            page_id: "level-9".into(),
        });
        layout.pages.push(KeypadPage {
            id: "level-9".into(),
            name: "Deep".into(),
            slots: vec![None; 15],
            artwork: Default::default(),
        });
        assert!(profile.validate().is_err());
    }
    #[test]
    fn migration_preserves_selectors_and_other_modules() {
        let profile = Profile::from_json(br#"{"schema_version":1,"name":"Legacy","bindings":[{"module":"m4315","control":14,"gesture":"released","serial_number":"Desk","action":{"type":"toggle_mute"}},{"module":"m003k","control":0,"gesture":"clockwise","action":{"type":"volume","delta":5}}]}"#).unwrap();
        let migrated = profile.clone().into_current().unwrap();
        assert_eq!(migrated.binding_count(), profile.binding_count());
        assert_eq!(migrated.bindings, vec![profile.bindings[1].clone()]);
        let Some(KeypadTile::Action { bindings, .. }) =
            &migrated.keypad.unwrap().pages[0].slots[14]
        else {
            panic!("action missing")
        };
        assert_eq!(bindings[0].serial_number.as_deref(), Some("Desk"));
        assert_eq!(bindings[0].gesture, Gesture::Released);
    }

    fn with_artwork(page: usize, keys: &[(&str, KeyArtwork)]) -> Profile {
        let mut profile = fixture();
        let artwork = &mut profile.keypad.as_mut().unwrap().pages[page].artwork;
        for (key, value) in keys {
            artwork.insert((*key).into(), value.clone());
        }
        profile
    }
    #[test]
    fn artwork_keys_are_canonical_wire_indexes_and_child_back_is_reserved() {
        use crate::key_artwork::tests::{artwork, frame, jpeg};
        let key = artwork(vec![frame(&jpeg(72, 40), 100)]);
        for accepted in ["0", "14"] {
            with_artwork(0, &[(accepted, key.clone())])
                .validate()
                .unwrap();
        }
        for rejected in ["01", "+1", "15", "-1", " 1", "", "x"] {
            let error = with_artwork(0, &[(rejected, key.clone())])
                .validate()
                .unwrap_err();
            assert!(
                error.contains("canonical 0 through 14"),
                "{rejected:?}: {error}"
            );
        }
        // Wire 0 of a child page is its generated Back key; other child keys are open.
        let error = with_artwork(1, &[("0", key.clone())])
            .validate()
            .unwrap_err();
        assert!(error.contains("Back key artwork is reserved"), "{error}");
        with_artwork(1, &[("1", key.clone())]).validate().unwrap();
        let invalid = artwork(vec![frame(&jpeg(72, 40), 99)]);
        assert!(with_artwork(0, &[("3", invalid)]).validate().is_err());
    }
    #[test]
    fn profile_artwork_bytes_are_bounded_across_pages() {
        use crate::key_artwork::tests::{artwork, frame, padded};
        let largest = frame(&padded(40, KeyArtwork::MAX_FRAME_BYTES), 100);
        // 21 full frames fit the 700 KiB budget; a 22nd does not.
        let ten = artwork(vec![largest.clone(); 10]);
        let eleven = artwork(vec![largest; 11]);
        let mut profile = with_artwork(0, &[("1", ten), ("2", eleven.clone())]);
        profile.validate().unwrap();
        profile.keypad.as_mut().unwrap().pages[1]
            .artwork
            .insert("1".into(), eleven.clone());
        let error = profile.validate().unwrap_err();
        assert!(error.contains("700 KiB"), "{error}");
        let error = with_artwork(0, &[("1", eleven.clone()), ("2", eleven)])
            .validate()
            .unwrap_err();
        assert!(error.contains("700 KiB"), "{error}");
        // Base64 of a full budget still leaves room inside the 1 MiB JSON
        // limit, so the byte budget is the limit a profile reaches first.
        const {
            assert!(
                KeyArtwork::MAX_PROFILE_BYTES.div_ceil(3) * 4 < Profile::MAX_JSON_BYTES - 64 * 1024
            )
        };
    }
    #[test]
    fn artwork_round_trips_and_is_omitted_when_absent() {
        use crate::key_artwork::tests::{artwork, design, frame, jpeg};
        let plain = fixture();
        let value = serde_json::to_value(&plain).unwrap();
        for page in value["keypad"]["pages"].as_array().unwrap() {
            assert!(page.get("artwork").is_none());
        }
        let json = serde_json::to_vec(&plain).unwrap();
        assert_eq!(Profile::from_json(&json).unwrap(), plain);

        let mut animated = artwork(vec![frame(&jpeg(72, 40), 100), frame(&jpeg(72, 200), 250)]);
        animated.loop_count = 3;
        let profile = with_artwork(1, &[("4", animated)]);
        let json = serde_json::to_vec(&profile).unwrap();
        assert_eq!(Profile::from_json(&json).unwrap(), profile);

        // A saved design reopens the studio with it and survives the profile
        // round trip; the drawn frames are untouched by it.
        let mut designed = artwork(vec![frame(&jpeg(72, 40), 100)]);
        designed.design = Some(design());
        let profile = with_artwork(0, &[("2", designed)]);
        let value = serde_json::to_value(&profile).unwrap();
        assert_eq!(
            value["keypad"]["pages"][0]["artwork"]["2"]["design"]["text"],
            json!("Mute")
        );
        assert_eq!(
            Profile::from_json(&serde_json::to_vec(&profile).unwrap()).unwrap(),
            profile
        );
    }

    /// The fixture profile at schema 5 with the shared key face fixture's variables, and `faces` on root slot 0.
    fn with_faces(faces: Value) -> Result<Profile, String> {
        let shared: Value =
            serde_json::from_str(include_str!("../tests/fixtures/key-faces.json")).unwrap();
        let mut value = serde_json::to_value(fixture()).unwrap();
        value["schema_version"] = json!(5);
        value["variables"] = shared["variables"].clone();
        value["keypad"]["pages"][0]["slots"][0]["faces"] = faces;
        Profile::from_json(&serde_json::to_vec(&value).unwrap())
    }

    /// Shared with the editor (`draft-model.test.ts`), which holds `keyFaceIssue` to the same cases.
    #[test]
    fn key_faces_follow_the_shared_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/key-faces.json")).unwrap();
        let valid = fixture["valid"].clone();
        let profile = with_faces(valid.clone()).unwrap();
        let Some(KeypadTile::Action { faces, .. }) =
            &profile.keypad.as_ref().unwrap().pages[0].slots[0]
        else {
            panic!("root slot 0 is an action tile");
        };
        assert_eq!(faces.len(), valid.as_array().unwrap().len());
        // Round trip, with absent label or background left out.
        let value = serde_json::to_value(&profile).unwrap();
        assert_eq!(value["keypad"]["pages"][0]["slots"][0]["faces"], valid);
        assert_eq!(
            Profile::from_json(&serde_json::to_vec(&profile).unwrap()).unwrap(),
            profile
        );
        for case in fixture["invalid"].as_array().unwrap() {
            let error = with_faces(json!([case["face"]])).unwrap_err();
            let expected = case["error"].as_str().unwrap();
            assert!(error.contains(expected), "{expected}: {error}");
        }
    }

    #[test]
    fn a_tile_without_faces_serializes_without_the_field_and_holds_at_most_eight() {
        let value = serde_json::to_value(fixture()).unwrap();
        assert!(
            value["keypad"]["pages"][0]["slots"][0]
                .get("faces")
                .is_none()
        );
        let face =
            json!({ "when": { "variable_is": { "name": "live", "value": true } }, "label": "On" });
        assert!(with_faces(json!(vec![face.clone(); 8])).is_ok());
        let error = with_faces(json!(vec![face; 9])).unwrap_err();
        assert!(error.contains("at most 8 key faces"), "{error}");
    }

    #[test]
    fn a_variable_condition_holds_against_the_current_values() {
        use crate::VariableValue::{self, Bool, Number, Text};
        let values = |name: &str| match name {
            "state" => Some(Text("waiting".into())),
            "count" => Some(Number(3)),
            "live" => Some(Bool(true)),
            _ => None,
        };
        let is = |name: &str, value: VariableValue| Condition::VariableIs {
            name: name.into(),
            value,
        };
        let at_least = |name: &str, value: i32| Condition::VariableAtLeast {
            name: name.into(),
            value,
        };
        let cases = [
            (is("state", Text("waiting".into())), Some(true)),
            (is("state", Text("idle".into())), Some(false)),
            (is("live", Bool(false)), Some(false)),
            (is("live", Number(1)), None),
            (is("missing", Bool(true)), None),
            (at_least("count", 3), Some(true)),
            (at_least("count", 4), Some(false)),
            (at_least("state", 1), None),
            (
                Condition::All(vec![is("live", Bool(true)), at_least("count", 4)]),
                Some(false),
            ),
            (
                Condition::Any(vec![is("live", Bool(true)), at_least("count", 4)]),
                Some(true),
            ),
            (
                Condition::Any(vec![is("live", Bool(true)), is("missing", Bool(true))]),
                None,
            ),
            (Condition::Streaming(true), None),
        ];
        for (condition, expected) in cases {
            assert_eq!(condition.holds_with(values), expected, "{condition:?}");
        }
        assert!(
            Condition::Any(vec![is("live", Bool(true)), at_least("count", 1)]).is_variable_only()
        );
        assert!(
            !Condition::All(vec![is("live", Bool(true)), Condition::OutputMuted(true)])
                .is_variable_only()
        );
        // A key face never asks a plugin: it would be asked at every redraw.
        let face: KeyFace = serde_json::from_value(json!({
            "when": {"plugin": {"plugin": "com.x.y", "min_version": "1.0.0", "condition": "on", "value": true}},
            "label": "On"
        }))
        .unwrap();
        assert_eq!(
            face.validate().unwrap_err(),
            "a key face condition can only test variables"
        );
        assert_eq!(face.when.holds_with(values), None);
    }

    fn plugin(action: &str, channel: &str) -> Value {
        json!({"type": "plugin", "plugin": "com.x.talk", "min_version": "1.0.0", "action": action,
            "params": {"channel": channel}})
    }

    fn plugin_profile() -> ProfileSession {
        let mut root: Vec<Value> = vec![Value::Null; 15];
        // Key 0: Talk on press; its release is the plugin's.
        root[0] = json!({"type": "action", "label": "Talk",
            "bindings": [{"gesture": "pressed", "action": plugin("talk", "team")}]});
        // Key 1: Talk on press, an ordinary action on release: nothing is the plugin's.
        root[1] = json!({"type": "action", "label": "Talk and mute", "bindings": [
            {"gesture": "pressed", "action": plugin("talk", "all")},
            {"gesture": "released", "action": {"type": "toggle_mute"}}]});
        // Key 2: the plugin action only as a step of a multi_action: owns nothing.
        root[2] = json!({"type": "action", "label": "Several", "bindings": [{"gesture": "pressed",
            "action": {"type": "multi_action", "actions": [plugin("talk", "team"), {"type": "toggle_mute"}]}}]});
        root[3] = json!({"type": "folder", "page_id": "more"});
        // Key 4: bound on release only, so the press is the plugin's.
        root[4] = json!({"type": "action", "label": "On release",
            "bindings": [{"gesture": "released", "action": plugin("talk", "late")}]});
        let mut more: Vec<Value> = vec![Value::Null; 15];
        more[5] = json!({"type": "action", "label": "Other", "bindings": [{"gesture": "pressed", "action": {"type": "toggle_mute"}}]});
        let profile = json!({"schema_version": 2, "name": "Plugins",
        "bindings": [
            {"module": "m003k", "control": 1, "gesture": "clockwise", "action": plugin("volume", "a")},
            {"module": "m003k", "control": 2, "gesture": "clockwise", "action": plugin("volume", "any")},
            {"module": "m003k", "control": 2, "gesture": "clockwise", "serial_number": "desk", "action": {"type": "volume", "delta": 5}},
            {"module": "m003k", "control": 2, "gesture": "pressed", "action": plugin("press", "any")}
        ],
        "keypad": {"pages": [{"id": "root", "name": "Keys", "slots": root},
            {"id": "more", "name": "More", "slots": more}]},
        "dial_functions": [
            {"name": "Lights", "bindings": [{"gesture": "right", "action": plugin("dim", "desk")}]},
            {"name": "Volume", "bindings": [{"gesture": "right", "action": {"type": "volume", "delta": 5}}]}
        ]});
        ProfileSession::new(Profile::from_json(profile.to_string().as_bytes()).unwrap()).unwrap()
    }

    fn channels(actions: Vec<Action>) -> Vec<String> {
        actions
            .into_iter()
            .map(|action| match action {
                Action::Plugin { action, params, .. } => {
                    let channel = match &params["channel"] {
                        crate::PluginValue::Text(text) => text.clone(),
                        other => panic!("{other:?}"),
                    };
                    format!("{action}:{channel}")
                }
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_keys_unbound_gestures_go_to_the_plugin_action_it_runs_directly() {
        let mut session = plugin_profile();
        let keypad = device();
        let targets = |session: &ProfileSession, control, gesture| {
            channels(session.plugin_event_targets(&keypad, &event(control, gesture)))
        };
        // The press runs the action itself: an invoke, not an event.
        assert!(targets(&session, 0, Gesture::Pressed).is_empty());
        session.route(&keypad, &event(0, Gesture::Pressed)).unwrap();
        assert_eq!(targets(&session, 0, Gesture::Released), ["talk:team"]);
        // A second press while held is a duplicate and goes to nobody.
        assert!(targets(&session, 0, Gesture::Pressed).is_empty());
        session
            .route(&keypad, &event(0, Gesture::Released))
            .unwrap();
        // An unmatched release goes to nobody.
        assert!(targets(&session, 0, Gesture::Released).is_empty());
        // A release bound to another action runs it; the plugin is not told.
        session.route(&keypad, &event(1, Gesture::Pressed)).unwrap();
        assert!(targets(&session, 1, Gesture::Released).is_empty());
        // A plugin action as a step owns nothing.
        session.route(&keypad, &event(2, Gesture::Pressed)).unwrap();
        assert!(targets(&session, 2, Gesture::Released).is_empty());
        // Bound on release only: the press is an event, the release an invoke.
        assert_eq!(targets(&session, 4, Gesture::Pressed), ["talk:late"]);
        session.route(&keypad, &event(4, Gesture::Pressed)).unwrap();
        assert!(targets(&session, 4, Gesture::Released).is_empty());
        // Folder keys belong to no plugin.
        assert!(targets(&session, 3, Gesture::Pressed).is_empty());
    }

    #[test]
    fn a_release_goes_to_the_tile_that_took_the_press_even_after_a_page_change() {
        let mut session = plugin_profile();
        let keypad = device();
        session.route(&keypad, &event(0, Gesture::Pressed)).unwrap();
        // Another key opens the folder while Talk is held.
        session.route(&keypad, &event(3, Gesture::Pressed)).unwrap();
        assert_eq!(session.current_page().unwrap().id, "more");
        assert_eq!(
            channels(session.plugin_event_targets(&keypad, &event(0, Gesture::Released))),
            ["talk:team"]
        );
        // A key pressed on the new page is that page's: key 0 is Back there, and key 4 is empty.
        assert!(
            session
                .plugin_event_targets(&keypad, &event(4, Gesture::Pressed))
                .is_empty()
        );
        // Another keypad's release of the same slot is not the held key's.
        let other = DeviceContext {
            module: ServiceModule::M4315,
            serial_number: Some("other".into()),
        };
        assert!(
            session
                .plugin_event_targets(&other, &event(0, Gesture::Released))
                .is_empty()
        );
    }

    #[test]
    fn flat_controls_follow_serial_precedence() {
        let session = plugin_profile();
        let knobs = |serial: Option<&str>| DeviceContext {
            module: ServiceModule::M003K,
            serial_number: serial.map(str::to_owned),
        };
        let targets = |serial, control, gesture| {
            channels(session.plugin_event_targets(&knobs(serial), &event(control, gesture)))
        };
        assert_eq!(targets(None, 1, Gesture::CounterClockwise), ["volume:a"]);
        assert_eq!(targets(None, 1, Gesture::Pressed), ["volume:a"]);
        assert!(targets(None, 1, Gesture::Clockwise).is_empty());
        assert!(targets(None, 0, Gesture::Clockwise).is_empty());
        // Control 2 has two plugin actions for any serial; listed once each.
        assert_eq!(
            targets(None, 2, Gesture::CounterClockwise),
            ["volume:any", "press:any"]
        );
        // On the desk unit the exact serial's volume hides the wildcard's plugin turn.
        assert_eq!(
            targets(Some("desk"), 2, Gesture::CounterClockwise),
            ["press:any"]
        );
        assert!(targets(Some("desk"), 2, Gesture::Clockwise).is_empty());
    }

    #[test]
    fn the_dial_is_the_current_functions_and_a_short_press_is_never_an_event() {
        let mut session = plugin_profile();
        let dial = DeviceContext {
            module: ServiceModule::M131E,
            serial_number: None,
        };
        let targets = |session: &ProfileSession, gesture| {
            channels(session.plugin_event_targets(&dial, &event(0, gesture)))
        };
        assert_eq!(targets(&session, Gesture::Left), ["dim:desk"]);
        assert!(targets(&session, Gesture::Pressed).is_empty());
        assert!(targets(&session, Gesture::Released).is_empty());
        assert_eq!(targets(&session, Gesture::LongPress), ["dim:desk"]);
        assert!(targets(&session, Gesture::Right).is_empty());
        assert!(targets(&session, Gesture::ShortPress).is_empty());
        session
            .route(&dial, &event(0, Gesture::ShortPress))
            .unwrap();
        assert!(targets(&session, Gesture::Left).is_empty());
    }
}
