//! The update notice that follows `lint`, `doctor` and `prime`.
//!
//! The policy (CI, opt-outs, offline mode, the 24-hour cache) lives in `slopvac-update`. This
//! module only asks for the notice and prints it on stderr in the report's text form.

use slopvac_model::{KebabName, NonEmptyString, Notice, NoticeLevel, NoticeMessage};
use slopvac_update::command::Runtime;

/// Commands that show the notice. `slopvac-update` decides which of them may refresh the cache.
const COMMANDS: [&str; 3] = ["lint", "doctor", "prime"];

/// Write the update notice for `command` to `err_out`, when there is one.
///
/// Never fails: a missing notice, a refresh that fails and a write error on stderr all leave the
/// command's result as it was.
pub(crate) fn emit(rt: &Runtime, command: &str, err_out: &mut dyn std::io::Write) {
    if !COMMANDS.contains(&command) {
        return;
    }
    let Some(found) = rt.notice(Some(command)) else {
        return;
    };
    let (Ok(code), Ok(message)) = (
        KebabName::new(found.code),
        NonEmptyString::new(found.message),
    ) else {
        return;
    };
    let notice = Notice::from_catalog(NoticeLevel::Info, NoticeMessage::new(code, message));
    let _ = writeln!(err_out, "{}", slopvac_report::render_notice(&notice));
}
