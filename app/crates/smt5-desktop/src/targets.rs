//! Server mode connections: one supervisor thread per server keeps an
//! `ssh -N -L …` tunnel up (with backoff), reads the server's secrets into
//! memory and reports a `Link` state. Nothing here blocks other servers.

use serde::Serialize;
use smt5_core::exec::{build, Cmd};
use smt5_core::remote::{classify_ssh_error, control_exit_cmd, tunnel_cmd, PortBlock, RemotePorts, SshOpts};
use smt5_core::remote_setup::{read_env_script, read_env_script_for, ServerMode};
use smt5_core::{ApiClient, Runner, SystemRunner};
use std::collections::HashMap;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::servers::{exec, ssh_opts, ServerConfig};

/// Connection state of one server, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Link {
    /// Not managed (no such server).
    Unknown,
    Connecting,
    Connected { since: f64 },
    /// kind: key_required | host_key_changed | port_in_use | network
    Failed { kind: String, detail: String, at: f64 },
}

/// What the app reads from the server's `.env` (memory only).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Secrets {
    pub token: Option<String>,
    pub password: Option<String>,
    pub remote: RemotePorts,
    /// The runtime digest on the server; None = not installed.
    pub version: Option<String>,
    pub linger: bool,
    /// Windows: `smt5.ps1 install` finished (`INSTALLED=yes`); VERSION and
    /// .env alone mean only that the runtime was uploaded. Linux: unused.
    pub installed: bool,
}

pub fn parse_secrets(out: &str) -> Secrets {
    let mut s = Secrets::default();
    let nz = |v: &str| (!v.trim().is_empty()).then(|| v.trim().to_owned());
    for line in out.lines() {
        // PowerShell may start its output with a BOM
        let Some((k, v)) = line.trim_start_matches('\u{feff}').split_once('=') else { continue };
        let port = |d: u16| v.trim().parse::<u16>().ok().filter(|p| *p > 0).unwrap_or(d);
        match k.trim() {
            "SMT5_API_TOKEN" => s.token = nz(v),
            "SMT5_WEB_PASSWORD" => s.password = nz(v),
            "SMT5_BRIDGE_PORT" => s.remote.bridge = port(8001),
            "SMT5_STATUS_PORT" => s.remote.status = port(8002),
            "SMT5_DISPLAY_PORT" => s.remote.display = port(6081),
            "VERSION" => s.version = nz(v),
            "LINGER" => s.linger = v.trim() == "yes",
            "INSTALLED" => s.installed = v.trim() == "yes",
            _ => {}
        }
    }
    s
}

/// The status API of a server, through its tunnel.
pub fn api_with(s: &ServerConfig, secrets: &Secrets) -> ApiClient {
    ApiClient::new(format!("http://127.0.0.1:{}", PortBlock(s.port_base).status()), secrets.token.clone())
}

/// 2, 4, 8, 16, 32, then 60 s between reconnects.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt.saturating_add(1)).min(60))
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn private_dir(d: &Path) {
    let _ = std::fs::create_dir_all(d);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
    }
}

/// Kill a tunnel left by an earlier run of the app (its pid file), but only
/// when that pid is still an `ssh -N` (pids get reused).
pub fn kill_stale(dir: &Path, id: &str) {
    let f = dir.join(format!("{id}.pid"));
    if let Some(pid) = std::fs::read_to_string(&f).ok().and_then(|p| p.trim().parse::<u32>().ok()) {
        let o = SystemRunner.run(&Cmd::new("ps").args(["-p".to_owned(), pid.to_string(), "-o".to_owned(), "command=".to_owned()]));
        let cmdline = o.stdout_trimmed();
        if o.ok() && cmdline.contains("ssh") && cmdline.contains("-N") {
            let _ = SystemRunner.run(&Cmd::new("kill").arg(pid.to_string()));
        }
    }
    let _ = std::fs::remove_file(f);
}

