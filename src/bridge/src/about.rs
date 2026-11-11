//! App name, version, and license, plus the source and license of the bundled data, shown by About in the menu
//! (the content of README.md and `assets/kanjivg/README.md` in English; UI text is English).
//! On macOS it uses the standard dialog (AppleScript display dialog). On Windows it uses muda's About (tray.rs).

use crate::config;

/// License of the app itself (LICENSE in the repository)
pub const APP_LICENSE: &str = "MIT License";
/// Copyright notice of the app (same as LICENSE)
pub const APP_COPYRIGHT: &str = "Copyright (c) 2026 dnishi";

/// Pointer to the licenses of the bundled crates, and the source, license, and modifications of the bundled glyph data (KanjiVG).
/// The full crate license texts are put into the .app's Resources at build time (tools/third_party_licenses.py).
/// The KanjiVG notice is assets/kanjivg/NOTICE.txt
pub const THIRD_PARTY_NOTICE: &str = concat!(
    "Licenses of the open source software in this app: THIRD-PARTY-LICENSES.txt in the app's Resources folder.\n\n",
    include_str!("../assets/kanjivg/NOTICE.txt"),
);

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Full text shown by About
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn text() -> String {
    format!(
        "{name} {version}\n{APP_COPYRIGHT}\nLicensed under the {APP_LICENSE}.\n\n{THIRD_PARTY_NOTICE}",
        name = config::APP_NAME,
        version = version(),
    )
}

/// AppleScript expression. Each line becomes a string joined with linefeed (not relying on how newlines are written in strings)
#[cfg(any(test, target_os = "macos"))]
fn script() -> String {
    let message = text()
        .lines()
        .map(crate::approval_dialog::quote)
        .collect::<Vec<_>>()
        .join(" & linefeed & ");
    format!(
        "display dialog ({message}) with title {title} buttons {{\"OK\"}} default button \"OK\"",
        title = crate::approval_dialog::quote(&format!("About {}", config::APP_NAME)),
    )
}

/// Shows the About dialog. Does not wait for it to close
#[cfg(target_os = "macos")]
pub fn show() {
    // activate: bring the dialog in front of other apps (same as the approval dialog)
    let spawned = std::process::Command::new("osascript")
        .args(["-e", "activate", "-e", &script()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        // Clean up the process once it is closed (does not block the event loop)
        Ok(mut process) => {
            std::thread::spawn(move || process.wait());
        }
        Err(error) => tracing::warn!("About のダイアログを出せない: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_has_app_license_and_kanjivg_attribution() {
        let text = text();
        assert!(text.starts_with(&format!("Draw MCP Connector {}\n", version())));
        assert!(text.contains("MIT License"));
        // Same as the copyright notice in LICENSE
        let license = include_str!("../../../LICENSE");
        assert!(license.lines().any(|line| line == APP_COPYRIGHT));
        for required in [
            "KanjiVG",
            "Ulrich Apel",
            "https://kanjivg.tagaini.net",
            "Attribution-Share Alike 3.0",
            "https://creativecommons.org/licenses/by-sa/3.0/",
            "r20260714",
            "THIRD-PARTY-LICENSES.txt",
        ] {
            assert!(text.contains(required), "{required}");
        }
    }

    #[test]
    fn script_joins_quoted_lines_with_linefeed() {
        let script = script();
        assert!(script.starts_with("display dialog (\"Draw MCP Connector "));
        assert!(script.contains(
            "\" & linefeed & \"Copyright (c) 2026 dnishi\" & linefeed & \"Licensed under the MIT License.\""
        ));
        // Blank lines become empty strings
        assert!(script.contains("& linefeed & \"\" & linefeed &"));
        assert!(script.ends_with(
            "with title \"About Draw MCP Connector\" buttons {\"OK\"} default button \"OK\""
        ));
        assert!(!script.contains('\n'), "改行は linefeed で表す");
    }
}
