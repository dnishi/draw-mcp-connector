//! Dialog asking whether to approve a connection from the canvas (macOS).
//! Shows macOS's standard dialog (AppleScript display dialog) via osascript. Uses no extra crate.

use std::io::Read;
use std::net::IpAddr;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use tracing::warn;

use crate::config;
use crate::lock::lock;
use crate::ui::ApprovalPrompt;

const ALLOW: &str = "Allow";
const DENY: &str = "Deny";

/// Escapes characters that have a special meaning inside an AppleScript string (also used by the About dialog)
pub fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn script(ip: IpAddr) -> String {
    let message = format!("Allow the canvas at {ip} to connect?");
    format!(
        "display dialog {message} with title {title} buttons {{{deny}, {allow}}} \
         default button {allow} cancel button {deny} giving up after {seconds} with icon caution",
        message = quote(&message),
        title = quote(config::APP_NAME),
        deny = quote(DENY),
        allow = quote(ALLOW),
        seconds = config::USER_RESPONSE_TIMEOUT.as_secs(),
    )
}

/// Whether the output means "Allow" was pressed (on deny or timeout, osascript fails or reports gave up:true)
fn allowed(stdout: &str) -> bool {
    stdout.contains(&format!("button returned:{ALLOW}")) && !stdout.contains("gave up:true")
}

/// A shown dialog (the osascript process). Empty if it could not be shown, which counts as a denial
#[derive(Default)]
pub struct Dialog {
    process: Mutex<Option<Child>>,
    /// The answer is written here. `answer` takes it out and reads it
    stdout: Mutex<Option<ChildStdout>>,
}

/// Shows the dialog. Does not wait for the answer
// On OSes other than macOS, this is compiled only for tests
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn ask(ip: IpAddr) -> Dialog {
    // activate: bring the dialog in front of other apps
    let spawned = Command::new("osascript")
        .args(["-e", "activate", "-e", &script(ip)])
        .stdout(Stdio::piped())
        // Do not show the error (User canceled) when denied
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(mut process) => Dialog {
            stdout: Mutex::new(process.stdout.take()),
            process: Mutex::new(Some(process)),
        },
        Err(error) => {
            warn!("許可のダイアログを出せない: {error}");
            Dialog::default()
        }
    }
}

impl ApprovalPrompt for Dialog {
    fn answer(&self) -> bool {
        let Some(mut stdout) = lock(&self.stdout).take() else {
            return false;
        };
        // Reading finishes when a button is pressed, the dialog times out, or close ends the process
        let mut output = String::new();
        let read = stdout.read_to_string(&mut output);
        // Reap the finished process so that a later close never signals a reused PID
        let exited = lock(&self.process).take().map(|mut process| process.wait());
        let succeeded = matches!(exited, Some(Ok(status)) if status.success());
        read.is_ok() && succeeded && allowed(&output)
    }

    fn close(&self) {
        // After the answer is out, `answer` has already taken it, so do nothing
        if let Some(process) = lock(&self.process).as_mut() {
            let _ = process.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn builds_dialog_script() {
        let script = script(Ipv4Addr::new(192, 168, 0, 2).into());
        assert!(
            script.starts_with("display dialog \"Allow the canvas at 192.168.0.2 to connect?\"")
        );
        assert!(script.contains(
            "buttons {\"Deny\", \"Allow\"} default button \"Allow\" cancel button \"Deny\""
        ));
        assert!(script.contains("giving up after 180"));
    }

    #[test]
    fn quotes_special_characters() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn reads_the_answer() {
        assert!(allowed("button returned:Allow, gave up:false\n"));
        assert!(!allowed("button returned:, gave up:true\n"));
        assert!(!allowed(""));
    }
}
