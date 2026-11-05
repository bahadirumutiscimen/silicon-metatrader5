//! The `runtime/` folder (compose.yaml, scripts, status API) the CLI and app
//! operate on, and where the user's settings (`.env`) live.
//!
//! * Repository checkout: settings in `runtime/.env`, as always.
//! * Packaged install (Homebrew, .deb, AppImage — the runtime folder carries a
//!   `.packaged` marker): the runtime files are read-only and replaced on every
//!   upgrade, so settings live in the user's config folder instead
//!   (`~/Library/Application Support/SiliconMT5/.env` on macOS,
//!   `$XDG_CONFIG_HOME/siliconmt5/.env` on Linux).
//!
//! `SMT5_ENV_FILE` overrides the settings file in both cases.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::env::EnvFile;

/// File that marks a runtime folder installed by a package.
pub const PACKAGED_MARKER: &str = ".packaged";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDir {
    pub root: PathBuf,
    env: PathBuf,
}

fn is_runtime_dir(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("compose.yaml"))
        .map(|s| s.lines().any(|l| l.trim() == "name: siliconmt5"))
        .unwrap_or(false)
}

/// The user's SiliconMetaTrader5 config folder (settings of packaged installs).
pub fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        return home.join("Library/Application Support/SiliconMT5");
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("siliconmt5")
}

/// Runtime folders a package puts next to the executable:
/// Homebrew `bin/` → `share/siliconmt5/runtime`, a macOS app bundle
/// `Contents/MacOS/` → `Contents/Resources/runtime`, .deb / AppImage
/// `usr/bin/` → `usr/lib/SiliconMetaTrader5/runtime` (`SiliconMT5` before the rename).
pub fn packaged_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    let up = exe_dir.parent().unwrap_or(exe_dir);
    vec![
        up.join("share/siliconmt5/runtime"),
        up.join("Resources/runtime"),
        up.join("lib/SiliconMetaTrader5/runtime"),
        up.join("lib/SiliconMT5/runtime"),
        up.join("lib/siliconmt5/runtime"),
    ]
}

impl RuntimeDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let env = if let Some(p) = std::env::var_os("SMT5_ENV_FILE").filter(|p| !p.is_empty()) {
            PathBuf::from(p)
        } else if root.join(PACKAGED_MARKER).exists() {
            config_dir().join(".env")
        } else {
            root.join(".env")
        };
        Self { root, env }
    }

    /// Installed by a package (settings in the config folder)?
    pub fn packaged(&self) -> bool {
        self.root.join(PACKAGED_MARKER).exists()
    }

    /// Find the runtime folder:
    /// 1. `explicit` (CLI `--runtime-dir`), 2. `$SMT5_RUNTIME_DIR`,
    /// 3. the current directory or a parent of it (also `<parent>/runtime`),
    /// 4. `extra` (e.g. the desktop app's resource folder),
    /// 5. next to the executable: a package layout, or `…/runtime` above
    ///    `app/target/release` in a checkout.
    pub fn discover(explicit: Option<&Path>) -> Result<Self> {
        Self::discover_with(explicit, &[])
    }

    pub fn discover_with(explicit: Option<&Path>, extra: &[PathBuf]) -> Result<Self> {
        if let Some(p) = explicit {
            if is_runtime_dir(p) {
                return Ok(Self::new(p));
            }
            bail!("{} is not a SiliconMetaTrader5 runtime folder (no compose.yaml with `name: siliconmt5`)", p.display());
        }
        if let Some(p) = std::env::var_os("SMT5_RUNTIME_DIR") {
            let p = PathBuf::from(p);
            if is_runtime_dir(&p) {
                return Ok(Self::new(p));
            }
            bail!("SMT5_RUNTIME_DIR={} is not a SiliconMetaTrader5 runtime folder", p.display());
        }
        let ancestors_of = |start: &Path| -> Option<Self> {
            for dir in start.ancestors() {
                if is_runtime_dir(dir) {
                    return Some(Self::new(dir));
                }
                let sub = dir.join("runtime");
                if is_runtime_dir(&sub) {
                    return Some(Self::new(sub));
                }
            }
            None
        };
        if let Some(rt) = std::env::current_dir().ok().and_then(|cwd| ancestors_of(&cwd)) {
            return Ok(rt);
        }
        if let Some(p) = extra.iter().find(|p| is_runtime_dir(p)) {
            return Ok(Self::new(p));
        }
        if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.parent().map(Path::to_path_buf)) {
            if let Some(p) = packaged_candidates(&dir).into_iter().find(|p| is_runtime_dir(p)) {
                return Ok(Self::new(p));
            }
            if let Some(rt) = ancestors_of(&dir) {
                return Ok(rt);
            }
        }
        bail!("could not find the SiliconMetaTrader5 runtime folder; run inside the repository or pass --runtime-dir")
    }

    pub fn compose_file(&self) -> PathBuf {
        self.root.join("compose.yaml")
    }

    /// The user's settings file (see the module docs).
    pub fn env_path(&self) -> PathBuf {
        self.env.clone()
    }

    pub fn env_example_path(&self) -> PathBuf {
        self.root.join(".env.example")
    }

    /// `.env`, or `.env.example` when `.env` does not exist yet.
    pub fn load_env(&self) -> EnvFile {
        if self.env_path().exists() {
            EnvFile::load_or_default(&self.env_path())
        } else {
            EnvFile::load_or_default(&self.env_example_path())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_explicit_and_nested() {
        let base = std::env::temp_dir().join(format!("smt5-rt-{}", std::process::id()));
        let rt = base.join("repo").join("runtime");
        std::fs::create_dir_all(&rt).unwrap();
        std::fs::write(rt.join("compose.yaml"), "# x\nname: siliconmt5\nservices: {}\n").unwrap();
        assert_eq!(RuntimeDir::discover(Some(&rt)).unwrap().root, rt);
        assert!(RuntimeDir::discover(Some(&base)).is_err());
        assert!(is_runtime_dir(&rt));
        assert!(!is_runtime_dir(&base.join("repo")));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn settings_live_in_the_config_dir_for_packages() {
        let base = std::env::temp_dir().join(format!("smt5-pkg-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let checkout = RuntimeDir::new(&base);
        assert!(!checkout.packaged());
        assert_eq!(checkout.env_path(), base.join(".env"));
        std::fs::write(base.join(PACKAGED_MARKER), "").unwrap();
        let pkg = RuntimeDir::new(&base);
        assert!(pkg.packaged());
        assert_eq!(pkg.env_path(), config_dir().join(".env"));
        assert_eq!(pkg.env_example_path(), base.join(".env.example"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn package_layouts() {
        let c = packaged_candidates(Path::new("/opt/homebrew/Cellar/siliconmt5/1.3.0/bin"));
        assert!(c.contains(&PathBuf::from("/opt/homebrew/Cellar/siliconmt5/1.3.0/share/siliconmt5/runtime")));
        let c = packaged_candidates(Path::new("/x/SiliconMetaTrader5.app/Contents/MacOS"));
        assert!(c.contains(&PathBuf::from("/x/SiliconMetaTrader5.app/Contents/Resources/runtime")));
        let c = packaged_candidates(Path::new("/usr/bin"));
        assert!(c.contains(&PathBuf::from("/usr/lib/SiliconMetaTrader5/runtime")));
    }
}
