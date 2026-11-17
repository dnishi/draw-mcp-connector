//! Dialog asking whether to approve a connection from the canvas (macOS / Windows).
//! On macOS, shows the standard dialog (AppleScript display dialog) via osascript;
//! on Windows, calls the Win32 MessageBoxW (user32.dll) from PowerShell. Neither uses an extra crate.

use std::io::Read;
use std::net::IpAddr;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use tracing::warn;

use crate::config;
use crate::lock::lock;
use crate::ui::ApprovalPrompt;

const ALLOW: &str = "Allow";
const DENY: &str = "Deny";

/// The dialog's message (shared by macOS and Windows)
fn message(ip: IpAddr) -> String {
    format!("Allow the canvas at {ip} to connect?")
}

/// Escapes characters that have a special meaning inside an AppleScript string
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn apple_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// macOS: the AppleScript that shows the approval dialog
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn apple_script(ip: IpAddr) -> String {
    let text = message(ip);
    // Make Deny the default button so that pressing Enter does not approve
    format!(
        "display dialog {text} with title {title} buttons {{{deny}, {allow}}} \
         default button {deny} cancel button {deny} giving up after {seconds} with icon caution",
        text = apple_quote(&text),
        title = apple_quote(config::APP_NAME),
        deny = apple_quote(DENY),
        allow = apple_quote(ALLOW),
        seconds = config::USER_RESPONSE_TIMEOUT.as_secs(),
    )
}

/// macOS: whether the output means "Allow" was pressed (on deny or timeout, osascript fails or reports gave up:true)
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn apple_allowed(stdout: &str) -> bool {
    stdout.contains(&format!("button returned:{ALLOW}")) && !stdout.contains("gave up:true")
}

// uType of MessageBoxW (source: Microsoft "MessageBox function (winuser.h)")
const MB_YESNO: u32 = 0x0000_0004;
const MB_ICONWARNING: u32 = 0x0000_0030;
const MB_DEFBUTTON2: u32 = 0x0000_0100;
const MB_SETFOREGROUND: u32 = 0x0001_0000;
const MB_TOPMOST: u32 = 0x0004_0000;
/// Value MessageBoxW returns for Yes (IDYES)
const ID_YES: i32 = 6;

/// Flags used to show the dialog.
/// No is the default so that pressing Enter does not approve. MB_TOPMOST creates a WS_EX_TOPMOST window,
/// so the dialog does not hide behind other windows. Windows restricts when MB_SETFOREGROUND can make it
/// the foreground window (while the user works in another window, it may only flash the taskbar button;
/// source: Microsoft "SetForegroundWindow function"), but MB_TOPMOST works independently of that
const WINDOWS_DIALOG_FLAGS: u32 =
    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_TOPMOST | MB_SETFOREGROUND;

/// Windows: inside a single-quoted PowerShell string, `''` stands for one `'`
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Windows: the PowerShell script that shows the Win32 MessageBoxW and writes out the answer's value.
/// `$q = [char]34` is the double quote character. C#'s `DllImport("user32.dll")` contains double quotes,
/// so it is built inside PowerShell to keep double quotes off the command line
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn ps_script(ip: IpAddr) -> String {
    format!(
        "$q = [char]34; \
         Add-Type -Namespace DrawMcp -Name Native -MemberDefinition ('[DllImport(' + $q + 'user32.dll' + $q + ', CharSet = CharSet.Unicode)] public static extern int MessageBoxW(IntPtr hWnd, string lpText, string lpCaption, uint uType);'); \
         $answer = [DrawMcp.Native]::MessageBoxW([IntPtr]::Zero, {text}, {title}, {flags}); \
         Write-Output $answer",
        text = ps_quote(&message(ip)),
        title = ps_quote(config::APP_NAME),
        flags = WINDOWS_DIALOG_FLAGS,
    )
}

/// Windows: whether the answer on stdout is Yes (6) (compared after trimming whitespace and newlines; No (7), empty, or anything else is a denial)
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn ps_allowed(stdout: &str) -> bool {
    stdout.trim() == ID_YES.to_string()
}

/// Checks whether stdout means approval (per OS)
fn is_allowed(stdout: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        ps_allowed(stdout)
    }
    #[cfg(not(target_os = "windows"))]
    {
        apple_allowed(stdout)
    }
}

// Create the process without a console window (source: Microsoft "Process Creation Flags")
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The command that shows the approval dialog (osascript on macOS, PowerShell on Windows)
// On OSes other than macOS and Windows, this is compiled only for tests on the development Linux
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn command(ip: IpAddr) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &ps_script(ip)]);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(target_os = "windows"))]
    {
        // activate: bring the dialog in front of other apps
        let mut command = Command::new("osascript");
        command.args(["-e", "activate", "-e", &apple_script(ip)]);
        command
    }
}

/// A shown dialog (the osascript or PowerShell process). Empty if it could not be shown, which counts as a denial
#[derive(Default)]
pub struct Dialog {
    process: Mutex<Option<Child>>,
    /// The answer is written here. `answer` takes it out and reads it
    stdout: Mutex<Option<ChildStdout>>,
}

/// Shows the dialog. Does not wait for the answer
// On OSes other than macOS and Windows, this is compiled only for tests on the development Linux
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub fn ask(ip: IpAddr) -> Dialog {
    let spawned = command(ip)
        .stdout(Stdio::piped())
        // Do not show the output on denial or error
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
        read.is_ok() && succeeded && is_allowed(&output)
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
        let script = apple_script(Ipv4Addr::new(192, 168, 0, 2).into());
        assert!(
            script.starts_with("display dialog \"Allow the canvas at 192.168.0.2 to connect?\"")
        );
        assert!(script.contains(
            "buttons {\"Deny\", \"Allow\"} default button \"Deny\" cancel button \"Deny\""
        ));
        assert!(script.contains("giving up after 180"));
    }

    #[test]
    fn quotes_special_characters() {
        assert_eq!(apple_quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn reads_the_answer() {
        assert!(apple_allowed("button returned:Allow, gave up:false\n"));
        assert!(!apple_allowed("button returned:, gave up:true\n"));
        assert!(!apple_allowed(""));
    }

    #[test]
    fn builds_windows_script() {
        let script = ps_script(Ipv4Addr::new(192, 168, 0, 2).into());
        assert!(script.contains("'Allow the canvas at 192.168.0.2 to connect?'"));
        assert!(script.contains("'Draw MCP Connector'"));
        assert!(script.contains("user32.dll"));
        assert!(script.contains("MessageBoxW"));
        assert!(script.contains("[IntPtr]::Zero"));
        assert!(script.contains(&WINDOWS_DIALOG_FLAGS.to_string()));
        assert!(script.contains("Write-Output $answer"));
        assert!(!script.contains('"'));
        assert!(!script.contains('\n'));
        assert!(!script.contains("System.Windows.Forms"));
    }

    #[test]
    fn windows_dialog_flags_are_topmost() {
        // 0x00000004 + 0x00000030 + 0x00000100 + 0x00010000 + 0x00040000
        assert_eq!(WINDOWS_DIALOG_FLAGS, 0x0005_0134);
    }

    #[test]
    fn quotes_special_characters_for_windows() {
        assert_eq!(ps_quote("a'b"), "'a''b'");
        assert_eq!(ps_quote("abc"), "'abc'");
    }

    #[test]
    fn reads_the_windows_answer() {
        assert!(ps_allowed("6\r\n"));
        assert!(!ps_allowed("7\r\n"));
        assert!(!ps_allowed(""));
        assert!(!ps_allowed("Yes"));
        assert!(!ps_allowed("60"));
    }
}
