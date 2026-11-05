//! The Homebrew-installed SiliconMetaTrader5.app and its copy in Applications.
//!
//! Homebrew formulae cannot write to /Applications, and Spotlight / Launchpad
//! ignore symbolic links, so `smt5 app-install` (or the app itself) copies the
//! bundle there. After `brew upgrade` the copy is outdated; the app notices
//! and offers to refresh it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::exec::{Cmd, Runner};

pub const BUNDLE_ID: &str = "com.siliconmt5.desktop";
pub const APP_NAME: &str = "SiliconMetaTrader5.app";
/// The bundle name before 1.3.0 was renamed (replaced by `install`).
pub const LEGACY_APP_NAME: &str = "SiliconMT5.app";

/// Value of a `<key>K</key><string>V</string>` pair in an XML Info.plist.
pub fn plist_string(plist: &str, key: &str) -> Option<String> {
    let at = plist.find(&format!("<key>{key}</key>"))?;
    let rest = &plist[at..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    Some(rest[start..start + end].trim().to_owned())
}

fn info(app: &Path) -> Option<String> {
    std::fs::read_to_string(app.join("Contents/Info.plist")).ok()
}

/// CFBundleShortVersionString of a SiliconMetaTrader5 bundle (None for other apps).
pub fn bundle_version(app: &Path) -> Option<String> {
    let plist = info(app)?;
    (plist_string(&plist, "CFBundleIdentifier").as_deref() == Some(BUNDLE_ID)).then(|| plist_string(&plist, "CFBundleShortVersionString")).flatten()
}

/// The app inside the Homebrew keg: next to this executable's keg, or the
/// usual `opt` links.
pub fn brew_bundle() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) {
        // <keg>/bin/smt5 or <keg>/SiliconMetaTrader5.app/Contents/MacOS/siliconmt5
        for a in exe.ancestors() {
            if a.file_name().map(|n| n == APP_NAME).unwrap_or(false) && a.to_string_lossy().contains("/Cellar/") {
                candidates.push(a.to_path_buf());
            }
            if a.join(APP_NAME).join("Contents/Info.plist").exists() && a.to_string_lossy().contains("/Cellar/") {
                candidates.push(a.join(APP_NAME));
            }
        }
    }
    for prefix in ["/opt/homebrew", "/usr/local"] {
        candidates.push(Path::new(prefix).join("opt/siliconmt5").join(APP_NAME));
        candidates.push(Path::new(prefix).join("opt/siliconmt5").join(LEGACY_APP_NAME));
    }
    candidates.into_iter().find(|p| bundle_version(p).is_some()).map(|p| p.canonicalize().unwrap_or(p))
}

/// Where the copy goes: /Applications when writable, else ~/Applications.
pub fn install_dir() -> PathBuf {
    let sys = Path::new("/Applications");
    let writable = std::fs::metadata(sys).map(|m| !m.permissions().readonly()).unwrap_or(false)
        && tempfile_probe(sys);
    if writable {
        sys.to_path_buf()
    } else {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Applications")
    }
}

fn tempfile_probe(dir: &Path) -> bool {
    let probe = dir.join(format!(".smt5-write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn app_dirs() -> [PathBuf; 2] {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    [PathBuf::from("/Applications"), home.join("Applications")]
}

/// An existing copy (or old symlink) in either Applications folder.
pub fn installed_copy() -> Option<PathBuf> {
    app_dirs().into_iter().map(|d| d.join(APP_NAME)).find(|p| p.symlink_metadata().is_ok())
}

/// A copy under the old name (SiliconMT5.app) that is ours: link or our bundle.
pub fn legacy_copy() -> Option<PathBuf> {
    app_dirs().into_iter().map(|d| d.join(LEGACY_APP_NAME)).find(|p| {
        p.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) || bundle_version(p).is_some()
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CopyStatus {
    /// Homebrew bundle and its version (None: not a Homebrew install).
    pub brew: Option<String>,
    pub brew_version: Option<String>,
    /// The copy in Applications, its version, and whether it is only a link.
    pub copy: Option<String>,
    pub copy_version: Option<String>,
    pub copy_is_link: bool,
    /// Copy missing, a link, or older than the Homebrew bundle.
    pub needs_install: bool,
}

pub fn status() -> CopyStatus {
    let Some(brew) = brew_bundle() else { return CopyStatus::default() };
    let brew_version = bundle_version(&brew);
    let copy = installed_copy();
    let copy_is_link = copy.as_ref().and_then(|p| p.symlink_metadata().ok()).map(|m| m.file_type().is_symlink()).unwrap_or(false);
    let copy_version = copy.as_ref().filter(|_| !copy_is_link).and_then(|p| bundle_version(p));
    CopyStatus {
        needs_install: copy.is_none() || copy_is_link || copy_version != brew_version || legacy_copy().is_some(),
        brew: Some(brew.display().to_string()),
        brew_version,
        copy: copy.map(|p| p.display().to_string()),
        copy_version,
        copy_is_link,
    }
}

/// Copy the Homebrew bundle into Applications (replacing an older copy or a
/// link; never anything that is not SiliconMetaTrader5). Returns the destination.
pub fn install(runner: &dyn Runner) -> Result<PathBuf> {
    let src = brew_bundle().context("SiliconMetaTrader5.app was not found in Homebrew (brew tap bahadirumutiscimen/siliconmt5 https://github.com/bahadirumutiscimen/silicon-metatrader5 && brew install siliconmt5)")?;
    let legacy = legacy_copy();
    let dest = match (installed_copy(), &legacy) {
        (Some(existing), _) => existing,
        (None, Some(old)) => old.with_file_name(APP_NAME), // same folder as the renamed app
        (None, None) => install_dir().join(APP_NAME),
    };
    if let Ok(meta) = dest.symlink_metadata() {
        if meta.file_type().is_symlink() {
            std::fs::remove_file(&dest).with_context(|| format!("removing the link {}", dest.display()))?;
        } else if bundle_version(&dest).is_some() {
            std::fs::remove_dir_all(&dest).with_context(|| format!("removing the old copy {}", dest.display()))?;
        } else {
            bail!("{} exists and is not SiliconMetaTrader5; move it away first", dest.display());
        }
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // ditto keeps the bundle's signature, extended attributes and symlinks.
    let o = runner.run(&Cmd::new("/usr/bin/ditto").arg(src.display().to_string()).arg(dest.display().to_string()));
    if !o.ok() {
        bail!("ditto failed: {}", o.stderr.trim());
    }
    // the renamed app replaces SiliconMT5.app
    if let Some(old) = legacy {
        let _ = if old.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) { std::fs::remove_file(&old) } else { std::fs::remove_dir_all(&old) };
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLIST: &str = r#"<plist><dict>
      <key>CFBundleIdentifier</key><string>com.siliconmt5.desktop</string>
      <key>CFBundleShortVersionString</key><string>1.3.0</string>
    </dict></plist>"#;

    #[test]
    fn plist_values_and_versions() {
        assert_eq!(plist_string(PLIST, "CFBundleShortVersionString").as_deref(), Some("1.3.0"));
        assert_eq!(plist_string(PLIST, "Missing"), None);
        let dir = std::env::temp_dir().join(format!("smt5-app-{}", std::process::id()));
        let app = dir.join(APP_NAME);
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), PLIST).unwrap();
        assert_eq!(bundle_version(&app).as_deref(), Some("1.3.0"));
        std::fs::write(app.join("Contents/Info.plist"), PLIST.replace(BUNDLE_ID, "com.other")).unwrap();
        assert_eq!(bundle_version(&app), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