pub(crate) struct Slot {
    server: ServerConfig,
    pub(crate) link: Link,
    secrets: Option<Secrets>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

/// All server connections of the app.
#[derive(Clone)]
pub struct Targets {
    config_dir: PathBuf,
    slots: Arc<Mutex<HashMap<String, Slot>>>,
}

impl Targets {
    pub fn new(config_dir: PathBuf) -> Self {
        Targets { config_dir, slots: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Start supervisors for new (or changed) servers, stop removed ones.
    pub fn sync(&self, servers: &[ServerConfig]) {
        let mut start = Vec::new();
        {
            let mut slots = self.slots.lock().unwrap();
            let keep: Vec<&str> = servers.iter().map(|s| s.id.as_str()).collect();
            let gone: Vec<String> = slots.keys().filter(|k| !keep.contains(&k.as_str())).cloned().collect();
            for id in gone {
                if let Some(slot) = slots.remove(&id) {
                    stop_slot(&slot);
                }
            }
            for s in servers {
                let changed = slots.get(&s.id).map(|old| old.server != *s).unwrap_or(true);
                if !changed {
                    continue;
                }
                if let Some(old) = slots.remove(&s.id) {
                    stop_slot(&old);
                }
                let slot = Slot { server: s.clone(), link: Link::Connecting, secrets: None, stop: Arc::new(AtomicBool::new(false)), child: Arc::new(Mutex::new(None)) };
                start.push((s.clone(), slot.stop.clone(), slot.child.clone()));
                slots.insert(s.id.clone(), slot);
            }
        }
        for (s, stop, child) in start {
            let me = self.clone();
            std::thread::spawn(move || me.supervise(s, stop, child));
        }
    }

    pub fn link(&self, id: &str) -> Link {
        self.slots.lock().unwrap().get(id).map(|s| s.link.clone()).unwrap_or(Link::Unknown)
    }

    pub fn secrets(&self, id: &str) -> Option<Secrets> {
        self.slots.lock().unwrap().get(id).and_then(|s| s.secrets.clone())
    }

    pub fn api(&self, s: &ServerConfig) -> Option<ApiClient> {
        self.secrets(&s.id).map(|sec| api_with(s, &sec))
    }

    /// Read the server's `.env` again (after install / update / settings).
    pub fn refresh_secrets(&self, id: &str) {
        let Some(server) = self.slots.lock().unwrap().get(id).map(|s| s.server.clone()) else { return };
        if let Ok(sec) = read_secrets(&self.config_dir, &server) {
            self.set(id, |slot| slot.secrets = Some(sec));
        }
    }

    pub fn stop(&self, id: &str) {
        if let Some(slot) = self.slots.lock().unwrap().remove(id) {
            stop_slot(&slot);
        }
    }

    pub fn stop_all(&self) {
        for (_, slot) in self.slots.lock().unwrap().drain() {
            stop_slot(&slot);
        }
    }

    fn set(&self, id: &str, f: impl FnOnce(&mut Slot)) {
        if let Some(slot) = self.slots.lock().unwrap().get_mut(id) {
            f(slot);
        }
    }

    /// `set` for a supervisor: only while `stop` is still the slot's own flag
    /// (a replaced supervisor must not write into its successor's slot).
    pub(crate) fn set_if(&self, id: &str, stop: &Arc<AtomicBool>, f: impl FnOnce(&mut Slot)) {
        if let Some(slot) = self.slots.lock().unwrap().get_mut(id) {
            if Arc::ptr_eq(&slot.stop, stop) {
                f(slot);
            }
        }
    }

    fn is_current(&self, id: &str, stop: &Arc<AtomicBool>) -> bool {
        self.slots.lock().unwrap().get(id).is_some_and(|slot| Arc::ptr_eq(&slot.stop, stop))
    }

    #[cfg(test)]
    pub(crate) fn stop_flag(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.slots.lock().unwrap().get(id).map(|s| s.stop.clone())
    }

    fn supervise(&self, s: ServerConfig, stop: Arc<AtomicBool>, child_slot: Arc<Mutex<Option<Child>>>) {
        let opts = ssh_opts(&self.config_dir, &s);
        private_dir(&opts.dir);
        private_dir(&opts.control_dir);
        let Ok(target) = s.target() else {
            self.set_if(&s.id, &stop, |slot| slot.link = Link::Failed { kind: "network".into(), detail: format!("invalid SSH target {}", s.ssh), at: now() });
            return;
        };
        kill_stale(&opts.dir, &s.id);
        let _ = SystemRunner.run(&control_exit_cmd(&target, &opts));
        let mut attempt = 0u32;
        while !stop.load(Ordering::Relaxed) {
            self.set_if(&s.id, &stop, |slot| slot.link = Link::Connecting);
            let secrets = read_secrets(&self.config_dir, &s);
            if stop.load(Ordering::Relaxed) {
                break; // replaced or removed while ssh was running
            }
            let failure = match secrets {
                Err(e) => Some(e),
                Ok(sec) => {
                    let remote = sec.remote;
                    self.set_if(&s.id, &stop, |slot| slot.secrets = Some(sec));
                    self.run_tunnel(&s, &opts, &target, remote, &stop, &child_slot, &mut attempt)
                }
            };
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if let Some((kind, detail)) = failure {
                self.set_if(&s.id, &stop, |slot| slot.link = Link::Failed { kind, detail, at: now() });
            }
            let wait = backoff(attempt);
            attempt = attempt.saturating_add(1);
            let until = std::time::Instant::now() + wait;
            while std::time::Instant::now() < until && !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }

    /// Run one tunnel process until it exits; Some((kind, detail)) on failure.
    #[allow(clippy::too_many_arguments)]
    fn run_tunnel(
        &self,
        s: &ServerConfig,
        opts: &SshOpts,
        target: &smt5_core::remote::SshTarget,
        remote: RemotePorts,
        stop: &Arc<AtomicBool>,
        child_slot: &Mutex<Option<Child>>,
        attempt: &mut u32,
    ) -> Option<(String, String)> {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let cmd = tunnel_cmd(target, opts, PortBlock(s.port_base), &remote);
        let mut child = match build(&cmd).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(e) => return Some(("network".into(), format!("ssh: {e}"))),
        };
        if self.is_current(&s.id, stop) {
            let _ = std::fs::write(opts.dir.join(format!("{}.pid", s.id)), child.id().to_string());
        }
        let err_buf = Arc::new(Mutex::new(String::new()));
        if let Some(mut pipe) = child.stderr.take() {
            let buf = err_buf.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 1024];
                while let Ok(n) = pipe.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    let mut b = buf.lock().unwrap();
                    b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                    if b.len() > 8192 {
                        let cut = b.len() - 4096;
                        b.drain(..cut);
                    }
                }
            });
        }
        *child_slot.lock().unwrap() = Some(child);
        let local: SocketAddr = ([127, 0, 0, 1], PortBlock(s.port_base).status()).into();
        let mut connected_at: Option<std::time::Instant> = None;
        let code = loop {
            if stop.load(Ordering::Relaxed) {
                break None;
            }
            let exited = child_slot.lock().unwrap().as_mut().and_then(|c| c.try_wait().ok().flatten());
            if let Some(st) = exited {
                break st.code();
            }
            if connected_at.is_none() && TcpStream::connect_timeout(&local, Duration::from_millis(300)).is_ok() {
                connected_at = Some(std::time::Instant::now());
                self.set_if(&s.id, stop, |slot| slot.link = Link::Connected { since: now() });
            }
            std::thread::sleep(Duration::from_millis(500));
        };
        // stopped: the child may still run (stop_slot kills only what it finds)
        if let Some(mut c) = child_slot.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if self.is_current(&s.id, stop) {
            let _ = std::fs::remove_file(opts.dir.join(format!("{}.pid", s.id)));
        }
        if connected_at.map(|t| t.elapsed() > Duration::from_secs(60)).unwrap_or(false) {
            *attempt = 0;
        }
        let stderr = err_buf.lock().unwrap().clone();
        let e = classify_ssh_error(if stderr.trim().is_empty() { "ssh tunnel closed" } else { &stderr });
        let _ = code;
        Some((e.kind().to_owned(), e.detail()))
    }
}

