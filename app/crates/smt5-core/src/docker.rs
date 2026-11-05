//! docker / docker compose, always pinned to the provider's context through
//! `DOCKER_CONTEXT` for that one command (the global context stays untouched).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::exec::{Cmd, Runner};
use crate::provider::Provider;
use crate::runtime::RuntimeDir;
use crate::CONTAINER;

pub struct Docker<'a> {
    pub runner: &'a dyn Runner,
    pub provider: &'a Provider,
    pub runtime: &'a RuntimeDir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerState {
    pub status: String,
    pub health: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub mem_used_bytes: u64,
    pub mem_limit_bytes: u64,
    pub mem_percent: f64,
}

/// "42.10%|5.1GiB / 7.7GiB|66.2%" → stats.
pub fn parse_stats(line: &str) -> Option<ContainerStats> {
    let mut parts = line.trim().split('|');
    let pct = |s: &str| s.trim().trim_end_matches('%').trim().parse::<f64>().ok();
    let cpu = pct(parts.next()?)?;
    let (used, limit) = parts.next()?.split_once('/')?;
    let mem = pct(parts.next()?)?;
    Some(ContainerStats { cpu_percent: cpu, mem_used_bytes: parse_size(used)?, mem_limit_bytes: parse_size(limit)?, mem_percent: mem })
}

/// docker's human sizes: "512KiB", "5.1GiB", "120MB", "0B".
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = s.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    let mult: f64 = match unit {
        "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "KiB" => 1024.0,
        "MiB" => 1024f64.powi(2),
        "GiB" => 1024f64.powi(3),
        "TiB" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * mult) as u64)
}

impl<'a> Docker<'a> {
    pub fn new(runner: &'a dyn Runner, provider: &'a Provider, runtime: &'a RuntimeDir) -> Self {
        Self { runner, provider, runtime }
    }

    /// `docker <args>` with the pinned context.
    pub fn cmd<I, S>(&self, args: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Cmd::new("docker").args(args).envs(&self.provider.docker_env())
    }

    /// `docker compose <args>` run from the runtime folder. Settings outside
    /// it (packaged installs) are passed with `--env-file`.
    pub fn compose<I, S>(&self, args: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut c = Cmd::new("docker").arg("compose");
        let env = self.runtime.env_path();
        if env != self.runtime.root.join(".env") && env.exists() {
            c = c.arg("--env-file").arg(env.display().to_string());
        }
        // The image of this release unless .env pins another (e.g. `edge`).
        if self.runtime.load_env().get_nonempty("SMT5_VERSION").is_none() {
            c = c.env("SMT5_VERSION", env!("CARGO_PKG_VERSION"));
        }
        c.args(args).envs(&self.provider.docker_env()).cwd(&self.runtime.root)
    }

    /// Docker CLI version ("29.0.1"). `docker --version` works without a
    /// reachable engine (unlike `docker version`, which then exits 1).
    pub fn client_version(&self) -> Option<String> {
        let o = self.runner.run(&Cmd::new("docker").arg("--version"));
        if !o.ok() {
            return None;
        }
        let s = o.stdout_trimmed();
        let v = s.split_once("version ").map(|(_, r)| r.split(',').next().unwrap_or(r).trim()).unwrap_or(s);
        (!v.is_empty()).then(|| v.to_owned())
    }

    /// Architecture of the docker engine behind the context (`x86_64`, `aarch64`), `None` if unreachable.
    pub fn engine_arch(&self) -> Option<String> {
        let o = self.runner.run(&self.cmd(["info", "--format", "{{.Architecture}}"]));
        o.ok().then(|| o.stdout_trimmed().to_owned()).filter(|s| !s.is_empty())
    }

    pub fn container_state(&self) -> Option<ContainerState> {
        let o = self.runner.run(&self.cmd([
            "inspect",
            "-f",
            "{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{end}}",
            CONTAINER,
        ]));
        if !o.ok() {
            return None;
        }
        let (status, health) = o.stdout_trimmed().split_once('|').unwrap_or((o.stdout_trimmed(), ""));
        Some(ContainerState {
            status: status.to_owned(),
            health: (!health.is_empty()).then(|| health.to_owned()),
        })
    }

    /// CPU / memory of the runtime container (`docker stats --no-stream`).
    pub fn stats(&self) -> Option<ContainerStats> {
        let o = self.runner.run(&self.cmd(["stats", "--no-stream", "--format", "{{.CPUPerc}}|{{.MemUsage}}|{{.MemPerc}}", CONTAINER]));
        if !o.ok() {
            return None;
        }
        parse_stats(o.stdout_trimmed())
    }

    /// Is the docker engine behind the pinned context reachable?
    pub fn engine_up(&self) -> bool {
        self.engine_arch().is_some()
    }

