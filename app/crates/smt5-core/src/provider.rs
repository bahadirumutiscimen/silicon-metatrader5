//! Where the runtime runs.
//!
//! MT5 needs an x86_64 CPU. The safe path on ARM hosts is a full x86_64 QEMU
//! VM (Colima profile `x86`) running the linux/amd64 container. On macOS the
//! `mac-native` provider runs MT5 directly in macOS Wine (x86_64 Wine under
//! the Mac's own Rosetta 2, `runtime/macos/native.sh`): no VM, no Docker.

use serde::{Deserialize, Serialize};

use crate::env::EnvFile;
use crate::exec::{Cmd, Runner};

pub const DEFAULT_PROFILE: &str = "x86";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// macOS (Apple Silicon or Intel) with a Colima x86_64 profile.
    MacColima,
    /// Linux x86_64 with Docker Engine — no emulation.
    LinuxDocker,
    /// A remote Linux host through an SSH docker context.
    RemoteSsh,
    /// macOS: MT5 in macOS Wine 11 under Rosetta 2 (launchd agent, no Docker).
    MacNative,
    /// MT5 directly in Linux Wine 11 (x86_64), no Docker.
    LinuxNative,
}

impl ProviderKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mac-colima" => Some(Self::MacColima),
            "linux-docker" => Some(Self::LinuxDocker),
            "remote-ssh" => Some(Self::RemoteSsh),
            "mac-native" => Some(Self::MacNative),
            "linux-native" => Some(Self::LinuxNative),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::MacColima => "macOS · Colima x86_64 VM (QEMU)",
            Self::LinuxDocker => "Linux x86_64 · Docker Engine",
            Self::RemoteSsh => "Remote Linux · SSH docker context",
            Self::MacNative => "macOS · Wine 11 + Rosetta 2 (no VM)",
            Self::LinuxNative => "Linux · Wine 11 (no Docker)",
        }
    }

    pub fn uses_colima(&self) -> bool {
        matches!(self, Self::MacColima)
    }

    /// The app can start/stop the engine itself (Colima VM).
    pub fn manages_engine(&self) -> bool {
        self.uses_colima()
    }

    /// No container: MT5 runs in macOS Wine (see [`crate::native`]).
    pub fn is_native(&self) -> bool {
        matches!(self, Self::MacNative | Self::LinuxNative)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub os: String,
    pub arch: String,
}

impl HostInfo {
    pub fn current() -> Self {
        Self { os: std::env::consts::OS.to_owned(), arch: std::env::consts::ARCH.to_owned() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    pub kind: ProviderKind,
    /// Docker context pinned for every SiliconMetaTrader5 command (never `docker context use`).
    pub docker_context: Option<String>,
    /// Colima profile for VM-based providers.
    pub colima_profile: Option<String>,
}

/// VM size used when creating the Colima profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmSize {
    pub cpus: u32,
    pub memory_gib: u32,
    pub disk_gib: u32,
}

impl Default for VmSize {
    fn default() -> Self {
        Self { cpus: 4, memory_gib: 8, disk_gib: 60 }
    }
}

/// One `colima list --json` row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColimaProfile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub cpus: u64,
    #[serde(default)]
    pub memory: u64,
    #[serde(default)]
    pub disk: u64,
}

impl ColimaProfile {
    pub fn running(&self) -> bool {
        self.status.eq_ignore_ascii_case("running")
    }
}

/// Parse `colima list --json`: one JSON object per line (older versions) or a JSON array.
pub fn parse_colima_list(stdout: &str) -> Vec<ColimaProfile> {
    let t = stdout.trim();
    if t.starts_with('[') {
        return serde_json::from_str(t).unwrap_or_default();
    }
    t.lines().filter_map(|l| serde_json::from_str(l.trim()).ok()).collect()
}

