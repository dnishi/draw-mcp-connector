//! Shows the OS dialogs for exporting (format, target, save path, message, time stamp).
//! Like the approval dialog (approval_dialog.rs), on macOS it uses osascript,
//! on Windows PowerShell, and neither uses a new crate.
//! All of them block until there is an answer, so they are not called from the main thread (the event loop)

use std::path::PathBuf;
#[cfg(not(target_os = "windows"))]
use std::process::Command;

use crate::config;
use crate::export::{Format, Target};
use crate::os_script;

// The UI strings (in English)
const FORMAT_PROMPT: &str = "Export as:";
const TARGET_PROMPT: &str = "Export which sheets?";
const SAVE_PROMPT: &str = "Export";
const PNG_CHOICE: &str = "PNG";
const PDF_CHOICE: &str = "PDF";
const CURRENT_SHEET_CHOICE: &str = "Current Sheet";
const ALL_SHEETS_CHOICE: &str = "All Sheets";

/// macOS: an AppleScript that chooses from the two choices (the default is first. Cancel prints `false` to stdout)
fn choice_apple_script(prompt: &str, first: &str, second: &str) -> String {
    format!(
        "choose from list {{{first}, {second}}} with title {title} with prompt {prompt} \
         default items {{{first}}}",
        first = os_script::apple_quote(first),
        second = os_script::apple_quote(second),
        title = os_script::apple_quote(config::APP_NAME),
        prompt = os_script::apple_quote(prompt),
    )
}

/// macOS: an AppleScript that chooses the format
fn format_apple_script() -> String {
    choice_apple_script(FORMAT_PROMPT, PNG_CHOICE, PDF_CHOICE)
}

/// macOS: an AppleScript that chooses the target of the PDF
fn target_apple_script() -> String {
    choice_apple_script(TARGET_PROMPT, CURRENT_SHEET_CHOICE, ALL_SHEETS_CHOICE)
}

/// macOS: an AppleScript that chooses the save path (Cancel makes osascript fail)
fn save_apple_script(default_name: &str) -> String {
    format!(
        "POSIX path of (choose file name with prompt {prompt} with title {title} \
         default name {name})",
        prompt = os_script::apple_quote(SAVE_PROMPT),
        title = os_script::apple_quote(SAVE_PROMPT),
        name = os_script::apple_quote(default_name),
    )
}

/// macOS: an AppleScript that shows a message (OK only)
fn message_apple_script(text: &str) -> String {
    format!(
        "display dialog {text} buttons {{\"OK\"}} default button \"OK\" with title {title}",
        text = os_script::apple_quote(text),
        title = os_script::apple_quote(config::APP_NAME),
    )
}

/// Windows: a Form with one button for each choice and a Cancel button.
/// TopMost, centered on the screen, not resizable. Prints the text of the pressed button;
/// closing or Cancel prints nothing
fn choice_ps_script(prompt: &str, first: &str, second: &str) -> String {
    format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
         Add-Type -AssemblyName System.Windows.Forms; \
         $form = New-Object System.Windows.Forms.Form; \
         $form.Text = {title}; \
         $form.TopMost = $true; \
         $form.FormBorderStyle = 'FixedDialog'; \
         $form.StartPosition = 'CenterScreen'; \
         $form.ShowInTaskbar = $false; \
         $label = New-Object System.Windows.Forms.Label; \
         $label.Text = {prompt}; \
         $label.AutoSize = $true; \
         $label.Location = New-Object System.Drawing.Point(10, 10); \
         $first = New-Object System.Windows.Forms.Button; \
         $first.Text = {first}; \
         $first.Location = New-Object System.Drawing.Point(10, 40); \
         $first.Size = New-Object System.Drawing.Size(110, 28); \
         $first.Add_Click({{ $form.Tag = {first}; $form.Close(); }}); \
         $second = New-Object System.Windows.Forms.Button; \
         $second.Text = {second}; \
         $second.Location = New-Object System.Drawing.Point(130, 40); \
         $second.Size = New-Object System.Drawing.Size(110, 28); \
         $second.Add_Click({{ $form.Tag = {second}; $form.Close(); }}); \
         $cancel = New-Object System.Windows.Forms.Button; \
         $cancel.Text = 'Cancel'; \
         $cancel.Location = New-Object System.Drawing.Point(250, 40); \
         $cancel.Size = New-Object System.Drawing.Size(110, 28); \
         $cancel.Add_Click({{ $form.Close(); }}); \
         $form.Controls.Add($label); \
         $form.Controls.Add($first); \
         $form.Controls.Add($second); \
         $form.Controls.Add($cancel); \
         $form.ClientSize = New-Object System.Drawing.Size(370, 80); \
         [void]$form.ShowDialog(); \
         if ($form.Tag) {{ Write-Output $form.Tag }}",
        title = os_script::ps_quote(config::APP_NAME),
        prompt = os_script::ps_quote(prompt),
        first = os_script::ps_quote(first),
        second = os_script::ps_quote(second),
    )
}

