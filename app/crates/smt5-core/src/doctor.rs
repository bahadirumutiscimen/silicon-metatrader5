//! Prerequisite and health checks — `smt5 doctor` and setup-wizard step 1.
//!
//! Every check that can fail carries the exact command that fixes it, so the
//! wizard can show (and, with consent, run) it.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::api::ApiClient;
use crate::docker::{global_context, Docker};
use crate::env::EnvFile;
use crate::exec::{Cmd, Runner};
use crate::provider::{HostInfo, Provider, ProviderKind, VmSize};
use crate::runtime::RuntimeDir;

/// Minimum VM disk: image layers (stored twice by containerd) + MT5 history.
pub const MIN_VM_DISK_GIB: u64 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub title: String,
    pub level: Level,
    pub detail: String,
    /// Command (or instruction) that fixes the problem.
    pub fix: Option<String>,
}

fn check(id: &str, title: &str, level: Level, detail: impl Into<String>, fix: Option<String>) -> Check {
    Check { id: id.into(), title: title.into(), level, detail: detail.into(), fix }
}

pub struct Context<'a> {
    pub runner: &'a dyn Runner,
    pub host: HostInfo,
    pub provider: Provider,
    pub runtime: RuntimeDir,
    pub env: EnvFile,
    pub api: ApiClient,
}

impl<'a> Context<'a> {
    pub fn new(runner: &'a dyn Runner, runtime: RuntimeDir) -> Self {
        let env = runtime.load_env();
        let host = HostInfo::current();
        let provider = Provider::detect(&env, &host);
        let api = ApiClient::from_env(&env);
        Self { runner, host, provider, runtime, env, api }
    }

    pub fn docker(&self) -> Docker<'_> {
        Docker::new(self.runner, &self.provider, &self.runtime)
    }
}

