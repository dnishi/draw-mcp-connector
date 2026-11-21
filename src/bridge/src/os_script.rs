//! OS script parts shared by the approval and export dialogs.
//! Quoting for the script strings, how to build the `Command` for macOS (osascript) and Windows (PowerShell),
//! and a function that runs a command and waits for it. Neither uses an extra crate.

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

/// Escapes the characters that have a special meaning inside an AppleScript string
pub fn apple_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The characters PowerShell treats as a single quote. Besides `'`, the curly quotes end a string too
/// (source: Windows PowerShell Language Specification 3.0, 2.3.5.2 `single-quote-character`,
/// https://learn.microsoft.com/en-us/powershell/scripting/lang-spec/chapter-02)
const PS_SINGLE_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

/// Windows: wraps in a single-quoted PowerShell string. A single quote inside the string stands for
/// one when doubled (same spec), so every character of PS_SINGLE_QUOTES is doubled
pub fn ps_quote(text: &str) -> String {
    let mut quoted = String::from("'");
    for c in text.chars() {
        if PS_SINGLE_QUOTES.contains(&c) {
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted.push('\'');
    quoted
}

/// A PowerShell expression without `"` in it. When the text contains `", it is built
/// by concatenating `$q = [char]34` (defined by the caller).
/// The same as ps_quote when there are no double quotes
pub fn ps_quoted(text: &str) -> String {
    let mut quoted = String::new();
    for part in text.split('"') {
        if !quoted.is_empty() {
            quoted.push_str(" + $q + ");
        }
        quoted.push_str(&ps_quote(part));
    }
    quoted
}

// uType of MessageBoxW (source: Microsoft "MessageBox function (winuser.h)")
pub const MB_OK: u32 = 0x0000_0000;
pub const MB_YESNO: u32 = 0x0000_0004;
pub const MB_ICONWARNING: u32 = 0x0000_0030;
pub const MB_DEFBUTTON2: u32 = 0x0000_0100;
pub const MB_SETFOREGROUND: u32 = 0x0001_0000;
pub const MB_TOPMOST: u32 = 0x0004_0000;

// Create the process without a console window (source: Microsoft "Process Creation Flags")
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The command that shows a dialog (osascript on macOS, PowerShell on Windows).
/// Drops the script of the OS that is not used (the caller builds both)
pub fn command(apple_script: &str, ps_script: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let _ = apple_script;
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", ps_script]);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = ps_script;
        // activate: bring the dialog in front of other apps
        let mut command = Command::new("osascript");
        command.args(["-e", "activate", "-e", apple_script]);
        command
    }
}

/// Runs the command and waits until it finishes. Returns the standard output on success (exit code 0).
/// None on failure or when it cannot be started (stderr is dropped)
pub fn run(mut command: Command) -> Option<String> {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_special_characters() {
        assert_eq!(apple_quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn quotes_special_characters_for_windows() {
        assert_eq!(ps_quote("a'b"), "'a''b'");
        assert_eq!(ps_quote("abc"), "'abc'");
        // The curly quotes end a string too, so they are doubled
        assert_eq!(
            ps_quote("l\u{2019}a \u{2018}b\u{2019} \u{201A}\u{201B}"),
            "'l\u{2019}\u{2019}a \u{2018}\u{2018}b\u{2019}\u{2019} \u{201A}\u{201A}\u{201B}\u{201B}'"
        );
    }

    #[test]
    fn quoted_builds_double_quotes_from_the_q_variable() {
        assert_eq!(ps_quoted("abc"), "'abc'");
        assert_eq!(ps_quoted("a'b"), "'a''b'");
        assert_eq!(ps_quoted(r#"a\"b"#), r#"'a\' + $q + 'b'"#);
        assert_eq!(ps_quoted(r#"a\"b'c"#), r#"'a\' + $q + 'b''c'"#);
        assert_eq!(ps_quoted(r#"a"b""#), r#"'a' + $q + 'b' + $q + ''"#);
    }
}
