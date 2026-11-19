//! Explicit, identity-routed software-mode commands for the two IPS modules.

use cooltek_sdk_masterhub::{ModuleKind, ReportTransport, dial, keypad};

/// Whether the identified module has a software-derived reversible mode command.
pub fn supported(module: Option<ModuleKind>) -> bool {
    matches!(module, Some(ModuleKind::M131E | ModuleKind::M4315))
}

/// Sends one mode command, without querying, initializing, or retrying the device.
pub fn set<T: ReportTransport + ?Sized>(
    transport: &T,
    module: Option<ModuleKind>,
    enabled: bool,
) -> Result<(), String> {
    match module {
        Some(ModuleKind::M131E) => dial::set_software_mode(transport, enabled),
        Some(ModuleKind::M4315) => keypad::set_software_mode(transport, enabled),
        _ => return Err("software mode is supported only for M131E and M4315".into()),
    }
    .map_err(|error| error.to_string())
}

/// Sends the software-mode command and reads report `0x03` back, returning the
/// reply. An empty reply means the module ignored the command: one handed back
/// to hardware mode ignores every later software-mode command until it is
/// unplugged or loses power.
pub fn enter<T: ReportTransport + ?Sized>(
    transport: &T,
    module: Option<ModuleKind>,
) -> Result<Vec<u8>, String> {
    match module {
        Some(ModuleKind::M131E) => dial::enter_software_mode_with_reply(transport),
        Some(ModuleKind::M4315) => keypad::enter_software_mode_with_reply(transport),
        _ => return Err("software mode is supported only for M131E and M4315".into()),
    }
    .map_err(|error| error.to_string())
}