/// Windows: a PowerShell that chooses the format (prints the text of the pressed button)
fn format_ps_script() -> String {
    choice_ps_script(FORMAT_PROMPT, PNG_CHOICE, PDF_CHOICE)
}

/// Windows: a PowerShell that chooses the target of the PDF (prints the text of the pressed button)
fn target_ps_script() -> String {
    choice_ps_script(TARGET_PROMPT, CURRENT_SHEET_CHOICE, ALL_SHEETS_CHOICE)
}

/// Windows: a PowerShell (SaveFileDialog) that chooses the save path. It puts the dialog in
/// front of an invisible TopMost Form as its owner. If OK, it prints the path (ShowDialog
/// returns a DialogResult, and Cancel is also a value other than 0, so it is compared with OK)
fn save_ps_script(default_name: &str, format: Format) -> String {
    let filter = match format {
        Format::Png => "PNG image (*.png)|*.png",
        Format::Pdf => "PDF document (*.pdf)|*.pdf",
    };
    format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
         Add-Type -AssemblyName System.Windows.Forms; \
         $q = [char]34; \
         $dialog = New-Object System.Windows.Forms.SaveFileDialog; \
         $dialog.FileName = {name}; \
         $dialog.Filter = {filter}; \
         $dialog.AddExtension = $true; \
         $dialog.OverwritePrompt = $true; \
         $owner = New-Object System.Windows.Forms.Form; \
         $owner.Visible = $false; \
         $owner.TopMost = $true; \
         if ($dialog.ShowDialog($owner) -eq [System.Windows.Forms.DialogResult]::OK) {{ Write-Output $dialog.FileName }}",
        name = os_script::ps_quoted(default_name),
        filter = os_script::ps_quote(filter),
    )
}

/// Windows: the flags for the message (OK only). Unlike the approval dialog, OK only, not Yes/No
const MESSAGE_FLAGS: u32 = os_script::MB_OK
    | os_script::MB_ICONWARNING
    | os_script::MB_TOPMOST
    | os_script::MB_SETFOREGROUND;

/// Windows: a PowerShell that shows a message (OK only) (the same MessageBoxW as the approval dialog)
fn message_ps_script(text: &str) -> String {
    format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
         $q = [char]34; \
         Add-Type -Namespace DrawMcp -Name Native -MemberDefinition ('[DllImport(' + $q + 'user32.dll' + $q + ', CharSet = CharSet.Unicode)] public static extern int MessageBoxW(IntPtr hWnd, string lpText, string lpCaption, uint uType);'); \
         [void][DrawMcp.Native]::MessageBoxW([IntPtr]::Zero, {text}, {title}, {flags})",
        text = os_script::ps_quoted(text),
        title = os_script::ps_quote(config::APP_NAME),
        flags = MESSAGE_FLAGS,
    )
}

/// Windows: a PowerShell that prints the local time in the shape "YYYY-MM-DD-HHMM"
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn stamp_ps_script() -> &'static str {
    "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-Date -Format yyyy-MM-dd-HHmm"
}

