//! Runtime modes (macOS):
//!
//! * `safe` — Güvenli: Colima x86_64 QEMU VM + WineHQ container (slow, isolated)
//! * `native` — Mac yerel: MT5 in macOS Wine 11 under Rosetta 2 (fast, no VM)
//!
//! Each mode keeps its own data (Docker volumes vs. the native Wine prefix in
//! ~/Library/Application Support/SiliconMT5): switching never touches the other's.

use serde::Serialize;

use crate::env::EnvFile;
use crate::provider::ProviderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Safe,
    Native,
}

/// `.env` keys of the removed OrbStack / Colima vz experiment, cleared on switch.
const LEGACY_KEYS: &[&str] = &["SMT5_VARIANT"];

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Safe, Mode::Native];

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "safe" | "qemu" => Some(Self::Safe),
            "native" | "mac" => Some(Self::Native),
            _ => None,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Native => "native",
        }
    }

    pub fn label(&self) -> &'static str {
        self.label_for(std::env::consts::OS)
    }

    /// The mode's name on this OS (Güvenli / Mac yerel on macOS; Docker / Linux yerel on Linux).
    pub fn label_for(&self, host_os: &str) -> &'static str {
        match (self, host_os == "macos") {
            (Self::Safe, true) => "Güvenli · Colima x86_64 QEMU VM",
            (Self::Native, true) => "Mac yerel · Wine 11 + Rosetta 2",
            (Self::Safe, false) => "Docker · Wine 11 container",
            (Self::Native, false) => "Linux yerel · Wine 11",
        }
    }

    /// The mode `.env` currently selects (anything else counts as safe).
    pub fn current(env: &EnvFile) -> Self {
        match env.get_nonempty("SMT5_PROVIDER").and_then(ProviderKind::parse) {
            Some(ProviderKind::MacNative | ProviderKind::LinuxNative) => Self::Native,
            _ => Self::Safe,
        }
    }

    /// `.env` keys for this mode.
    pub fn env_values(&self, host_os: &str) -> Vec<(&'static str, String)> {
        match self {
            Self::Safe => vec![
                ("SMT5_PROVIDER", String::new()),
                ("SMT5_DOCKER_CONTEXT", if host_os == "macos" { "colima-x86".into() } else { String::new() }),
                ("SMT5_COLIMA_PROFILE", String::new()),
                ("SMT5_ALLOW_USER_EMULATION", "0".into()),
            ],
            // The docker context stays: the native mode imports MT5 from the safe container.
            Self::Native => vec![
                ("SMT5_PROVIDER", if host_os == "macos" { "mac-native" } else { "linux-native" }.into()),
                ("SMT5_ALLOW_USER_EMULATION", "0".into()),
            ],
        }
    }

    /// Write the mode into `env`; returns the keys that changed.
    pub fn apply(&self, env: &mut EnvFile, host_os: &str) -> Vec<String> {
        let mut changed = Vec::new();
        let legacy = LEGACY_KEYS.iter().filter(|k| env.get_nonempty(k).is_some()).map(|k| (*k, String::new()));
        let values: Vec<(&str, String)> = self.env_values(host_os).into_iter().chain(legacy.collect::<Vec<_>>()).collect();
        for (k, v) in values {
            if env.get(k).unwrap_or("") != v {
                env.set(k, &v);
                changed.push(k.to_owned());
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{HostInfo, Provider};

    #[test]
    fn switch_and_back() {
        let mac = HostInfo { os: "macos".into(), arch: "aarch64".into() };
        let mut env = EnvFile::parse("SMT5_DOCKER_CONTEXT=colima-x86\nSMT5_DISPLAY=novnc\nSMT5_VARIANT=-wow64\n");
        assert_eq!(Mode::current(&env), Mode::Safe);
        let changed = Mode::Native.apply(&mut env, "macos");
        assert!(changed.contains(&"SMT5_PROVIDER".to_owned()) && changed.contains(&"SMT5_VARIANT".to_owned()));
        assert_eq!(Mode::current(&env), Mode::Native);
        assert!(Provider::detect(&env, &mac).kind.is_native());
        assert_eq!(env.get("SMT5_DOCKER_CONTEXT"), Some("colima-x86"));
        Mode::Safe.apply(&mut env, "macos");
        assert_eq!(Mode::current(&env), Mode::Safe);
        assert_eq!(Provider::detect(&env, &mac).docker_context.as_deref(), Some("colima-x86"));
        assert!(Mode::Safe.apply(&mut env, "macos").is_empty());
        assert_eq!(Mode::parse("mac"), Some(Mode::Native));
        // Linux: the native mode is the Linux one, also when .env came from a Mac
        let linux = HostInfo { os: "linux".into(), arch: "x86_64".into() };
        let mut lenv = EnvFile::default();
        Mode::Native.apply(&mut lenv, "linux");
        assert_eq!(lenv.get("SMT5_PROVIDER"), Some("linux-native"));
        assert_eq!(Provider::detect(&lenv, &linux).kind, ProviderKind::LinuxNative);
        lenv.set("SMT5_PROVIDER", "mac-native");
        assert_eq!(Provider::detect(&lenv, &linux).kind, ProviderKind::LinuxNative);
        assert_eq!(Mode::current(&lenv), Mode::Native);
        assert_eq!(Mode::Safe.label_for("linux"), "Docker · Wine 11 container");
        assert_eq!(Mode::parse("orbstack"), None);
    }
}
