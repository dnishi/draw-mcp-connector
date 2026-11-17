//! Where the app's settings and data folders are.
//!
//! The folder name is the same as `env!("CARGO_BIN_NAME")`. Reading the environment variables is kept
//! apart from building paths from the values read. Tests that change environment variables break each
//! other when run in parallel, so the building part (a pure function) is tested (hence no `std::env::set_var`).

use std::ffi::OsStr;
use std::path::PathBuf;

const NAME: &str = env!("CARGO_BIN_NAME");

/// Folder for settings (such as the first-launch marker). Some if the environment tells
pub fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        build(std::env::var_os("APPDATA").as_deref(), None, None)
    } else if cfg!(target_os = "macos") {
        build(None, std::env::var_os("HOME").as_deref(), Some(MACOS_DIR))
    } else {
        build(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
            Some(".config"),
        )
    }
}

/// Folder for data (such as the saved canvas). Some if the environment tells
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        build(std::env::var_os("LOCALAPPDATA").as_deref(), None, None)
    } else if cfg!(target_os = "macos") {
        build(None, std::env::var_os("HOME").as_deref(), Some(MACOS_DIR))
    } else {
        build(
            std::env::var_os("XDG_DATA_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
            Some(".local/share"),
        )
    }
}

/// Where settings and data go on macOS (relative to $HOME)
const MACOS_DIR: &str = "Library/Application Support";

/// Builds the folder from the environment variable values read. Does not depend on the OS (the
/// differences between OSes come from the values the callers pass, so tests behave the same on every OS).
/// Uses `primary` if present, otherwise `home_fallback` under `home`.
/// An empty string counts as missing (as the XDG Base Directory spec says)
fn build(
    // Main environment variable: APPDATA / LOCALAPPDATA (Windows), XDG_CONFIG_HOME / XDG_DATA_HOME (Linux)
    primary: Option<&OsStr>,
    // $HOME
    home: Option<&OsStr>,
    // Path relative to $HOME when the main variable is missing
    home_fallback: Option<&str>,
) -> Option<PathBuf> {
    let base = non_empty(primary).or_else(|| {
        let home = non_empty(home)?;
        Some(home.join(home_fallback?))
    })?;
    Some(base.join(NAME))
}

fn non_empty(value: Option<&OsStr>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> Option<&OsStr> {
        Some(OsStr::new(value))
    }

    /// The main variable (such as an XDG variable) is used as the folder when present
    #[test]
    fn the_xdg_var_is_used_when_set() {
        assert_eq!(
            build(os("/xdg/config"), os("/home/u"), Some(".config")),
            Some(PathBuf::from(format!("/xdg/config/{NAME}")))
        );
    }

    /// An empty XDG variable counts as missing
    #[test]
    fn an_empty_xdg_var_is_missing() {
        assert_eq!(
            build(os(""), os("/home/u"), Some(".config")),
            Some(PathBuf::from(format!("/home/u/.config/{NAME}")))
        );
    }

    #[test]
    fn the_home_fallback_is_used_when_the_xdg_var_is_missing() {
        assert_eq!(
            build(None, os("/home/u"), Some(".config")),
            Some(PathBuf::from(format!("/home/u/.config/{NAME}")))
        );
    }

    /// macOS uses no main variable, only Library/Application Support under $HOME
    #[test]
    fn macos_uses_the_home_relative_dir() {
        assert_eq!(
            build(None, os("/Users/u"), Some(MACOS_DIR)),
            Some(PathBuf::from(format!(
                "/Users/u/Library/Application Support/{NAME}"
            )))
        );
    }

    /// Windows does not use $HOME, so without the main variable (such as APPDATA) it is None
    #[test]
    fn windows_without_the_var_is_none() {
        assert_eq!(build(None, None, None), None);
    }

    #[test]
    fn nothing_available_is_none() {
        assert_eq!(build(None, None, Some(".config")), None);
    }
}
