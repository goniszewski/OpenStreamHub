//! The real icon of an installed application, offered as key artwork when a
//! key opens that application. `app_icon` resolves a bundle id to the
//! installed .app through LaunchServices and encodes the icon as PNG. The
//! bytes only ever reach the editor's canvas: nothing is uploaded and nothing
//! is sent to a device. `app_icons` does the same for a batch, for the app
//! picker's list rows, whose icons only reach `<img>` elements.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Cap for one PNG app icon; a 128 px photographic icon stays far below it,
/// so anything larger is a pathology, not a real icon.
pub const MAX_ICON_BYTES: usize = 1024 * 1024;
/// Longest accepted bundle id; real ones are far shorter.
pub const MAX_BUNDLE_ID_BYTES: usize = 256;
/// Most bundle ids one `app_icons` call resolves; the picker asks in chunks.
pub const MAX_ICON_BATCH: usize = 64;
/// Cap for all the PNG bytes of one batch. A 64 px icon is a few KiB, so a
/// full batch stays far below it; icons past it are reported as missing.
pub const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppIconRequest {
    pub bundle_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppIconResult {
    pub png_base64: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppIconsRequest {
    pub bundle_ids: Vec<String>,
}

/// Each requested (trimmed) bundle id to its icon, or null when no installed
/// app has it or its icon could not be encoded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppIconsResult {
    pub icons: BTreeMap<String, Option<String>>,
}

fn valid_bundle_id(bundle_id: &str) -> Result<&str, String> {
    let bundle_id = bundle_id.trim();
    if bundle_id.is_empty() {
        return Err("Choose the application in the action first".into());
    }
    if bundle_id.len() > MAX_BUNDLE_ID_BYTES {
        return Err("The bundle id is too long".into());
    }
    Ok(bundle_id)
}

pub fn app_icon(request: &AppIconRequest) -> Result<AppIconResult, String> {
    let bundle_id = valid_bundle_id(&request.bundle_id)?;
    #[cfg(target_os = "macos")]
    return macos::icon_png(bundle_id).map(|png_base64| AppIconResult { png_base64 });
    #[cfg(not(target_os = "macos"))]
    Err("Application icons are only offered on a Mac".into())
}

/// The icons of up to MAX_ICON_BATCH apps. Every id is validated before any
/// lookup, so one bad id rejects the batch; a lookup that fails only leaves
/// that id without an icon (off a Mac, every id).
pub fn app_icons(request: &AppIconsRequest) -> Result<AppIconsResult, String> {
    if request.bundle_ids.len() > MAX_ICON_BATCH {
        return Err(format!(
            "Ask for at most {MAX_ICON_BATCH} app icons at a time"
        ));
    }
    let ids = request
        .bundle_ids
        .iter()
        .map(|id| valid_bundle_id(id))
        .collect::<Result<Vec<_>, _>>()?;
    #[cfg(target_os = "macos")]
    let lookup = |bundle_id: &str| macos::icon_png(bundle_id).ok();
    #[cfg(not(target_os = "macos"))]
    let lookup = |_: &str| None;
    Ok(AppIconsResult {
        icons: batch_icons(&ids, lookup),
    })
}

// Looks each distinct id up once and adds no more icons once the batch's
// bytes would pass MAX_BATCH_BYTES.
fn batch_icons(
    ids: &[&str],
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> BTreeMap<String, Option<String>> {
    let mut icons = BTreeMap::new();
    let mut total = 0;
    for id in ids {
        if icons.contains_key(*id) {
            continue;
        }
        let icon = if total < MAX_BATCH_BYTES {
            lookup(id).filter(|png| {
                total += png.len();
                total <= MAX_BATCH_BYTES
            })
        } else {
            None
        };
        icons.insert((*id).to_string(), icon);
    }
    icons
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

#[cfg(target_os = "macos")]
mod macos {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSString};

    use super::{MAX_ICON_BYTES, is_png};

    /// The edge length worth aiming for: the key shows 72 px, and .icns files
    /// step 16, 32, 64, 128, …, so the nearest representation wins and a
    /// larger one is preferred over a blurrier upscale.
    const TARGET_EDGE: isize = 64;

    /// The installed app's icon as PNG. LaunchServices resolves the bundle id
    /// (including asset-catalog icons, which a hand-rolled .icns reader would
    /// miss). The icon's TIFF carries every stored size up to 1024 px, so all
    /// representations are decoded and the one closest to the key's size is
    /// encoded — TIFF representations are always bitmap representations.
    pub fn icon_png(bundle_id: &str) -> Result<String, String> {
        let workspace = NSWorkspace::sharedWorkspace();
        let identifier = NSString::from_str(bundle_id);
        let app_url = workspace
            .URLForApplicationWithBundleIdentifier(&identifier)
            .ok_or_else(|| {
                format!("No app with the bundle id {bundle_id} is installed on this Mac")
            })?;
        let path = app_url.path().ok_or("The app location has no file path")?;
        let icon = workspace.iconForFile(&path);
        let tiff = icon
            .TIFFRepresentation()
            .ok_or("The app icon has no image data")?;
        let best = NSBitmapImageRep::imageRepsWithData(&tiff)
            .to_vec()
            .into_iter()
            .filter_map(|rep| {
                let (wide, high) = (rep.pixelsWide(), rep.pixelsHigh());
                (wide > 0 && high > 0).then_some(((wide - TARGET_EDGE).abs(), rep))
            })
            .min_by_key(|(distance, rep)| (*distance, isize::from(rep.pixelsWide() < TARGET_EDGE)))
            .map(|(_, rep)| rep)
            .ok_or("The app icon has no usable image")?;
        let bitmap = best
            .downcast::<NSBitmapImageRep>()
            .map_err(|_| "The app icon could not be decoded")?;
        // SAFETY: the properties dictionary is empty, so the generic value
        // type of no entry can be wrong.
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }
        .ok_or("The app icon could not be encoded as PNG")?;
        let bytes = png.to_vec();
        if bytes.len() > MAX_ICON_BYTES {
            return Err("The app icon is unexpectedly large".into());
        }
        if !is_png(&bytes) {
            return Err("The app icon could not be encoded as PNG".into());
        }
        Ok(STANDARD.encode(bytes))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::app_icon::{AppIconRequest, app_icon};

        #[test]
        fn a_known_app_resolves_to_a_png_and_an_unknown_one_is_reported() {
            // TextEdit ships with every macOS install and holds icons up to
            // 1024 px, so this also proves the size pick and the byte cap.
            let icon = app_icon(&AppIconRequest {
                bundle_id: "com.apple.TextEdit".into(),
            })
            .unwrap();
            let bytes = STANDARD.decode(&icon.png_base64).unwrap();
            assert!(bytes.len() <= MAX_ICON_BYTES);
            assert!(is_png(&bytes));
            let error = app_icon(&AppIconRequest {
                bundle_id: "dev.cooltek.missing-app".into(),
            })
            .unwrap_err();
            assert!(error.contains("installed on this Mac"), "{error}");
        }

        #[test]
        fn a_batch_resolves_installed_apps_and_reports_missing_ones_as_null() {
            use crate::app_icon::{AppIconsRequest, app_icons};
            let result = app_icons(&AppIconsRequest {
                bundle_ids: vec![
                    " com.apple.TextEdit ".into(),
                    "dev.cooltek.missing-app".into(),
                ],
            })
            .unwrap();
            let png = result.icons["com.apple.TextEdit"].as_ref().unwrap();
            assert!(is_png(&STANDARD.decode(png).unwrap()));
            assert_eq!(result.icons["dev.cooltek.missing-app"], None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_ids_are_validated_before_any_lookup() {
        assert!(
            app_icon(&AppIconRequest {
                bundle_id: "  ".into()
            })
            .unwrap_err()
            .contains("application")
        );
        assert!(
            app_icon(&AppIconRequest {
                bundle_id: "x".repeat(MAX_BUNDLE_ID_BYTES + 1)
            })
            .unwrap_err()
            .contains("too long")
        );
    }

    #[test]
    fn batches_are_capped_and_validated_before_any_lookup() {
        let many = AppIconsRequest {
            bundle_ids: vec!["com.apple.TextEdit".into(); MAX_ICON_BATCH + 1],
        };
        assert!(app_icons(&many).unwrap_err().contains("at most"));
        let blank = AppIconsRequest {
            bundle_ids: vec!["com.apple.TextEdit".into(), " ".into()],
        };
        assert!(app_icons(&blank).unwrap_err().contains("application"));
        let long = AppIconsRequest {
            bundle_ids: vec!["x".repeat(MAX_BUNDLE_ID_BYTES + 1)],
        };
        assert!(app_icons(&long).unwrap_err().contains("too long"));
        let empty = AppIconsRequest { bundle_ids: vec![] };
        assert!(app_icons(&empty).unwrap().icons.is_empty());
    }

    #[test]
    fn a_batch_looks_each_id_up_once_and_stops_at_the_byte_cap() {
        let mut looked_up = Vec::new();
        let icons = batch_icons(&["a", "b", "a", "missing"], |id| {
            looked_up.push(id.to_string());
            (id != "missing").then(|| format!("png-{id}"))
        });
        assert_eq!(looked_up, ["a", "b", "missing"]);
        assert_eq!(icons.get("a"), Some(&Some("png-a".into())));
        assert_eq!(icons.get("missing"), Some(&None));
        let half = MAX_BATCH_BYTES / 2 + 1;
        let capped = batch_icons(&["a", "b", "c"], |_| Some("x".repeat(half)));
        assert!(capped["a"].is_some());
        assert_eq!(capped["b"], None);
        assert_eq!(capped["c"], None);
    }
}