impl Provider {
    /// Pick the provider from `.env` (`SMT5_PROVIDER`, `SMT5_DOCKER_CONTEXT`,
    /// `SMT5_COLIMA_PROFILE`) or from the host OS/arch.
    pub fn detect(env: &EnvFile, host: &HostInfo) -> Self {
        let mut kind = env.get_nonempty("SMT5_PROVIDER").and_then(ProviderKind::parse).unwrap_or(
            match (host.os.as_str(), host.arch.as_str()) {
                ("macos", _) => ProviderKind::MacColima,
                _ => ProviderKind::LinuxDocker,
            },
        );
        // the native mode is the one of this OS, whatever .env was written on
        if kind.is_native() {
            kind = if host.os == "macos" { ProviderKind::MacNative } else { ProviderKind::LinuxNative };
        }
        let profile = env.get_nonempty("SMT5_COLIMA_PROFILE").unwrap_or(DEFAULT_PROFILE).to_owned();
        let explicit_ctx = env.get_nonempty("SMT5_DOCKER_CONTEXT").map(str::to_owned);
        let (docker_context, colima_profile) = if kind.is_native() {
            (None, None)
        } else if kind.uses_colima() {
            // Colima names the context `colima-<profile>` (just `colima` for "default").
            let ctx = if profile == "default" { "colima".to_owned() } else { format!("colima-{profile}") };
            (Some(explicit_ctx.unwrap_or(ctx)), Some(profile))
        } else {
            (explicit_ctx, None)
        };
        Self { kind, docker_context, colima_profile }
    }

    /// Environment pinning docker to this provider for one command.
    pub fn docker_env(&self) -> Vec<(String, String)> {
        self.docker_context.iter().map(|c| ("DOCKER_CONTEXT".to_owned(), c.clone())).collect()
    }

    fn profile(&self) -> &str {
        self.colima_profile.as_deref().unwrap_or(DEFAULT_PROFILE)
    }

    /// The Güvenli (safe) mode provider of this host, whatever `.env` selects:
    /// the native mode imports MT5 from it and the uninstall job removes it too.
    pub fn safe(env: &EnvFile, host: &HostInfo) -> Self {
        let mut e = env.clone();
        if e.get_nonempty("SMT5_PROVIDER").and_then(ProviderKind::parse).map(|k| k.is_native()).unwrap_or(false) {
            e.set("SMT5_PROVIDER", "");
        }
        Self::detect(&e, host)
    }

    /// Start the engine: `colima start` for a full x86_64 QEMU VM (also
    /// enlarges the disk of an existing profile).
    pub fn vm_start_cmd(&self, size: VmSize) -> Option<Cmd> {
        self.kind.uses_colima().then(|| {
            Cmd::new("colima").args([
                "start".to_owned(),
                "--profile".to_owned(),
                self.profile().to_owned(),
                "--arch".to_owned(),
                "x86_64".to_owned(),
                "--vm-type=qemu".to_owned(),
                "--cpu".to_owned(),
                size.cpus.to_string(),
                "--memory".to_owned(),
                size.memory_gib.to_string(),
                "--disk".to_owned(),
                size.disk_gib.to_string(),
            ])
        })
    }

    pub fn vm_stop_cmd(&self) -> Option<Cmd> {
        self.kind.uses_colima().then(|| Cmd::new("colima").args(["stop", "--profile", self.profile()]))
    }

    /// `colima delete` for our profile only (never `default` or other projects'
    /// profiles). `with_data` adds `--data` (Colima ≥ 0.8 keeps the container
    /// data disk unless asked), pass it only when `colima delete --help` lists it.
    pub fn vm_delete_cmd(&self, with_data: bool) -> Option<Cmd> {
        self.kind.uses_colima().then(|| {
            let mut c = Cmd::new("colima").args(["delete", "--profile", self.profile(), "--force"]);
            if with_data {
                c = c.arg("--data");
            }
            c
        })
    }

    pub fn profile_name(&self) -> &str {
        self.profile()
    }

    /// Colima profiles that belong to something else (e.g. `default` used by other projects).
    pub fn other_vm_profiles(&self, runner: &dyn Runner) -> Vec<ColimaProfile> {
        if !self.kind.uses_colima() {
            return vec![];
        }
        let out = runner.run(&Cmd::new("colima").args(["list", "--json"]));
        parse_colima_list(&out.stdout).into_iter().filter(|p| p.name != self.profile()).collect()
    }