fn port_in_use(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

pub fn run(ctx: &Context) -> Vec<Check> {
    let mut out = Vec::new();
    let p = &ctx.provider;
    let ctx_name = p.docker_context.clone().unwrap_or_else(global_context);
    let mac = ctx.host.os == "macos";

    out.push(check(
        "host",
        "Host",
        Level::Info,
        format!("{} {} — {}", ctx.host.os, ctx.host.arch, p.kind.label()),
        None,
    ));
    if p.kind.is_native() {
        native_checks(ctx, &mut out);
        return out;
    }

    // docker CLI
    let docker = ctx.docker();
    match docker.client_version() {
        Some(v) => out.push(check("docker-cli", "Docker CLI", Level::Ok, format!("docker {v}"), None)),
        None => {
            out.push(check(
                "docker-cli",
                "Docker CLI",
                Level::Fail,
                "docker command not found",
                Some(if mac { "brew install docker docker-compose".into() } else { "install Docker Engine: https://docs.docker.com/engine/install/".into() }),
            ));
            return out;
        }
    }

    // `docker compose` is a CLI plugin: Homebrew's docker-compose must be linked
    // into ~/.docker/cli-plugins (removing Docker Desktop / OrbStack can drop it).
    if !ctx.runner.run(&Cmd::new("docker").args(["compose", "version"])).ok() {
        out.push(check(
            "compose",
            "Docker Compose",
            Level::Fail,
            "`docker compose` plugin not found",
            Some(if mac {
                "mkdir -p ~/.docker/cli-plugins && ln -sfn \"$(brew --prefix)/opt/docker-compose/bin/docker-compose\" ~/.docker/cli-plugins/docker-compose".into()
            } else {
                "install the compose plugin: https://docs.docker.com/compose/install/linux/".into()
            }),
        ));
        return out;
    }

    // Colima VM (macOS)
    if p.kind.uses_colima() {
        let colima_ok = ctx.runner.run(&Cmd::new("colima").arg("version")).ok();
        if !colima_ok {
            out.push(check(
                "colima",
                "Colima",
                Level::Fail,
                "colima command not found",
                Some(if mac { "brew install colima qemu lima-additional-guestagents".into() } else { "install colima: https://github.com/abiosoft/colima".into() }),
            ));
            return out;
        }
        let start = p.vm_start_cmd(VmSize::default()).map(|c| c.display());
        let want = "x86_64";
        let title = "x86_64 VM";
        match p.vm_profile(ctx.runner) {
            None => out.push(check("vm", title, Level::Fail, format!("Colima profile '{}' does not exist", p.colima_profile.as_deref().unwrap_or("x86")), start)),
            Some(vm) if vm.arch != want => out.push(check(
                "vm",
                title,
                Level::Fail,
                format!("profile '{}' is {} — this mode needs {want}", vm.name, vm.arch),
                Some("use another profile name: set SMT5_COLIMA_PROFILE in .env, then smt5 vm start".into()),
            )),
            Some(vm) if !vm.running() => out.push(check("vm", title, Level::Fail, format!("profile '{}' is {}", vm.name, vm.status), Some(format!("colima start --profile {}", vm.name)))),
            Some(vm) => {
                let disk_gib = vm.disk / (1024 * 1024 * 1024);
                if disk_gib < MIN_VM_DISK_GIB {
                    out.push(check(
                        "vm",
                        title,
                        Level::Warn,
                        format!("running, but the disk is only {disk_gib} GiB (≥ {MIN_VM_DISK_GIB} GiB recommended)"),
                        Some(format!("colima stop --profile {0} && colima start --profile {0} --disk 60", vm.name)),
                    ));
                } else {
                    out.push(check(
                        "vm",
                        title,
                        Level::Ok,
                        format!("'{}' running · {} CPU · {} GiB RAM · {disk_gib} GiB disk", vm.name, vm.cpus, vm.memory / (1024 * 1024 * 1024)),
                        None,
                    ));
                }
            }
        }
    }

    // Docker engine behind the pinned context
    match docker.engine_arch() {
        None => {
            let fix = match p.kind {
                ProviderKind::MacColima => "smt5 vm start",
                ProviderKind::MacNative | ProviderKind::LinuxNative => "smt5 up",
                ProviderKind::LinuxDocker => "sudo systemctl start docker (and add yourself to the docker group)",
                ProviderKind::RemoteSsh => "check the SSH docker context: docker --context <name> info",
            };
            // A context that does not exist yet is a setup step, not an outage.
            let exists = ctx.runner.run(&Cmd::new("docker").args(["context", "inspect", ctx_name.as_str()])).ok();
            if !exists {
                out.push(check("engine", "Docker engine", Level::Fail, format!("docker context '{ctx_name}' does not exist yet"), Some(fix.to_owned())));
            } else {
                out.push(check("engine", "Docker engine", Level::Fail, format!("context '{ctx_name}' is not reachable"), Some(fix.to_owned())));
            }
            return out;
        }
        Some(a) if a == "x86_64" => out.push(check("engine", "Docker engine", Level::Ok, format!("context '{ctx_name}' · x86_64"), None)),
        Some(a) => {
            out.push(check(
                "engine",
                "Docker engine",
                Level::Fail,
                format!("context '{ctx_name}' runs {a}: MT5 would run under Rosetta/qemu-user and Wine crashes"),
                Some(if mac {
                    "use the x86_64 VM (smt5 vm start) or the Mac yerel mode (smt5 mode native)".into()
                } else {
                    "Linux ARM64 is not supported: MT5 needs an x86_64 Linux host".into()
                }),
            ));
            return out;
        }
    }

    // The global context belongs to other projects.
    let global = global_context();
    if p.docker_context.as_deref() == Some(global.as_str()) && p.kind.manages_engine() {
        out.push(check(
            "global-context",
            "Global docker context",
            Level::Warn,
            format!("the global context is '{global}' (colima start switches it): other projects would run in SiliconMetaTrader5's engine"),
            Some("docker context use colima".into()),
        ));
    } else {
        out.push(check("global-context", "Global docker context", Level::Info, format!("'{global}' (other projects) — SiliconMetaTrader5 pins '{ctx_name}'"), None));
    }

    if p.kind.uses_colima() {
        let others = docker.other_containers();
        if others.is_empty() {
            out.push(check("vm-neighbours", "Other containers in the VM", Level::Ok, "none", None));
        } else {
            out.push(check(
                "vm-neighbours",
                "Other containers in the VM",
                Level::Warn,
                format!("{} — they are emulated too and slow MT5 down", others.join(", ")),
                Some(format!("docker --context {ctx_name} stop {}", others.join(" "))),
            ));
        }
    }

    out.push(env_check(ctx, false));

    // Container + runtime
    let state = docker.container_state();
    let running = state.as_ref().map(|s| s.status == "running").unwrap_or(false);
    match &state {
        None => out.push(check("container", "Container", Level::Info, "not created yet", Some("smt5 build && smt5 up".into()))),
        Some(s) if running => out.push(check(
            "container",
            "Container",
            if s.health.as_deref() == Some("unhealthy") { Level::Warn } else { Level::Ok },
            format!("running{}", s.health.as_ref().map(|h| format!(" ({h})")).unwrap_or_default()),
            None,
        )),
        Some(s) => out.push(check("container", "Container", Level::Warn, s.status.clone(), Some("smt5 up".into()))),
    }

    if running {
        let starting = state.as_ref().and_then(|s| s.health.as_deref()) == Some("starting");
        runtime_checks(ctx, &mut out, starting);
    } else {
        port_checks(ctx, &mut out, &[("SMT5_DISPLAY_PORT", 6081u16), ("SMT5_BRIDGE_PORT", 8001), ("SMT5_STATUS_PORT", 8002)]);
    }

    out
}

fn env_check(ctx: &Context, native: bool) -> Check {
    if !ctx.runtime.env_path().exists() {
        return check("env", "Configuration (.env)", Level::Warn, "runtime/.env does not exist yet", Some("smt5 init".into()));
    }
    let mut missing = Vec::new();
    // The native mode has no web desktop: the MT5 window is a macOS window.
    if !native && ctx.env.get_nonempty("SMT5_WEB_PASSWORD").is_none() {
        missing.push("SMT5_WEB_PASSWORD (desktop is unprotected)");
    }
    if ctx.env.get_nonempty("SMT5_API_TOKEN").is_none() {
        missing.push("SMT5_API_TOKEN (data purge / restarts disabled)");
    }
    if missing.is_empty() {
        let what = if native { "API token set" } else { "password and API token set" };
        check("env", "Configuration (.env)", Level::Ok, what, None)
    } else {
        check("env", "Configuration (.env)", Level::Warn, format!("empty: {}", missing.join(", ")), Some("smt5 init".into()))
    }
}

fn port_checks(ctx: &Context, out: &mut Vec<Check>, ports: &[(&str, u16)]) {
    let mut busy = Vec::new();
    let mut all = Vec::new();
    for (key, default) in ports {
        let port = ctx.env.get_nonempty(key).and_then(|v| v.parse().ok()).unwrap_or(*default);
        all.push(port.to_string());
        if port_in_use(port) {
            busy.push(format!("{port} ({key})"));
        }
    }
    if busy.is_empty() {
        out.push(check("ports", "Ports", Level::Ok, format!("{} free", all.join(" / ")), None));
    } else {
        out.push(check("ports", "Ports", Level::Warn, format!("in use by another program: {}", busy.join(", ")), Some("change the port in runtime/.env (or stop the other mode: smt5 down)".into())));
    }
}

/// Status API answers: phase, CPU translation, disk.
fn runtime_checks(ctx: &Context, out: &mut Vec<Check>, starting: bool) {
    let native = ctx.provider.kind.is_native();
        match ctx.api.status() {
            // Right after `up` the status server needs a few seconds; that is not a problem.
            Err(_) if starting => out.push(check(
                "runtime",
                "Runtime",
                Level::Info,
                "runtime is starting, status API not up yet",
                Some("smt5 watch".into()),
            )),
            Err(e) => out.push(check("runtime", "Runtime", Level::Warn, e.to_string(), Some("wait a minute, then smt5 status".into()))),
            Ok(s) => {
                let detail = if s.detail.is_empty() { s.supervisor.detail.clone() } else { s.detail.clone() };
                let text = if detail.is_empty() { s.phase.clone() } else { format!("{} — {detail}", s.phase) };
                let level = match s.phase.as_str() {
                    "ready" => Level::Ok,
                    "error" => Level::Fail,
                    _ => Level::Info,
                };
                out.push(check("runtime", "Runtime", level, text, (s.phase == "error").then(|| "smt5 logs supervisor".to_owned())));
                if native {
                    out.push(check("translation", "CPU", Level::Info, "x86_64 Wine under Rosetta 2 (Mac yerel mod)", None));
                } else if !s.host.cpu_translation.is_empty() && s.host.cpu_translation != "none" {
                    out.push(check("translation", "CPU", Level::Fail, format!("container runs under {} translation", s.host.cpu_translation), Some("use a full x86_64 VM".into())));
                }
                if let Some(free) = s.disk.free_mb {
                    let level = if free < 2048 { Level::Warn } else { Level::Ok };
                    out.push(check(
                        "disk",
                        if native { "Disk" } else { "Disk (/config)" },
                        level,
                        format!("{free} MB free of {} MB", s.disk.total_mb.unwrap_or(0)),
                        (level == Level::Warn).then(|| "smt5 data, then smt5 purge … or enlarge the VM disk".to_owned()),
                    ));
                }
            }
        }
}

/// Mac yerel mode: Rosetta 2, host Python, Wine 11, the installed prefix, the agent.
fn native_checks(ctx: &Context, out: &mut Vec<Check>) {
    use crate::native::{self, NativePaths, MIN_WINE};
    let mac = ctx.host.os == "macos";
    if !mac && !(ctx.host.os == "linux" && ctx.host.arch == "x86_64") {
        out.push(check("native", "Native mode", Level::Fail, "only available on macOS and Linux x86_64", Some("smt5 mode safe".into())));
        return;
    }
    let paths = NativePaths::current();
    let f = native::facts(ctx.runner, &paths);

    if !mac {
        // Linux: no translation; a desktop shows MT5 as a window, otherwise Xvfb + noVNC
        let desktop = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
        let has = |t: &str| ctx.runner.run(&Cmd::new("sh").args(["-c", &format!("command -v {t}")])).ok();
        let missing: Vec<&str> = ["Xvfb", "x11vnc", "websockify"].into_iter().filter(|t| !has(t)).collect();
        out.push(if desktop {
            check("display", "Display", Level::Ok, "desktop session: MT5 opens as a window", None)
        } else if missing.is_empty() {
            check("display", "Display", Level::Ok, "headless: Xvfb + noVNC on 127.0.0.1", None)
        } else {
            check("display", "Display", Level::Fail, format!("headless and missing: {}", missing.join(", ")), Some("sudo apt install xvfb x11vnc websockify novnc openbox".into()))
        });
    } else if ctx.host.arch == "x86_64" {
        out.push(check("rosetta", "Rosetta 2", Level::Ok, "Intel Mac: no translation needed", None));
    } else if ctx.runner.run(&Cmd::new("arch").args(["-x86_64", "/usr/bin/true"])).ok() {
        out.push(check("rosetta", "Rosetta 2", Level::Ok, "installed", None));
    } else {
        out.push(check("rosetta", "Rosetta 2", Level::Fail, "not installed: x86_64 Wine cannot run", Some("softwareupdate --install-rosetta --agree-to-license".into())));
        return;
    }

    if f.venv {
        out.push(check("python", "Python (status API)", Level::Ok, format!("venv in {}", paths.venv.display()), None));
    } else {
        let o = ctx.runner.run(&Cmd::new("python3").args(["-c", "import sys; print('%d.%d' % sys.version_info[:2])"]));
        let v = o.stdout_trimmed().to_owned();
        let ok = o.ok() && v.split_once('.').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))).map(|ab| ab >= (3, 9)).unwrap_or(false);
        out.push(if ok {
            check("python", "Python (status API)", Level::Ok, format!("python3 {v}; the venv is created on install"), None)
        } else {
            check("python", "Python (status API)", Level::Fail, "python3 ≥ 3.9 not found", Some(if mac { "xcode-select --install" } else { "sudo apt install python3 python3-venv" }.into()))
        });
    }

    match &f.wine {
        Some(w) => {
            let v = ctx.runner.run(&Cmd::new(w.as_str()).arg("--version"));
            let version = v.stdout_trimmed().to_owned();
            match native::wine_major(&version) {
                Some(m) if m >= MIN_WINE => out.push(check("wine", "Wine", Level::Ok, version, None)),
                Some(_) => out.push(check("wine", "Wine", Level::Fail, format!("{version}: Wine {MIN_WINE}+ needed (numpy)"), Some("smt5 build (downloads Wine 11.21; unset SMT5_WINE)".into()))),
                None => out.push(check("wine", "Wine", Level::Warn, format!("{w} does not start: {}", v.stderr.trim()), Some("smt5 build".into()))),
            }
        }
        None => out.push(check("wine", "Wine", Level::Warn, format!("not downloaded yet (Wine 11, {})", if mac { "~215 MB" } else { "~100 MB" }), Some("smt5 build".into()))),
    }

    if f.installed {
        out.push(check("install", "MT5 + Windows Python", Level::Ok, format!("{}", paths.mt5_dir().display()), None));
    } else {
        let mut missing = Vec::new();
        if !f.mt5 {
            missing.push("MetaTrader 5");
        }
        if !f.windows_python {
            missing.push("Windows Python");
        }
        if !f.venv {
            missing.push("status API venv");
        }
        let from = if f.probe_prefix {
            " · the probe's prefix will be reused"
        } else {
            " · MT5 is copied from the Güvenli mode container if it exists (login + history), else installed"
        };
        out.push(check("install", "MT5 + Windows Python", Level::Warn, format!("missing: {}{from}", missing.join(", ")), Some("smt5 build".into())));
    }

    out.push(env_check(ctx, true));

    let agent = if mac { "launchd agent" } else { "runtime service" };
    if f.running {
        out.push(check("agent", agent, Level::Ok, format!("{} running", if mac { native::LABEL } else { native::UNIT }), None));
        runtime_checks(ctx, out, true);
    } else {
        out.push(check("agent", agent, Level::Info, "stopped", Some("smt5 up".into())));
        port_checks(ctx, out, &[("SMT5_BRIDGE_PORT", 8001), ("SMT5_STATUS_PORT", 8002)]);
    }
}