/// Chooses the format. Cancel is None
pub fn choose_format() -> Option<Format> {
    let stdout = os_script::run(os_script::command(
        &format_apple_script(),
        &format_ps_script(),
    ))?;
    choice_to_format(&stdout)
}

/// Chooses the target of the PDF. Cancel is None
pub fn choose_target() -> Option<Target> {
    let stdout = os_script::run(os_script::command(
        &target_apple_script(),
        &target_ps_script(),
    ))?;
    choice_to_target(&stdout)
}

/// Chooses the save path (the default name default_name). Cancel is None.
/// The extension is fixed on the caller side with with_extension
pub fn choose_save_path(default_name: &str, format: Format) -> Option<PathBuf> {
    let stdout = os_script::run(os_script::command(
        &save_apple_script(default_name),
        &save_ps_script(default_name, format),
    ))?;
    path_from_output(&stdout)
}

/// Shows a message (OK only). Used for errors and guidance
pub fn show_message(text: &str) {
    let command = os_script::command(&message_apple_script(text), &message_ps_script(text));
    let _ = os_script::run(command);
}

/// The local time in the shape "YYYY-MM-DD-HHMM". None if it cannot be obtained
pub fn local_stamp() -> Option<String> {
    #[cfg(target_os = "windows")]
    let command = os_script::command("", stamp_ps_script());
    #[cfg(not(target_os = "windows"))]
    // macOS uses the date command
    let command = {
        let mut command = Command::new("date");
        // The format argument of date starts with a plus sign
        command.arg("+%Y-%m-%d-%H%M");
        command
    };
    let stdout = os_script::run(command)?;
    stamp_from_output(&stdout)
}

/// The output of a choice. Accepts only what exactly matches a choice after removing the
/// whitespace and newlines around it (`false`, empty, or other text is None)
fn choice_to_format(stdout: &str) -> Option<Format> {
    match stdout.trim() {
        PNG_CHOICE => Some(Format::Png),
        PDF_CHOICE => Some(Format::Pdf),
        _ => None,
    }
}

/// The output of a choice (the same rule as choice_to_format above)
fn choice_to_target(stdout: &str) -> Option<Target> {
    match stdout.trim() {
        CURRENT_SHEET_CHOICE => Some(Target::CurrentSheet),
        ALL_SHEETS_CHOICE => Some(Target::AllSheets),
        _ => None,
    }
}

