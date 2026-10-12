//! What the app does for a demo (the Demos page): a blank profile to run under it, and key pictures from the webview.

use crate::artwork::{Fit, PrepareRequest, Target, prepare_artwork};
use cooltek_sdk_service::Profile;

/// A demo's process reports here, not on the Run panel's `service://` events: it shares the module's service id.
pub const RECORD_EVENT: &str = "demo://record";
pub const STATUS_EVENT: &str = "demo://status";

/// The profile a demo's run holds: no bindings, so nothing a key does reaches the Mac.
pub fn blank_profile() -> Result<Profile, String> {
    let profile = Profile::from_json(br#"{"schema_version":1,"name":"Demo","bindings":[]}"#)
        .map_err(|error| error.to_string())?;
    profile.validate()?;
    Ok(profile)
}

/// A key picture as the webview drew it (a PNG, base64) turned into the 72x72 baseline JPEG the module takes.
pub fn key_jpeg(image_base64: String) -> Result<String, String> {
    prepare_artwork(&PrepareRequest {
        image_base64,
        target: Target::Keypad,
        quality: 90,
        fit: Fit::Stretch,
        quarter_turns: 0,
        background: [0, 0, 0],
    })
    .map(|prepared| prepared.jpeg_base64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use image::ImageEncoder;

    fn png(side: u32) -> String {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                &vec![90; (side * side * 3) as usize],
                side,
                side,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        STANDARD.encode(bytes)
    }

    #[test]
    fn the_blank_profile_binds_nothing() {
        assert_eq!(blank_profile().unwrap().binding_count(), 0);
    }

    #[test]
    fn a_drawn_picture_becomes_a_key_sized_jpeg_and_a_bad_one_is_refused() {
        let jpeg = STANDARD.decode(key_jpeg(png(144)).unwrap()).unwrap();
        cooltek_sdk_masterhub::keypad::validate_jpeg(&jpeg).unwrap();
        assert!(key_jpeg("not an image".into()).is_err());
    }
}
