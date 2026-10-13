//! "Is there a newer release?" for the Settings page. It runs only when the
//! user presses the button, or on launch when they turned the daily check on.
//! The request is one anonymous GET of GitHub's latest-release record through
//! the system `curl`; nothing about the user, the Mac or the profiles is sent
//! beyond what any HTTPS request carries (address and a user agent naming the
//! app and its version). The app never downloads or installs anything: the
//! user opens the release page and installs by hand.

use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};

const CURL: &str = "/usr/bin/curl";
const OPEN: &str = "/usr/bin/open";
const LATEST_URL: &str = "https://api.github.com/repos/goniszewski/OpenStreamHub/releases/latest";
/// Where "Open the release page" goes. Fixed: the page address is never taken from the response.
const RELEASE_PAGE: &str = "https://github.com/goniszewski/OpenStreamHub/releases/latest";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub available: bool,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

/// `v1.2.3` or `1.2.3` as numbers; anything else (including a pre-release tag) is not a version we compare.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text
        .trim()
        .strip_prefix('v')
        .unwrap_or(text.trim())
        .split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// Compares the running version with a latest-release response body.
pub fn evaluate(current: &str, body: &[u8]) -> Result<UpdateInfo, String> {
    let release: Release = serde_json::from_slice(body)
        .map_err(|_| "GitHub's answer was not a release record.".to_string())?;
    let latest = parse_version(&release.tag_name)
        .ok_or_else(|| "The latest release has a version this app cannot read.".to_string())?;
    let running = parse_version(current)
        .ok_or_else(|| "This app's own version could not be read.".to_string())?;
    Ok(UpdateInfo {
        current: current.to_string(),
        latest: release.tag_name.trim_start_matches('v').to_string(),
        available: latest > running,
    })
}

pub fn check() -> Result<UpdateInfo, String> {
    if !cfg!(target_os = "macos") {
        return Err("checking for updates is supported on macOS only".into());
    }
    let output = Command::new(CURL)
        .args([
            "--silent",
            "--write-out",
            "\n%{http_code}",
            "--proto",
            "=https",
            "--max-time",
            "10",
            "--max-filesize",
            "1048576",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
        ])
        .arg(format!("User-Agent: OpenStreamHub/{CURRENT}"))
        .arg(LATEST_URL)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Could not start the update check.".to_string())?;
    if !output.status.success() {
        return Err(unreachable());
    }
    let (body, status) = split_status(&output.stdout);
    match status {
        Some(200) => {}
        Some(404) => return Err("No public release was found.".into()),
        Some(403 | 429) => {
            return Err("GitHub is limiting requests right now. Try again later.".into());
        }
        _ => return Err(unreachable()),
    }
    evaluate(CURRENT, body)
}

fn unreachable() -> String {
    "Could not reach GitHub. Check your connection and try again.".into()
}

/// Splits curl's output into the body and the status code its `--write-out` appended on the last line.
fn split_status(output: &[u8]) -> (&[u8], Option<u16>) {
    let Some(cut) = output.iter().rposition(|byte| *byte == b'\n') else {
        return (output, None);
    };
    let status = std::str::from_utf8(&output[cut + 1..])
        .ok()
        .and_then(|text| text.trim().parse().ok());
    (&output[..cut], status)
}

pub fn open_release_page() -> Result<(), String> {
    let status = Command::new(OPEN)
        .arg(RELEASE_PAGE)
        .status()
        .map_err(|_| "Could not open the release page.".to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Could not open the release page.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers_not_text() {
        assert_eq!(parse_version("v0.10.2"), Some((0, 10, 2)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("v1.2.3-beta"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn a_newer_tag_is_an_update_and_the_same_or_older_is_not() {
        let body = |tag: &str| format!("{{\"tag_name\":\"{tag}\",\"name\":\"x\"}}").into_bytes();
        let newer = evaluate("0.1.0", &body("v0.1.1")).unwrap();
        assert_eq!((newer.available, newer.latest.as_str()), (true, "0.1.1"));
        assert!(!evaluate("0.1.0", &body("v0.1.0")).unwrap().available);
        assert!(!evaluate("0.2.0", &body("v0.1.9")).unwrap().available);
    }

    #[test]
    fn the_status_code_is_split_off_the_end_of_the_output() {
        assert_eq!(
            split_status(b"{\"a\":1}\n200"),
            (&b"{\"a\":1}"[..], Some(200))
        );
        assert_eq!(split_status(b"\n404"), (&b""[..], Some(404)));
        assert_eq!(split_status(b"no status"), (&b"no status"[..], None));
    }

    #[test]
    fn an_unreadable_answer_is_an_error_not_an_update() {
        assert!(evaluate("0.1.0", b"not json").is_err());
        assert!(evaluate("0.1.0", b"{\"tag_name\":\"nightly\"}").is_err());
        assert!(evaluate("0.1.0", b"{\"message\":\"Not Found\"}").is_err());
    }
}