    /// Does the compose project still own volumes (Wine prefix, broker login,
    /// history)? True after `compose down` too — the runtime is installed.
    pub fn has_volumes(&self) -> bool {
        let o = self.runner.run(&self.cmd(["volume", "ls", "-q", "--filter", "label=com.docker.compose.project=siliconmt5"]));
        o.ok() && !o.stdout_trimmed().is_empty()
    }

    /// Volumes of the compose project (`siliconmt5_*`).
    pub fn project_volumes(&self) -> Vec<String> {
        let o = self.runner.run(&self.cmd(["volume", "ls", "-q", "--filter", "label=com.docker.compose.project=siliconmt5"]));
        o.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
    }

    /// Local runtime images as "repo:tag (size)".
    pub fn runtime_images(&self) -> Vec<String> {
        let o = self.runner.run(&self.cmd([
            "image",
            "ls",
            "--filter",
            "reference=ghcr.io/bahadirumutiscimen/siliconmt5",
            "--format",
            "{{.Repository}}:{{.Tag}} ({{.Size}})",
        ]));
        o.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
    }

    pub fn image_present(&self, image: &str) -> bool {
        self.runner.run(&self.cmd(["image", "inspect", "--format", "{{.Id}}", image])).ok()
    }

    /// Running containers in the same engine that are not SiliconMetaTrader5.
    pub fn other_containers(&self) -> Vec<String> {
        let o = self.runner.run(&self.cmd(["ps", "--format", "{{.Names}}"]));
        if !o.ok() {
            return vec![];
        }
        o.stdout.lines().map(str::trim).filter(|n| !n.is_empty() && *n != CONTAINER).map(str::to_owned).collect()
    }
}

/// The context `docker context use` selected globally (what other projects see).
pub fn global_context() -> String {
    let path = std::env::var_os("DOCKER_CONFIG")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".docker")))
        .map(|d| d.join("config.json"));
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("currentContext").and_then(|c| c.as_str()).map(str::to_owned))
        .unwrap_or_else(|| "default".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::EnvFile;
    use crate::exec::MockRunner;
    use crate::provider::HostInfo;

    fn mac() -> Provider {
        Provider::detect(&EnvFile::default(), &HostInfo { os: "macos".into(), arch: "aarch64".into() })
    }

    #[test]
    fn compose_is_pinned_and_runs_in_runtime_dir() {
        let p = mac();
        let rt = RuntimeDir::new("/repo/runtime");
        let m = MockRunner::new();
        let d = Docker::new(&m, &p, &rt);
        let c = d.compose(["up", "-d"]);
        assert_eq!(c.display(), "docker compose up -d");
        // no .env with SMT5_VERSION: the image of this release
        assert_eq!(c.env, vec![("SMT5_VERSION".into(), env!("CARGO_PKG_VERSION").into()), ("DOCKER_CONTEXT".into(), "colima-x86".into())]);
        assert_eq!(c.cwd.as_deref(), Some(std::path::Path::new("/repo/runtime")));
    }

    #[test]
    fn state_and_others() {
        let p = mac();
        let rt = RuntimeDir::new("/r");
        let m = MockRunner::new()
            .on("docker inspect", 0, "running|healthy\n")
            .on("docker ps", 0, "siliconmt5\nkozmos-postgres\n")
            .on("docker info", 0, "x86_64\n")
            .on("docker --version", 0, "Docker version 29.0.1, build eedd969\n");
        let d = Docker::new(&m, &p, &rt);
        assert_eq!(d.client_version().as_deref(), Some("29.0.1"));
        assert_eq!(d.container_state().unwrap(), ContainerState { status: "running".into(), health: Some("healthy".into()) });
        assert_eq!(d.other_containers(), vec!["kozmos-postgres".to_owned()]);
        assert_eq!(d.engine_arch().as_deref(), Some("x86_64"));
    }

    #[test]
    fn stats_parsing() {
        let s = parse_stats("42.10%|5.1GiB / 7.7GiB|66.2%").unwrap();
        assert_eq!(s.cpu_percent, 42.1);
        assert_eq!(s.mem_used_bytes, (5.1 * 1024f64.powi(3)) as u64);
        assert_eq!(s.mem_percent, 66.2);
        assert_eq!(parse_stats("--|--|--"), None);
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("512KiB"), Some(512 * 1024));
    }

    #[test]
    fn volumes_and_image() {
        let p = mac();
        let rt = RuntimeDir::new("/r");
        let m = MockRunner::new().on("docker volume ls", 0, "siliconmt5_siliconmt5_config\n").on("docker image inspect", 1, "");
        let d = Docker::new(&m, &p, &rt);
        assert!(d.has_volumes());
        assert!(!d.image_present("x"));
        let m = MockRunner::new().on("docker volume ls", 0, "\n");
        assert!(!Docker::new(&m, &p, &rt).has_volumes());
    }

    #[test]
    fn missing_container() {
        let p = mac();
        let rt = RuntimeDir::new("/r");
        let m = MockRunner::new().on("docker inspect", 1, "");
        assert_eq!(Docker::new(&m, &p, &rt).container_state(), None);
    }
}