/// Worst level among the checks.
pub fn summary(checks: &[Check]) -> Level {
    checks.iter().map(|c| c.level).max().unwrap_or(Level::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::MockRunner;

    fn ctx<'a>(runner: &'a MockRunner, arch: &str) -> Context<'a> {
        let runtime = RuntimeDir::new(std::env::temp_dir().join("smt5-doctor-none"));
        let env = EnvFile::default();
        let host = HostInfo { os: "macos".into(), arch: arch.into() };
        let provider = Provider::detect(&env, &host);
        assert_eq!(provider.kind, ProviderKind::MacColima);
        Context { runner, host, provider, runtime, env, api: ApiClient::new("http://127.0.0.1:1", None) }
    }

    fn find<'c>(checks: &'c [Check], id: &str) -> &'c Check {
        checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no check {id}: {checks:#?}"))
    }

    const X86: &str = r#"{"name":"x86","status":"Running","arch":"x86_64","cpus":4,"memory":8589934592,"disk":64424509440}"#;

    #[test]
    fn missing_profile_suggests_create_command() {
        let m = MockRunner::new()
            .on("docker --version", 0, "Docker version 29.0.1, build x\n")
            .on("docker compose version", 0, "Docker Compose version v2.40.0\n")
            .on("colima version", 0, "colima version 0.9\n")
            .on("colima list --json", 0, r#"{"name":"default","status":"Running","arch":"aarch64"}"#);
        let checks = run(&ctx(&m, "aarch64"));
        let vm = find(&checks, "vm");
        assert_eq!(vm.level, Level::Fail);
        assert_eq!(vm.fix.as_deref(), Some("colima start --profile x86 --arch x86_64 --vm-type=qemu --cpu 4 --memory 8 --disk 60"));
        assert_eq!(summary(&checks), Level::Fail);
    }

    #[test]
    fn rosetta_engine_is_refused() {
        let m = MockRunner::new()
            .on("docker --version", 0, "Docker version 29.0.1, build x\n")
            .on("docker compose version", 0, "Docker Compose version v2.40.0\n")
            .on("colima version", 0, "x\n")
            .on("colima list --json", 0, X86)
            .on("docker info", 0, "aarch64\n");
        let checks = run(&ctx(&m, "aarch64"));
        assert_eq!(find(&checks, "engine").level, Level::Fail);
        assert!(find(&checks, "engine").detail.contains("Rosetta"));
    }

    #[test]
    fn small_disk_and_neighbours_warn() {
        let small = X86.replace("64424509440", "21474836480");
        let m = MockRunner::new()
            .on("docker --version", 0, "Docker version 29.0.1, build x\n")
            .on("docker compose version", 0, "Docker Compose version v2.40.0\n")
            .on("colima version", 0, "x\n")
            .on("colima list --json", 0, &small)
            .on("docker info", 0, "x86_64\n")
            .on("docker ps", 0, "kozmos-postgres\nsiliconmt5\n")
            .on("docker inspect", 1, "");
        let checks = run(&ctx(&m, "aarch64"));
        assert_eq!(find(&checks, "vm").level, Level::Warn);
        assert!(find(&checks, "vm").fix.as_deref().unwrap().contains("--disk 60"));
        let n = find(&checks, "vm-neighbours");
        assert_eq!(n.level, Level::Warn);
        assert_eq!(n.fix.as_deref(), Some("docker --context colima-x86 stop kozmos-postgres"));
        assert_eq!(find(&checks, "container").level, Level::Info);
        assert_eq!(find(&checks, "env").fix.as_deref(), Some("smt5 init"));
    }

    #[test]
    fn missing_docker_stops_early() {
        let m = MockRunner::new();
        let checks = run(&ctx(&m, "aarch64"));
        assert_eq!(checks.last().unwrap().id, "docker-cli");
        assert_eq!(checks.last().unwrap().fix.as_deref(), Some("brew install docker docker-compose"));
    }
}