/// The output of a path. Removes the newlines around it. None if empty
fn path_from_output(stdout: &str) -> Option<PathBuf> {
    let path = stdout.trim_matches(|c| c == '\n' || c == '\r');
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

/// The output of the time. None unless it has the shape "YYYY-MM-DD-HHMM" (15 characters).
/// Checks the characters by hand, without a regular expression crate
fn stamp_from_output(stdout: &str) -> Option<String> {
    let stamp = stdout.trim_matches(|c| c == '\n' || c == '\r');
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 13, 14];
    let ok = stamp.len() == 15
        && [4, 7, 10].iter().all(|i| stamp.as_bytes()[*i] == b'-')
        && digits.iter().all(|i| stamp.as_bytes()[*i].is_ascii_digit());
    if ok { Some(stamp.to_string()) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_format_script_has_the_prompt_the_choices_and_the_default() {
        let apple = format_apple_script();
        assert!(apple.contains("with title \"Draw MCP Connector\""));
        assert!(apple.contains("with prompt \"Export as:\""));
        assert!(apple.contains("choose from list {\"PNG\", \"PDF\"}"));
        assert!(apple.contains("default items {\"PNG\"}"));

        let ps = format_ps_script();
        assert!(ps.contains("$form.Text = 'Draw MCP Connector'"));
        assert!(ps.contains("$label.Text = 'Export as:'"));
        assert!(ps.contains("$first.Text = 'PNG'"));
        assert!(ps.contains("$second.Text = 'PDF'"));
        assert!(ps.contains("$cancel.Text = 'Cancel'"));
        assert!(!ps.contains('"'));
    }

    #[test]
    fn the_target_script_defaults_to_the_current_sheet() {
        let apple = target_apple_script();
        assert!(apple.contains("with prompt \"Export which sheets?\""));
        assert!(apple.contains("choose from list {\"Current Sheet\", \"All Sheets\"}"));
        assert!(apple.contains("default items {\"Current Sheet\"}"));

        let ps = target_ps_script();
        assert!(ps.contains("$label.Text = 'Export which sheets?'"));
        assert!(ps.contains("$first.Text = 'Current Sheet'"));
        assert!(ps.contains("$second.Text = 'All Sheets'"));
        assert!(!ps.contains('"'));
    }

    #[test]
    fn the_save_script_escapes_the_default_name() {
        let name = "a\"b'c\\d";
        let apple = save_apple_script(name);
        assert!(apple.contains("with prompt \"Export\" with title \"Export\""));
        assert!(apple.contains(&format!("default name {}", os_script::apple_quote(name))));

        let ps = save_ps_script(name, Format::Png);
        assert!(ps.contains(&format!(
            "$dialog.FileName = {}",
            os_script::ps_quoted(name)
        )));
        assert!(ps.contains("PNG image (*.png)|*.png"));
        assert!(!ps.contains('"'));
        assert!(save_ps_script("n", Format::Pdf).contains("PDF document (*.pdf)|*.pdf"));
    }

    #[test]
    fn the_message_script_escapes_the_text() {
        let text = "Couldn't export: a\"b";
        let apple = message_apple_script(text);
        assert!(apple.contains(&format!("display dialog {}", os_script::apple_quote(text))));
        assert!(apple.contains("buttons {\"OK\"} default button \"OK\""));

        let ps = message_ps_script(text);
        assert!(ps.contains("MessageBoxW"));
        assert!(ps.contains(&format!(
            "[IntPtr]::Zero, {}, {}",
            os_script::ps_quoted(text),
            os_script::ps_quote(config::APP_NAME)
        )));
        assert!(!ps.contains('"'));
    }

    #[test]
    fn the_choice_output_is_the_trimmed_choice_or_none() {
        assert_eq!(choice_to_format("PNG\n"), Some(Format::Png));
        assert_eq!(choice_to_format("  PDF \r\n"), Some(Format::Pdf));
        assert_eq!(choice_to_format("false"), None, "Cancel は標準出力が false");
        assert_eq!(choice_to_format(""), None);
        assert_eq!(choice_to_format("PNGG"), None);
        assert_eq!(choice_to_target("All Sheets\n"), Some(Target::AllSheets));
        assert_eq!(choice_to_target("false"), None);
        assert_eq!(choice_to_target("Current"), None);
    }

    #[test]
    fn the_path_output_strips_the_trailing_newline() {
        assert_eq!(
            path_from_output("/Users/a/Draw.png\n"),
            Some(PathBuf::from("/Users/a/Draw.png"))
        );
        assert_eq!(
            path_from_output("C:\\Users\\a\\Draw.png\r\n"),
            Some(PathBuf::from("C:\\Users\\a\\Draw.png"))
        );
        assert_eq!(
            path_from_output("  with spaces  \n"),
            Some(PathBuf::from("  with spaces  "))
        );
        assert_eq!(path_from_output("\r\n"), None);
        assert_eq!(path_from_output(""), None);
    }

    #[test]
    fn the_stamp_output_must_have_the_shape() {
        assert_eq!(
            stamp_from_output("2026-11-19-1530\n"),
            Some("2026-11-19-1530".to_string())
        );
        assert_eq!(
            stamp_from_output("2026-11-19-1530\r\n"),
            Some("2026-11-19-1530".to_string())
        );
        assert_eq!(stamp_from_output("2026-1-19-1530\n"), None);
        assert_eq!(stamp_from_output("2026-11-19-153\n"), None);
        assert_eq!(stamp_from_output("2026-11-19-15300\n"), None);
        assert_eq!(stamp_from_output("2026/11/19/1530\n"), None);
        assert_eq!(stamp_from_output("2026-11-19-153a\n"), None);
        assert_eq!(stamp_from_output(""), None);
    }
}
