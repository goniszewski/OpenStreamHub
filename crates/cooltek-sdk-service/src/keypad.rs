use crate::{
    Action, Binding, BindingEngine, DeviceContext, Gesture, InputEvent, Profile, ServiceModule,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
    },
    /// Enter a child page on a press; its page name supplies the folder label.
    Folder {
        /// ID of a page with exactly one parent.
        page_id: String,
    },
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
            for (slot, tile) in page.slots.iter().enumerate() {
                if let Some(KeypadTile::Action { label, bindings }) = tile {
                    validate_label(label, "tile label")?;
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

#[derive(Clone, Debug)]
struct HeldKey {
    page_id: String,
    serial_number: Option<String>,
    action_tile: bool,
}

/// Stateful profile routing for one physical keypad interface.
///
/// Non-key modules and schema-1 profiles keep ordinary flat binding behavior.
/// Schema-2 keypad presses capture the original page and serial before navigating;
/// their release can resolve only on that original action tile. Navigation keys,
/// empty keys, duplicate presses, and unmatched releases never invoke a child
/// action. Use a separate session for each physical keypad and reset after
/// disconnect, suspension, or replacing the selected profile.
#[derive(Clone, Debug)]
pub struct ProfileSession {
    engine: BindingEngine,
    path: Vec<String>,
    held: [Option<HeldKey>; KeypadLayout::KEY_COUNT],
}

impl ProfileSession {
    /// Validates and owns an immutable profile, starting at root with no held keys.
    pub fn new(profile: Profile) -> Result<Self, String> {
        profile.validate()?;
        Ok(Self {
            engine: BindingEngine::from_validated(profile),
            path: vec![KeypadLayout::ROOT_PAGE_ID.into()],
            held: std::array::from_fn(|_| None),
        })
    }

    /// Returns the immutable validated profile.
    pub fn profile(&self) -> &Profile {
        self.engine.profile()
    }

    /// Returns the original flat selector, or `None` for schema-2 keypad routing.
    pub fn binding_for(&self, device: &DeviceContext, event: &InputEvent) -> Option<&Binding> {
        if device.module == ServiceModule::M4315 && self.profile().keypad.is_some() {
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

    /// Returns to root and forgets held keys without synthetic releases or actions.
    pub fn reset(&mut self) {
        self.path.truncate(1);
        self.clear_held();
    }

    fn clear_held(&mut self) {
        self.held = std::array::from_fn(|_| None);
    }

    /// Resolves one normalized input without hardware or operating-system I/O.
    pub fn resolve(
        &mut self,
        device: &DeviceContext,
        event: &InputEvent,
    ) -> Result<KeypadResolution, String> {
        event.validate_for(device.module)?;
        crate::profile::validate_serial_number(device.serial_number.as_deref())?;
        if device.module != ServiceModule::M4315 || self.profile().keypad.is_none() {
            return Ok(match self.engine.action_for(device, event)? {
                Some(action) => KeypadResolution::Action {
                    action,
                    page_id: None,
                },
                None => KeypadResolution::Unbound,
            });
        }
        let index = event.control as usize;
        match event.gesture {
            Gesture::Pressed => {
                if self.held[index].is_some() {
                    return Ok(KeypadResolution::Unbound);
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
                self.held[index] = Some(HeldKey {
                    page_id: page_id.clone(),
                    serial_number: device.serial_number.clone(),
                    action_tile,
                });
                if back || folder.is_some() {
                    if back {
                        self.path.pop();
                    } else if let Some(folder) = folder {
                        self.path.push(folder);
                    }
                    return Ok(KeypadResolution::Navigation {
                        navigation: KeypadNavigation {
                            from_page: page_id,
                            to_page: self.path.last().expect("root path cannot be empty").clone(),
                            path: self.path.clone(),
                            depth: self.depth(),
                        },
                    });
                }
                Ok(action.map_or(KeypadResolution::Unbound, |action| {
                    KeypadResolution::Action {
                        action,
                        page_id: Some(page_id),
                    }
                }))
            }
            Gesture::Released => {
                // Any release clears the key so a mismatched serial cannot leave it
                // stuck; only a release from the pressing serial may act.
                let Some(held) = self.held[index].take() else {
                    return Ok(KeypadResolution::Unbound);
                };
                if held.serial_number != device.serial_number || !held.action_tile {
                    return Ok(KeypadResolution::Unbound);
                }
                let page = self
                    .profile()
                    .keypad
                    .as_ref()
                    .expect("keypad routing selected")
                    .page(&held.page_id)
                    .expect("immutable validated origin page");
                let Some(KeypadTile::Action { bindings, .. }) = &page.slots[index] else {
                    return Ok(KeypadResolution::Unbound);
                };
                Ok(select_action(bindings, device, Gesture::Released).map_or(
                    KeypadResolution::Unbound,
                    |action| KeypadResolution::Action {
                        action,
                        page_id: Some(held.page_id),
                    },
                ))
            }
            _ => unreachable!("keypad selector validation accepts only button transitions"),
        }
    }
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
    fn release_from_another_serial_clears_the_held_key() {
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
            session
                .resolve(&device(), &event(0, Gesture::Released))
                .unwrap(),
            KeypadResolution::Unbound
        );
        // The key is no longer held: the matching release cannot act late,
        // and a new press is accepted instead of being treated as a duplicate.
        assert_eq!(
            session
                .resolve(&unit, &event(0, Gesture::Released))
                .unwrap(),
            KeypadResolution::Unbound
        );
        assert!(matches!(
            session.resolve(&unit, &event(0, Gesture::Pressed)).unwrap(),
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
        });
        assert!(profile.validate().is_err());
    }
    #[test]
    fn migration_preserves_selectors_and_other_modules() {
        let profile = Profile::from_json(br#"{"schema_version":1,"name":"Legacy","bindings":[{"module":"m4315","control":14,"gesture":"released","serial_number":"Desk","action":{"type":"toggle_mute"}},{"module":"m003k","control":0,"gesture":"clockwise","action":{"type":"volume","delta":5}}]}"#).unwrap();
        let migrated = profile.clone().into_v2().unwrap();
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
}
