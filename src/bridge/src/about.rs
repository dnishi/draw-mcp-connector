//! App name, version, and license, plus the source and license of the bundled data, shown by About in the menu
//! (the content of README.md and `assets/kanjivg/README.md` in English; UI text is English).
//! On macOS it brings the app to the front and then shows the standard About panel (orderFrontStandardAboutPanelWithOptions).
//! On Windows it uses muda's About (tray.rs).

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

// Called only from Windows (tray.rs) and macOS (show)
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Text for the Credits (text area) of the macOS About panel. The panel shows the name, version, and copyright
/// separately, so this holds only the license and the source of the bundled data
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn credits() -> String {
    format!("Licensed under the {APP_LICENSE}.\n\n{THIRD_PARTY_NOTICE}")
}

/// Shows the About panel. Does not wait for it to close
#[cfg(target_os = "macos")]
pub fn show() {
    use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{
        NSAboutPanelOptionApplicationName, NSAboutPanelOptionApplicationVersion,
        NSAboutPanelOptionCredits, NSAboutPanelOptionVersion, NSApplication,
    };
    use objc2_foundation::{NSAttributedString, NSDictionary, NSString, ns_string};

    // The menu event loop runs on the main thread (tray.rs), so this is normally Some
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("About のパネルを出せない(メインスレッドで無い)");
        return;
    };

    let app = NSApplication::sharedApplication(mtm);

    // Bring the app to the front. This app is not in the Dock (Accessory), so otherwise
    // the panel may appear behind other apps' windows.
    // activate() is available from macOS 14.0, so earlier versions use the deprecated
    // activateIgnoringOtherApps (as tao 0.37.2 does)
    if objc2::available!(macos = 14.0) {
        app.activate();
    } else {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }

    // SAFETY: only reads the About panel keys defined by AppKit (extern statics)
    let (name_key, app_version_key, version_key, credits_key) = unsafe {
        (
            NSAboutPanelOptionApplicationName,
            NSAboutPanelOptionApplicationVersion,
            NSAboutPanelOptionVersion,
            NSAboutPanelOptionCredits,
        )
    };
    // Dictionary values are all AnyObject
    let text = |value: &str| -> Retained<AnyObject> {
        Retained::into_super(Retained::into_super(NSString::from_str(value)))
    };
    let credits_text = NSAttributedString::from_nsstring(&NSString::from_str(&credits()));
    let options = NSDictionary::from_retained_objects(
        &[
            name_key,
            app_version_key,
            version_key,
            // objc2-app-kit has no constant for the copyright key, so use the name from Apple's documentation (as muda 0.21.2 does)
            ns_string!("Copyright"),
            credits_key,
        ],
        &[
            text(crate::config::APP_NAME),
            text(version()),
            // Empty string: CFBundleVersion in Info.plist is the same version, so do not show it twice as "0.3.0 (0.3.0)"
            text(""),
            text(APP_COPYRIGHT),
            Retained::into_super(Retained::into_super(credits_text)),
        ],
    );
    // SAFETY: all keys are NSString, and the values are types the About panel accepts (NSString for text, NSAttributedString for Credits)
    unsafe { app.orderFrontStandardAboutPanelWithOptions(&options) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_has_app_license_and_kanjivg_attribution() {
        let credits = credits();
        assert!(credits.contains("MIT License"));
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
            assert!(credits.contains(required), "{required}");
        }
    }
}