fn stop_slot(slot: &Slot) {
    slot.stop.store(true, Ordering::Relaxed);
    if let Some(mut c) = slot.child.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Run `read_env_script` (for the server's mode) on the server;
/// Err((kind, detail)) when ssh fails.
pub fn read_secrets(config_dir: &Path, s: &ServerConfig) -> Result<Secrets, (String, String)> {
    let opts = ssh_opts(config_dir, s);
    let script = ServerMode::parse(&s.mode).map(read_env_script_for).unwrap_or_else(|| read_env_script().to_owned());
    let cmd = exec(s, &opts, &script).map_err(|e| ("network".to_owned(), e))?;
    let o = SystemRunner.run(&cmd);
    if o.status == 255 || o.status < 0 {
        let e = classify_ssh_error(&o.stderr);
        return Err((e.kind().to_owned(), e.detail()));
    }
    Ok(parse_secrets(&o.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::servers::ServerConfig;
    use std::time::Instant;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("smt5-tg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn server(id: &str, ssh: &str, base: u16) -> ServerConfig {
        ServerConfig { id: id.into(), name: id.into(), ssh: ssh.into(), mode: "linux-native".into(), port_base: base, ..Default::default() }
    }

    #[test]
    fn backoff_schedule() {
        let s: Vec<u64> = (0..8).map(|a| backoff(a).as_secs()).collect();
        assert_eq!(s, vec![2, 4, 8, 16, 32, 60, 60, 60]);
    }

    #[test]
    fn secrets_parse() {
        let s = parse_secrets("SMT5_API_TOKEN=tok\nSMT5_WEB_PASSWORD=pw\nSMT5_STATUS_PORT=9002\nVERSION=abc\nLINGER=yes\n");
        assert_eq!((s.token.as_deref(), s.password.as_deref(), s.version.as_deref(), s.linger), (Some("tok"), Some("pw"), Some("abc"), true));
        assert_eq!((s.remote.status, s.remote.bridge, s.remote.display), (9002, 8001, 6081));
        let empty = parse_secrets("VERSION=\nLINGER=no\n");
        assert_eq!((empty.token, empty.version, empty.linger), (None, None, false));
    }

    #[test]
    fn parse_secrets_strips_crlf() {
        let s = parse_secrets("SMT5_API_TOKEN=abc\r\nSMT5_BRIDGE_PORT=8001\r\nVERSION=d1\r\n");
        assert_eq!((s.token.as_deref(), s.version.as_deref(), s.remote.bridge), (Some("abc"), Some("d1"), 8001));
    }

    #[test]
    fn parse_secrets_bom_and_installed() {
        let s = parse_secrets("\u{feff}SMT5_API_TOKEN=abc\r\nVERSION=d1\r\nINSTALLED=yes\r\nLINGER=yes\r\n");
        assert_eq!((s.token.as_deref(), s.installed, s.linger), (Some("abc"), true, true));
        assert!(!parse_secrets("VERSION=d1\nINSTALLED=no\n").installed);
        assert!(!parse_secrets("VERSION=d1\n").installed, "Linux prints no INSTALLED line");
    }

    #[test]
    fn api_uses_block_and_token() {
        let s = server("a", "h", 18000);
        let sec = Secrets { token: Some("t".into()), ..Default::default() };
        let api = api_with(&s, &sec);
        assert!(api.has_token());
        assert_eq!(api.base, "http://127.0.0.1:18002");
    }

    #[test]
    fn unreachable_server_does_not_block() {
        let dir = tmp("unreach");
        let t = Targets::new(dir.clone());
        let a = server("aaaaaa", "127.0.0.1:1", 18000);
        let b = server("bbbbbb", "127.0.0.1:2", 18100);
        let t0 = Instant::now();
        t.sync(&[a.clone(), b.clone()]);
        assert!(t0.elapsed() < Duration::from_millis(200), "sync must not wait for ssh");
        let _ = (t.link("aaaaaa"), t.link("bbbbbb"));
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let both = [t.link("aaaaaa"), t.link("bbbbbb")].iter().all(|l| matches!(l, Link::Failed { kind, .. } if kind == "network"));
            if both {
                break;
            }
            assert!(Instant::now() < deadline, "links: {:?} {:?}", t.link("aaaaaa"), t.link("bbbbbb"));
            std::thread::sleep(Duration::from_millis(100));
        }
        t.sync(&[a]);
        assert!(matches!(t.link("bbbbbb"), Link::Unknown), "removed server has no link");
        t.stop_all();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_supervisor_cannot_write_into_a_new_slot() {
        // Final review I3: a supervisor stopped mid read_secrets must not touch the
        // slot its replacement owns
        let dir = tmp("race");
        let t = Targets::new(dir.clone());
        let a = server("eeeeee", "127.0.0.1:1", 18400);
        t.sync(std::slice::from_ref(&a));
        let old = t.stop_flag("eeeeee").unwrap();
        t.sync(&[ServerConfig { port_base: 18500, ..a.clone() }]);
        assert!(old.load(Ordering::Relaxed), "replaced supervisor is told to stop");
        t.set_if("eeeeee", &old, |slot| slot.link = Link::Connected { since: 9.0 });
        assert_ne!(t.link("eeeeee"), Link::Connected { since: 9.0 });
        let cur = t.stop_flag("eeeeee").unwrap();
        t.set_if("eeeeee", &cur, |slot| slot.link = Link::Connected { since: 9.0 });
        assert_eq!(t.link("eeeeee"), Link::Connected { since: 9.0 });
        t.stop_all();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_tunnel_pid_is_killed() {
        let dir = tmp("stale");
        let mut child = std::process::Command::new("bash").args(["-c", "exec -a 'ssh -N stale' sleep 300"]).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(dir.join("cccccc.pid"), child.id().to_string()).unwrap();
        // an unrelated process with a recycled pid must survive
        let mut other = std::process::Command::new("sleep").arg("300").spawn().unwrap();
        std::fs::write(dir.join("dddddd.pid"), other.id().to_string()).unwrap();
        kill_stale(&dir, "cccccc");
        kill_stale(&dir, "dddddd");
        std::thread::sleep(Duration::from_millis(300));
        assert!(child.try_wait().unwrap().is_some(), "stale ssh -N killed");
        assert!(other.try_wait().unwrap().is_none(), "unrelated process kept");
        assert!(!dir.join("cccccc.pid").exists());
        let _ = other.kill();
        let _ = other.wait();
        let _ = std::fs::remove_dir_all(dir);
    }
}
