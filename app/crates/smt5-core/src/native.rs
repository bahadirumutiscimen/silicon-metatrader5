//! macOS native mode ("Mac yerel"): MT5 + the bridge in macOS Wine 11 under
//! Rosetta 2, supervised by a launchd agent. `runtime/macos/native.sh` does
//! the work; this module knows its paths and asks the system for facts.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::docker::{ContainerState, ContainerStats};
use crate::exec::{Cmd, Runner};
use crate::runtime::RuntimeDir;

/// launchd label of the supervisor agent (see native.sh).
pub const LABEL: &str = "com.siliconmt5.runtime";
/// Minimum Wine major version (ucrtbase.crealf, needed by numpy).
pub const MIN_WINE: u32 = 11;

/// Folders under ~/Library/Application Support/SiliconMT5 (SMT5_NATIVE_HOME).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativePaths {
    pub base: PathBuf,
    pub wine: PathBuf,
    pub prefix: PathBuf,
    pub probe_prefix: PathBuf,
    pub venv: PathBuf,
    pub state: PathBuf,
    pub logs: PathBuf,
}

impl NativePaths {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        let base = base.into();
        Self {
            wine: base.join("wine"),
            prefix: base.join("prefix"),
            probe_prefix: base.join("native-probe"),
            venv: base.join("venv"),
            state: base.join("state"),
            logs: base.join("logs"),
            base,
        }
    }

    pub fn current() -> Self {
        if let Some(p) = std::env::var_os("SMT5_NATIVE_HOME") {
            return Self::new(PathBuf::from(p));
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        if cfg!(target_os = "macos") {
            return Self::new(home.join("Library/Application Support/SiliconMT5"));
        }
        let data = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
        Self::new(data.join("siliconmt5"))
    }

    /// How MT5 is shown (state/display, written by the supervisor): "window",
    /// or the noVNC port of a headless Linux runtime.
    pub fn display(&self) -> NativeDisplay {
        let s = std::fs::read_to_string(self.state.join("display")).unwrap_or_default();
        match s.trim().strip_prefix("novnc:").and_then(|p| p.parse().ok()) {
            Some(port) => NativeDisplay::NoVnc(port),
            None => NativeDisplay::Window,
        }
    }

    pub fn mt5_dir(&self) -> PathBuf {
        self.prefix.join("drive_c/Program Files/MetaTrader 5")
    }

    pub fn windows_python(&self) -> PathBuf {
        self.prefix.join("drive_c/Program Files/Python313/python.exe")
    }

    /// The Wine loader inside the downloaded Wine.app (or SMT5_WINE).
    pub fn wine_binary(&self) -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("SMT5_WINE").map(PathBuf::from).filter(|p| p.is_file()) {
            return Some(p);
        }
        find_wine(&self.wine, 0)
    }
}