    /// The Colima profile row for this provider, if Colima knows it.
    pub fn vm_profile(&self, runner: &dyn Runner) -> Option<ColimaProfile> {
        if !self.kind.uses_colima() {
            return None;
        }
        let out = runner.run(&Cmd::new("colima").args(["list", "--json"]));
        if !out.ok() {
            return None;
        }
        parse_colima_list(&out.stdout).into_iter().find(|p| p.name == self.profile())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::MockRunner;

    fn host(os: &str, arch: &str) -> HostInfo {
        HostInfo { os: os.into(), arch: arch.into() }
    }

    #[test]
    fn native_provider() {
        let mut e = EnvFile::default();
        e.set("SMT5_PROVIDER", "mac-native");
        e.set("SMT5_DOCKER_CONTEXT", "colima-x86");
        let n = Provider::detect(&e, &host("macos", "aarch64"));
        assert!(n.kind.is_native() && !n.kind.uses_colima() && !n.kind.manages_engine());
        assert_eq!((n.docker_context.as_deref(), n.colima_profile.as_deref()), (None, None));
        assert!(n.vm_start_cmd(VmSize::default()).is_none() && n.vm_delete_cmd(true).is_none());
        let s = Provider::safe(&e, &host("macos", "aarch64"));
        assert_eq!(s.kind, ProviderKind::MacColima);
        assert_eq!(s.docker_context.as_deref(), Some("colima-x86"));
    }

    #[test]
    fn delete_only_our_profile() {
        let p = Provider::detect(&EnvFile::default(), &host("macos", "aarch64"));
        assert_eq!(p.vm_delete_cmd(false).unwrap().display(), "colima delete --profile x86 --force");
        assert_eq!(p.vm_delete_cmd(true).unwrap().display(), "colima delete --profile x86 --force --data");
        assert!(Provider::detect(&EnvFile::default(), &host("linux", "x86_64")).vm_delete_cmd(true).is_none());
        let m = MockRunner::new().on("colima list", 0, r#"{"name":"default","status":"Running","arch":"aarch64"}
{"name":"x86","status":"Stopped","arch":"x86_64"}"#);
        let others = p.other_vm_profiles(&m);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].name, "default");
    }

    #[test]
    fn detect_by_host() {
        let e = EnvFile::default();
        let mac = Provider::detect(&e, &host("macos", "aarch64"));
        assert_eq!(mac.kind, ProviderKind::MacColima);
        assert_eq!(mac.docker_context.as_deref(), Some("colima-x86"));
        assert_eq!(mac.docker_env(), vec![("DOCKER_CONTEXT".into(), "colima-x86".into())]);
        assert_eq!(Provider::detect(&e, &host("linux", "x86_64")).kind, ProviderKind::LinuxDocker);
        assert_eq!(Provider::detect(&e, &host("linux", "x86_64")).docker_env(), vec![]);
        // Linux ARM64 is not supported: plain Docker, the doctor refuses its aarch64 engine
        assert_eq!(Provider::detect(&e, &host("linux", "aarch64")).kind, ProviderKind::LinuxDocker);
    }

    #[test]
    fn detect_from_env() {
        let e = EnvFile::parse("SMT5_PROVIDER=remote-ssh\nSMT5_DOCKER_CONTEXT=box\n");
        let p = Provider::detect(&e, &host("macos", "aarch64"));
        assert_eq!((p.kind, p.docker_context.as_deref(), p.colima_profile), (ProviderKind::RemoteSsh, Some("box"), None));
        let e = EnvFile::parse("SMT5_COLIMA_PROFILE=mt5\n");
        assert_eq!(Provider::detect(&e, &host("macos", "x86_64")).docker_context.as_deref(), Some("colima-mt5"));
    }

    #[test]
    fn vm_commands() {
        let p = Provider::detect(&EnvFile::default(), &host("macos", "aarch64"));
        assert_eq!(
            p.vm_start_cmd(VmSize::default()).unwrap().display(),
            "colima start --profile x86 --arch x86_64 --vm-type=qemu --cpu 4 --memory 8 --disk 60"
        );
        assert!(Provider::detect(&EnvFile::default(), &host("linux", "x86_64")).vm_start_cmd(VmSize::default()).is_none());
    }

    #[test]
    fn colima_list_formats() {
        let lines = r#"{"name":"default","status":"Running","arch":"aarch64","cpus":6,"memory":10737418240,"disk":107374182400}
{"name":"x86","status":"Running","arch":"x86_64","cpus":4,"memory":8589934592,"disk":64424509440}"#;
        let v = parse_colima_list(lines);
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].arch, "x86_64");
        assert_eq!(parse_colima_list(&format!("[{}]", lines.replace('\n', ","))).len(), 2);
        let m = MockRunner::new().on("colima list --json", 0, lines);
        let p = Provider::detect(&EnvFile::default(), &host("macos", "aarch64"));
        let prof = p.vm_profile(&m).unwrap();
        assert!(prof.running());
        assert_eq!(prof.disk, 64424509440);
    }
}