fn find_wine(dir: &Path, depth: u32) -> Option<PathBuf> {
    if depth > 6 {
        return None;
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    if dir.file_name().map(|n| n == "bin").unwrap_or(false) {
        for name in ["wine", "wine64"] {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    entries.iter().filter(|p| p.is_dir() && !p.is_symlink()).find_map(|p| find_wine(p, depth + 1))
}

/// `native.sh <sub>` of this runtime folder.
pub fn script(rt: &RuntimeDir) -> PathBuf {
    rt.root.join("macos").join("native.sh")
}

/// `native.sh <sub>`; the script reads the user's settings from SMT5_ENV_FILE.
pub fn cmd(rt: &RuntimeDir, sub: &str) -> Cmd {
    Cmd::new("/bin/bash").arg(script(rt).display().to_string()).arg(sub).env("SMT5_ENV_FILE", &rt.env_path().display().to_string())
}

/// "wine-11.21 (Staging)" → 11
pub fn wine_major(version: &str) -> Option<u32> {
    version.trim().strip_prefix("wine-")?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NativeFacts {
    /// Wine prefix with MT5 and Windows Python, venv and Wine present.
    pub installed: bool,
    pub wine: Option<String>,
    pub mt5: bool,
    pub windows_python: bool,
    pub venv: bool,
    /// The probe's prefix exists and will be reused by `install`.
    pub probe_prefix: bool,
    /// The launchd agent is loaded (supervisor running or stopping).
    pub running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeDisplay {
    Window,
    NoVnc(u16),
}

/// systemd --user unit of the Linux native runtime (native.sh UNIT).
pub const UNIT: &str = "siliconmt5-runtime";

pub fn agent_loaded(runner: &dyn Runner) -> bool {
    if cfg!(target_os = "macos") {
        return runner.run(&Cmd::new("launchctl").args(["list", LABEL])).ok();
    }
    if runner.run(&Cmd::new("systemctl").args(["--user", "is-active", "--quiet", UNIT])).ok() {
        return true;
    }
    // no user systemd: a detached supervisor recorded in state/agent.pid
    std::fs::read_to_string(NativePaths::current().state.join("agent.pid"))
        .ok()
        .and_then(|p| p.trim().parse::<u32>().ok())
        .map(|pid| Path::new(&format!("/proc/{pid}")).exists())
        .unwrap_or(false)
}

pub fn facts(runner: &dyn Runner, paths: &NativePaths) -> NativeFacts {
    let wine = paths.wine_binary();
    let mt5 = paths.mt5_dir().join("terminal64.exe").is_file();
    let windows_python = paths.windows_python().is_file();
    let venv = paths.venv.join("bin/python").exists();
    NativeFacts {
        installed: wine.is_some() && mt5 && windows_python && venv,
        wine: wine.map(|p| p.display().to_string()),
        mt5,
        windows_python,
        venv,
        probe_prefix: paths.probe_prefix.join("system.reg").is_file(),
        running: agent_loaded(runner),
    }
}

/// What the UI shows where the container state would be.
pub fn pseudo_container(f: &NativeFacts) -> Option<ContainerState> {
    if f.running {
        Some(ContainerState { status: "running".into(), health: None })
    } else if f.installed {
        Some(ContainerState { status: "exited".into(), health: None })
    } else {
        None
    }
}

/// CPU and memory of the Wine processes of our Wine build (`ps`), as
/// container-like stats. CPU% is per core (like `docker stats`).
pub fn stats(runner: &dyn Runner, paths: &NativePaths) -> Option<ContainerStats> {
    let o = runner.run(&Cmd::new("ps").args(["-axww", "-o", "rss=,%cpu=,command="]));
    if !o.ok() {
        return None;
    }
    let total = runner.run(&Cmd::new("sysctl").args(["-n", "hw.memsize"])).stdout_trimmed().parse::<u64>().ok()
        // Linux has no hw.memsize
        .or_else(|| std::fs::read_to_string("/proc/meminfo").ok().and_then(|m| mem_total(&m)))
        .unwrap_or(0);
    let (cpu, rss) = sum_processes(&o.stdout, &paths.base.display().to_string());
    Some(ContainerStats {
        cpu_percent: cpu,
        mem_used_bytes: rss,
        mem_limit_bytes: total,
        mem_percent: if total > 0 { rss as f64 * 100.0 / total as f64 } else { 0.0 },
    })
}

/// `MemTotal:  16314764 kB` of /proc/meminfo, in bytes.
pub fn mem_total(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
}

/// Sum `rss %cpu command` rows that belong to SiliconMetaTrader5's native runtime:
/// anything started from our folder (wineserver, the status API) and the
/// Windows processes, which Wine renames to their Windows command line
/// (`terminal64.exe /portable …`, `C:\Program Files\Python313\python.exe …`,
/// `C:\windows\system32\services.exe` …). MetaQuotes' own Mac app is skipped.
pub fn sum_processes(ps: &str, base: &str) -> (f64, u64) {
    let mut cpu = 0.0;
    let mut rss = 0u64;
    for line in ps.lines() {
        let mut it = line.split_whitespace();
        let (Some(r), Some(c)) = (it.next(), it.next()) else { continue };
        let command: String = it.collect::<Vec<_>>().join(" ");
        let lower = command.to_lowercase();
        let ours = command.contains(base)
            || lower.starts_with("terminal64.exe /portable /config:mt5cfg.ini")
            || lower.contains("python313\\python.exe")
            || lower.starts_with("c:\\windows\\");
        if ours && !lower.contains("net.metaquotes") {
            cpu += c.replace(',', ".").parse::<f64>().unwrap_or(0.0);
            rss += r.parse::<u64>().unwrap_or(0) * 1024;
        }
    }
    (cpu, rss)
}

#[cfg(test)]
mod tests {
    #[test]
    fn mem_total_from_meminfo() {
        assert_eq!(super::mem_total("MemTotal:       16314764 kB\nMemFree: 1 kB\n"), Some(16314764 * 1024));
        assert_eq!(super::mem_total("nothing"), None);
    }

    use super::*;
    use crate::exec::MockRunner;

    #[test]
    fn paths_and_wine_lookup() {
        let tmp = std::env::temp_dir().join(format!("smt5-native-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let p = NativePaths::new(&tmp);
        let bin = p.wine.join("Wine Devel.app/Contents/Resources/wine/bin");
        std::fs::create_dir_all(&bin).unwrap();
        assert_eq!(p.wine_binary(), None);
        std::fs::write(bin.join("wine"), "").unwrap();
        std::fs::write(bin.join("wineserver"), "").unwrap();
        assert_eq!(p.wine_binary(), Some(bin.join("wine")));
        let m = MockRunner::new().on("launchctl list", 1, "");
        let f = facts(&m, &p);
        assert!(!f.installed && !f.running && f.wine.is_some());
        assert_eq!(pseudo_container(&f), None);
        std::fs::create_dir_all(p.mt5_dir()).unwrap();
        std::fs::write(p.mt5_dir().join("terminal64.exe"), "").unwrap();
        std::fs::create_dir_all(p.windows_python().parent().unwrap()).unwrap();
        std::fs::write(p.windows_python(), "").unwrap();
        std::fs::create_dir_all(p.venv.join("bin")).unwrap();
        std::fs::write(p.venv.join("bin/python"), "").unwrap();
        let f = facts(&MockRunner::new().on("launchctl list", 0, "{}").on("systemctl --user is-active", 0, ""), &p);
        assert!(f.installed && f.running);
        assert_eq!(pseudo_container(&f).unwrap().status, "running");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn versions_and_processes() {
        assert_eq!(wine_major("wine-11.21"), Some(11));
        assert_eq!(wine_major("wine-10.0 (Staging)"), Some(10));
        assert_eq!(wine_major("garbage"), None);
        let ps = "  1024  12,5 /Users/u/Library/Application Support/SiliconMT5/wine/Wine Devel.app/Contents/Resources/wine/bin/wineserver\n\
                  204800  30.0 terminal64.exe /portable /config:mt5cfg.ini\n\
                  8192   2.0 C:\\windows\\system32\\services.exe\n\
                  4096   1.0 C:\\Program Files\\Python313\\python.exe -m siliconmetatrader5.server --host 127.0.0.1 -p 8001\n\
                  999999 50.0 /Applications/MetaTrader 5.app/Contents/SharedSupport/wine/bin/wine64-preloader C:\\Program Files\\MetaTrader 5\\terminal64.exe net.metaquotes\n\
                  2048   0.5 /usr/bin/zsh";
        let (cpu, rss) = sum_processes(ps, "/Users/u/Library/Application Support/SiliconMT5");
        assert!((cpu - 45.5).abs() < 1e-9, "{cpu}");
        assert_eq!(rss, (1024 + 204800 + 8192 + 4096) * 1024);
        assert_eq!(cmd(&RuntimeDir::new("/r/runtime"), "start").display(), "/bin/bash /r/runtime/macos/native.sh start");
    }
}
