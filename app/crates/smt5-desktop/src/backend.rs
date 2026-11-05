//! Everything the UI asks for, built on `smt5-core`. No Tauri types here, so
//! it is unit-testable and the commands in `main.rs` stay thin.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use smt5_core::api::{Accepted, DataTest, Inventory, Logs, PurgeResult, PurgeSpec, RestartTarget, Status};
use smt5_core::docker::{global_context, ContainerState, ContainerStats};
use smt5_core::doctor::{self, Check, Context};
use smt5_core::mode::Mode;
use smt5_core::native::{self, NativeFacts, NativePaths};
use smt5_core::provider::{ColimaProfile, Provider, VmSize};
use smt5_core::setup::{self, InitReport};
use smt5_core::util::random_hex;
use smt5_core::{ApiClient, Cmd, EnvFile, RuntimeDir, SystemRunner};

use crate::i18n::l;
use crate::lf;

pub static RUNNER: SystemRunner = SystemRunner;

pub fn ctx(rt: &RuntimeDir) -> Context<'static> {
    Context::new(&RUNNER, rt.clone())
}

/// GUI apps on macOS start with PATH=/usr/bin:/bin:/usr/sbin:/sbin, so
/// Homebrew's docker/colima/qemu would not be found. Add the usual places.
pub fn extend_path() {
    let home = std::env::var("HOME").unwrap_or_default();
    let extra = [
        "/opt/homebrew/bin".to_owned(),
        "/opt/homebrew/sbin".to_owned(),
        "/usr/local/bin".to_owned(),
        format!("{home}/.docker/bin"),
        format!("{home}/.local/bin"),
        format!("{home}/.cargo/bin"),
        "/snap/bin".to_owned(),
    ];
    let current = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = current.split(':').filter(|s| !s.is_empty()).map(str::to_owned).collect();
    for e in extra {
        if !parts.contains(&e) && Path::new(&e).is_dir() {
            parts.push(e);
        }
    }
    std::env::set_var("PATH", parts.join(":"));
}

// ------------------------------------------------------------- config ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub runtime_dir: Option<PathBuf>,
    /// "tr" or "en"; None until the user picks one (first-run wizard).
    pub lang: Option<String>,
    /// The first-run wizard finished: until then only the wizard is shown.
    pub setup_complete: bool,
    /// Closing the window: None = ask, "tray" = keep running in the menu bar,
    /// "quit" = stop the runtime and quit.
    pub close_action: Option<String>,
    /// Accounts seen in MT5 (the active one always comes from MT5 itself).
    pub accounts: Vec<KnownAccount>,
    /// Server mode: the VPSes this app manages.
    pub servers: Vec<crate::servers::ServerConfig>,
    /// What the pages show: "local" or a server id.
    pub active_target: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            runtime_dir: None,
            lang: None,
            setup_complete: false,
            close_action: None,
            accounts: Vec::new(),
            servers: Vec::new(),
            active_target: "local".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KnownAccount {
    pub login: u64,
    pub server: String,
    pub name: String,
    pub company: String,
    pub currency: String,
    pub trade_mode: String,
    pub last_seen: f64,
}

impl KnownAccount {
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        let login = v.get("login")?.as_u64()?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_owned();
        Some(Self { login, server: s("server"), name: s("name"), company: s("company"), currency: s("currency"), trade_mode: s("trade_mode"), last_seen: now_secs() })
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Put the active account first in the list (added, or refreshed).
pub fn remember_account(list: &mut Vec<KnownAccount>, acc: KnownAccount) {
    list.retain(|a| !(a.login == acc.login && a.server == acc.server));
    list.insert(0, acc);
    list.truncate(30);
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Accounts {
    /// Logged-in account right now, None when MT5 is not logged in.
    pub current: Option<KnownAccount>,
    pub connected: bool,
    pub known: Vec<KnownAccount>,
    pub error: Option<String>,
}

/// Ask MT5 which account is active and remember it.
pub fn accounts(api: &ApiClient, config_path: &Path) -> Accounts {
    let mut cfg = AppConfig::load(config_path);
    let mut out = Accounts::default();
    match api.account() {
        Ok(v) => {
            out.connected = v.get("connected").and_then(|c| c.as_bool()).unwrap_or(false);
            out.current = v.get("account").and_then(KnownAccount::from_json);
            if let Some(a) = &out.current {
                remember_account(&mut cfg.accounts, a.clone());
                let _ = cfg.save(config_path);
            }
        }
        Err(e) => out.error = Some(e.to_string()),
    }
    out.known = cfg.accounts;
    out
}

pub fn account_history(api: &ApiClient, range: &str) -> Result<serde_json::Value, String> {
    api.account_history(range).map_err(|e| e.to_string())
}

pub fn account_forget(config_path: &Path, login: u64, server: &str) -> Result<Vec<KnownAccount>, String> {
    let mut cfg = AppConfig::load(config_path);
    cfg.accounts.retain(|a| !(a.login == login && a.server == server));
    cfg.save(config_path).map_err(|e| e.to_string())?;
    Ok(cfg.accounts)
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
    }
}

/// Saved folder first, then the usual discovery (env var, cwd, the app's
/// bundled resources, next to the binary).
pub fn find_runtime(cfg: &AppConfig, bundled: &[PathBuf]) -> Result<RuntimeDir, String> {
    if let Some(p) = &cfg.runtime_dir {
        if let Ok(rt) = RuntimeDir::discover(Some(p)) {
            return Ok(rt);
        }
    }
    RuntimeDir::discover_with(None, bundled).map_err(|e| e.to_string())
}

/// Accept the repository folder or its `runtime/` subfolder.
pub fn runtime_from_pick(path: &Path) -> Result<RuntimeDir, String> {
    RuntimeDir::discover(Some(path))
        .or_else(|_| RuntimeDir::discover(Some(&path.join("runtime"))))
        .map_err(|_| lf!("{} bir SiliconMetaTrader5 klasörü değil (runtime/compose.yaml bulunamadı)", "{} is not a SiliconMetaTrader5 folder (runtime/compose.yaml not found)", path.display()))
}

// ----------------------------------------------------------- snapshot ----

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub runtime_dir: String,
    pub provider: String,
    pub provider_kind: String,
    pub docker_context: Option<String>,
    pub global_context: String,
    pub container: Option<ContainerState>,
    pub status: Option<Status>,
    pub status_error: Option<String>,
    pub display: String,
    pub display_port: u16,
    pub display_url: String,
    pub bridge_port: u16,
    pub status_port: u16,
    pub has_password: bool,
    pub has_token: bool,
    pub env_exists: bool,
    pub image: String,
    /// What the runtime is doing, for buttons and labels — see [`lifecycle`].
    pub lifecycle: String,
    /// Can the app start the engine itself (Colima VM)?
    pub manages_vm: bool,
    /// Runtime mode (`safe`, `native`) and what to call the engine.
    pub mode: String,
    pub mode_label: String,
    pub engine_label: String,
    /// Mac yerel mode: no container, the MT5 window is a macOS window.
    pub native: bool,
    pub native_home: Option<String>,
    /// What this snapshot is about: "local" or a server id (Server mode).
    pub target: String,
    pub target_name: String,
    /// Server mode: the ssh tunnel state.
    pub link: Option<crate::targets::Link>,
    /// Server mode: the server runs another runtime version than this app.
    pub update_needed: bool,
    /// Server mode: systemd linger is on (runtime survives SSH logout).
    pub linger: bool,
}

/// Facts the lifecycle is derived from (only the cheap ones are collected
/// while the container exists).
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub env_exists: bool,
    pub container: Option<ContainerState>,
    pub phase: Option<String>,
    pub engine_up: bool,
    pub has_volumes: bool,
    pub image_present: bool,
}

/// One word for the runtime's state:
/// `engine_down` (VM/Docker off) · `no_config` (no .env) · `no_image` ·
/// `not_created` (image ready, never started) · `stopped` (installed: a
/// stopped container or volumes after `down`) · `booting` (container up,
/// status API not yet) · otherwise the supervisor phase (`ready`,
/// `waiting_login`, `installing_mt5`, `error`, …).
pub fn lifecycle(f: &Facts) -> String {
    if let Some(c) = &f.container {
        return match c.status.as_str() {
            "running" => f.phase.clone().unwrap_or_else(|| "booting".into()),
            "restarting" => "booting".into(),
            _ => "stopped".into(),
        };
    }
    if !f.engine_up {
        "engine_down"
    } else if f.has_volumes {
        "stopped"
    } else if !f.env_exists {
        "no_config"
    } else if !f.image_present {
        "no_image"
    } else {
        "not_created"
    }
    .into()
}

/// Lifecycle of the Mac yerel mode: `no_config` · `not_installed` (Wine,
/// prefix, MT5 or venv missing) · `stopped` · `booting` · supervisor phase.
pub fn native_lifecycle(env_exists: bool, f: &NativeFacts, phase: Option<String>) -> String {
    if f.running {
        return phase.unwrap_or_else(|| "booting".into());
    }
    if !env_exists {
        "no_config"
    } else if !f.installed {
        "not_installed"
    } else {
        "stopped"
    }
    .into()
}

pub fn image_name(env: &EnvFile) -> String {
    let version = env.get_nonempty("SMT5_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    format!("ghcr.io/bahadirumutiscimen/siliconmt5:{version}")
}

fn port(env: &EnvFile, key: &str, default: u16) -> u16 {
    env.get_nonempty(key).and_then(|p| p.parse().ok()).unwrap_or(default)
}

/// URL of the noVNC desktop. The password goes in the fragment (never sent
/// over HTTP) so the viewer connects without asking.
pub fn display_url(port: u16, password: Option<&str>) -> String {
    let mut url = format!("http://127.0.0.1:{port}/vnc.html?autoconnect=1&resize=scale&reconnect=1&reconnect_delay=2000");
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        url.push_str("#password=");
        url.push_str(&percent_encode(p));
    }
    url
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn snapshot(rt: &RuntimeDir) -> Snapshot {
    let c = ctx(rt);
    let mode = Mode::current(&c.env);
    let env_exists = rt.env_path().exists();
    let status_of = |up: bool| if up {
        match c.api.status() {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e.to_string())),
        }
    } else {
        (None, None)
    };
    let mut snap = Snapshot {
        runtime_dir: rt.root.display().to_string(),
        provider: c.provider.kind.label().to_owned(),
        provider_kind: serde_json::to_value(c.provider.kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default(),
        docker_context: c.provider.docker_context.clone(),
        global_context: String::new(),
        container: None,
        status: None,
        status_error: None,
        display: String::new(),
        display_port: port(&c.env, "SMT5_DISPLAY_PORT", 6081),
        display_url: String::new(),
        bridge_port: port(&c.env, "SMT5_BRIDGE_PORT", 8001),
        status_port: port(&c.env, "SMT5_STATUS_PORT", 8002),
        has_password: c.env.get_nonempty("SMT5_WEB_PASSWORD").is_some(),
        has_token: c.api.has_token(),
        env_exists,
        image: String::new(),
        lifecycle: String::new(),
        manages_vm: c.provider.kind.uses_colima(),
        mode: mode.id().to_owned(),
        mode_label: mode.label().to_owned(),
        engine_label: if c.provider.kind.uses_colima() { "VM" } else { "Docker" }.to_owned(),
        native: false,
        native_home: None,
        target: "local".to_owned(),
        target_name: String::new(),
        link: None,
        update_needed: false,
        linger: false,
    };

    if c.provider.kind.is_native() {
        let paths = NativePaths::current();
        let f = native::facts(&RUNNER, &paths);
        let (status, status_error) = status_of(f.running);
        snap.lifecycle = native_lifecycle(env_exists, &f, status.as_ref().map(|s| s.phase.clone()).filter(|p| !p.is_empty()));
        snap.container = native::pseudo_container(&f);
        snap.status = status;
        snap.status_error = status_error;
        let mac = cfg!(target_os = "macos");
        // Linux without a desktop: the supervisor runs Xvfb + noVNC (state/display)
        match paths.display() {
            native::NativeDisplay::NoVnc(port) if !mac => {
                snap.display = "novnc".into();
                snap.display_port = port;
                snap.display_url = display_url(port, c.env.get_nonempty("SMT5_WEB_PASSWORD"));
            }
            _ => snap.display = "native".into(),
        }
        snap.image = f.wine.as_deref().map(|_| if mac { "Wine 11 + Rosetta 2" } else { "Wine 11" }).unwrap_or("Wine indirilmedi").to_owned();
        snap.engine_label = if mac { "Mac" } else { "Linux" }.into();
        snap.native = true;
        snap.native_home = Some(paths.base.display().to_string());
        return snap;
    }

    let docker = c.docker();
    let container = docker.container_state();
    let running = container.as_ref().map(|s| s.status == "running").unwrap_or(false);
    let (status, status_error) = status_of(running);
    let version = c.env.get_nonempty("SMT5_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    let mut facts = Facts {
        env_exists,
        phase: status.as_ref().map(|s| s.phase.clone()).filter(|p| !p.is_empty()),
        engine_up: container.is_some(),
        container: container.clone(),
        ..Default::default()
    };
    if container.is_none() {
        facts.engine_up = docker.engine_up();
        if facts.engine_up {
            facts.has_volumes = docker.has_volumes();
            facts.image_present = docker.image_present(&image_name(&c.env));
        }
    }
    snap.global_context = global_context();
    snap.lifecycle = lifecycle(&facts);
    snap.container = container;
    snap.status = status;
    snap.status_error = status_error;
    snap.display_url = display_url(snap.display_port, c.env.get_nonempty("SMT5_WEB_PASSWORD"));
    snap.image = format!("siliconmt5:{version}");
    snap.display = "novnc".into();
    snap
}

#[derive(Debug, Clone, Serialize)]
pub struct Resources {
    pub container: Option<ContainerStats>,
    pub vm: Option<ColimaProfile>,
}

pub fn resources(rt: &RuntimeDir) -> Resources {
    let c = ctx(rt);
    if c.provider.kind.is_native() {
        let paths = NativePaths::current();
        let running = native::agent_loaded(&RUNNER);
        return Resources { container: running.then(|| native::stats(&RUNNER, &paths)).flatten(), vm: None };
    }
    let vm = if c.provider.kind.uses_colima() { c.provider.vm_profile(&RUNNER) } else { None };
    Resources { container: c.docker().stats(), vm }
}

pub fn doctor(rt: &RuntimeDir) -> Vec<Check> {
    doctor::run(&ctx(rt))
}

pub fn logs(api: &ApiClient, source: &str, lines: u32) -> Result<Logs, String> {
    api.logs(source, lines).map_err(|e| e.to_string())
}

pub fn clear_logs(api: &ApiClient) -> Result<smt5_core::api::Cleared, String> {
    if !api.has_token() {
        return Err(l("SMT5_API_TOKEN boş: Ayarlar → Ağ ve güvenlik → Token oluştur, sonra uygula", "SMT5_API_TOKEN is empty: Settings → Network and security → create a token, then apply").into());
    }
    api.clear_logs().map_err(|e| e.to_string())
}

pub fn inventory(api: &ApiClient) -> Result<Inventory, String> {
    api.data().map_err(|e| e.to_string())
}

pub fn purge(api: &ApiClient, spec: &PurgeSpec) -> Result<PurgeResult, String> {
    if !api.has_token() {
        return Err(l("SMT5_API_TOKEN boş: Ayarlar → Ağ ve güvenlik → Token oluştur, sonra uygula", "SMT5_API_TOKEN is empty: Settings → Network and security → create a token, then apply").into());
    }
    api.purge(spec).map_err(|e| e.to_string())
}

pub fn supertrend(api: &ApiClient, symbol: &str, tf: &str, period: u32, multiplier: f64, bars: u32) -> Result<serde_json::Value, String> {
    api.supertrend(symbol, tf, period, multiplier, bars).map_err(|e| e.to_string())
}

/// Example source files shown on the Education pages (runtime/status/…).
pub const EXAMPLES: &[&str] = &["supertrend_strategy.py"];

pub fn example_source(rt: &RuntimeDir, name: &str) -> Result<String, String> {
    if !EXAMPLES.contains(&name) {
        return Err(format!("unknown example {name}"));
    }
    std::fs::read_to_string(rt.root.join("status").join(name)).map_err(|e| lf!("{name} okunamadı: {e}", "could not read {name}: {e}"))
}

pub fn datatest(api: &ApiClient, symbol: &str, tf: &str, count: u32, ticks: f64) -> Result<DataTest, String> {
    api.datatest(symbol, tf, count, ticks).map_err(|e| e.to_string())
}

pub fn restart(rt: &RuntimeDir, target: &str) -> Result<Accepted, String> {
    let c = ctx(rt);
    let t = match target {
        "mt5" => RestartTarget::Mt5,
        "bridge" => RestartTarget::Bridge,
        "container" => {
            let cmd = if c.provider.kind.is_native() { native::cmd(rt, "restart") } else { c.docker().compose(["restart"]) };
            let o = smt5_core::Runner::run(&RUNNER, &cmd);
            return if o.ok() {
                Ok(Accepted { accepted: true, target: "container".into() })
            } else {
                Err(format!("{}: {}", cmd.display(), o.stderr.trim()))
            };
        }
        other => return Err(format!("unknown restart target {other}")),
    };
    if !c.api.has_token() {
        return Err(l("SMT5_API_TOKEN boş: Ayarlar → Ağ ve güvenlik → Token oluştur, sonra uygula", "SMT5_API_TOKEN is empty: Settings → Network and security → create a token, then apply").into());
    }
    c.api.restart(t).map_err(|e| e.to_string())
}

// ----------------------------------------------------------- settings ----

/// `.env` keys the settings screen may change, with a validator each.
type Validator = fn(&str) -> bool;
const EDITABLE: &[(&str, Validator)] = &[
    ("SMT5_DISPLAY_PORT", valid_port),
    ("SMT5_BRIDGE_PORT", valid_port),
    ("SMT5_STATUS_PORT", valid_port),
    ("SMT5_MAXBARS", |v| v.parse::<u64>().map(|n| (1_000..=10_000_000).contains(&n)).unwrap_or(false)),
    ("SMT5_RESOLUTION", valid_resolution),
    ("SMT5_DPI", |v| v.parse::<u32>().map(|n| (72..=288).contains(&n)).unwrap_or(false)),
    ("TZ", |v| !v.is_empty() && v.len() < 64 && v.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c))),
    ("SMT5_ALGO_TRADING", |v| matches!(v, "0" | "1")),
    ("SMT5_DLL_IMPORT", |v| matches!(v, "0" | "1")),
    ("SMT5_LOG_MAX_MB", |v| v.parse::<u32>().map(|n| (1..=500).contains(&n)).unwrap_or(false)),
];

fn valid_port(v: &str) -> bool {
    v.parse::<u16>().map(|p| p >= 1024).unwrap_or(false)
}

fn valid_resolution(v: &str) -> bool {
    let parts: Vec<&str> = v.split('x').collect();
    (2..=3).contains(&parts.len())
        && parts.iter().all(|p| p.parse::<u32>().is_ok())
        && parts[0].parse::<u32>().map(|w| (800..=7680).contains(&w)).unwrap_or(false)
        && parts[1].parse::<u32>().map(|h| (600..=4320).contains(&h)).unwrap_or(false)
}

#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    pub values: BTreeMap<String, String>,
    pub has_password: bool,
    pub has_token: bool,
    pub env_path: String,
    pub env_exists: bool,
}

pub fn settings(rt: &RuntimeDir) -> Settings {
    let env = rt.load_env();
    let defaults = EnvFile::load_or_default(&rt.env_example_path());
    let values = EDITABLE
        .iter()
        .map(|(k, _)| ((*k).to_owned(), env.get(k).or_else(|| defaults.get(k)).unwrap_or("").to_owned()))
        .collect();
    Settings {
        values,
        has_password: env.get_nonempty("SMT5_WEB_PASSWORD").is_some(),
        has_token: env.get_nonempty("SMT5_API_TOKEN").is_some(),
        env_path: rt.env_path().display().to_string(),
        env_exists: rt.env_path().exists(),
    }
}

/// Validate and write the given keys; returns the keys that changed.
pub fn save_settings(rt: &RuntimeDir, values: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let mut env = if rt.env_path().exists() {
        EnvFile::load(&rt.env_path()).map_err(|e| e.to_string())?
    } else {
        EnvFile::load(&rt.env_example_path()).map_err(|e| e.to_string())?
    };
    let changed = apply_settings(&mut env, values)?;
    if !changed.is_empty() || !rt.env_path().exists() {
        env.save(&rt.env_path()).map_err(|e| e.to_string())?;
    }
    Ok(changed)
}

/// Validate `values` and set them in `env`; the keys that changed.
pub fn apply_settings(env: &mut EnvFile, values: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let mut changed = Vec::new();
    for (k, v) in values {
        let v = v.trim();
        let Some((_, ok)) = EDITABLE.iter().find(|(key, _)| key == k) else {
            return Err(lf!("{k} bu ekrandan değiştirilemez", "{k} cannot be changed here"));
        };
        if !ok(v) {
            return Err(lf!("{k}: geçersiz değer '{v}'", "{k}: invalid value '{v}'"));
        }
        if env.get(k) != Some(v) {
            env.set(k, v);
            changed.push(k.clone());
        }
    }
    let ports: Vec<&str> = ["SMT5_DISPLAY_PORT", "SMT5_BRIDGE_PORT", "SMT5_STATUS_PORT"].iter().filter_map(|k| env.get_nonempty(k)).collect();
    let mut uniq = ports.clone();
    uniq.sort_unstable();
    uniq.dedup();
    if uniq.len() != ports.len() {
        return Err(l("Portlar birbirinden farklı olmalı", "The ports must be different").into());
    }
    Ok(changed)
}

/// Settings of a server from its `.env` text (defaults: this app's example).
/// A Windows server has no noVNC text size (`SMT5_DPI`).
pub fn settings_from_env(env: &EnvFile, defaults: &EnvFile, env_path: &str, windows: bool) -> Settings {
    let values = EDITABLE
        .iter()
        .filter(|(k, _)| !(windows && *k == "SMT5_DPI"))
        .map(|(k, _)| ((*k).to_owned(), env.get(k).or_else(|| defaults.get(k)).unwrap_or("").to_owned()))
        .collect();
    Settings {
        values,
        has_password: env.get_nonempty("SMT5_WEB_PASSWORD").is_some(),
        has_token: env.get_nonempty("SMT5_API_TOKEN").is_some(),
        env_path: env_path.to_owned(),
        env_exists: env.get_nonempty("SMT5_API_TOKEN").is_some(),
    }
}

/// Script that writes validated settings into the server's `.env` (only
/// editable keys; values free of shell and sed specials).
pub fn remote_env_update_script(values: &BTreeMap<String, String>) -> Result<String, String> {
    let mut lines = vec![
        "set -e".to_owned(),
        "E=\"$HOME/siliconmetatrader5/runtime/.env\"".to_owned(),
        "[ -f \"$E\" ]".to_owned(),
        "set_kv() { k=\"${1%%=*}\"; if grep -q \"^$k=\" \"$E\"; then sed -i.bak \"s|^$k=.*|$1|\" \"$E\" && rm -f \"$E.bak\"; else printf '%s\\n' \"$1\" >> \"$E\"; fi; }".to_owned(),
    ];
    for (k, v) in values {
        let v = v.trim();
        let Some((_, ok)) = EDITABLE.iter().find(|(key, _)| key == k) else {
            return Err(lf!("{k} bu ekrandan değiştirilemez", "{k} cannot be changed here"));
        };
        if !ok(v) || v.chars().any(|c| c.is_control() || "'\"\\$`|&;<>".contains(c)) {
            return Err(lf!("{k}: geçersiz değer '{v}'", "{k}: invalid value '{v}'"));
        }
        lines.push(format!("set_kv {}", smt5_core::remote::shell_quote(&format!("{k}={v}"))));
    }
    Ok(lines.join("\n"))
}

/// [`remote_env_update_script`] for `mode`: on Windows the same validated
/// keys are written by PowerShell (LF line ends, UTF-8 without BOM; the
/// file keeps its ACL because it is rewritten in place).
pub fn remote_env_update_script_for(mode: smt5_core::remote_setup::ServerMode, values: &BTreeMap<String, String>) -> Result<String, String> {
    use smt5_core::remote_setup::{ps_quote, ServerMode};
    let linux = remote_env_update_script(values)?; // the same validation for both
    if mode != ServerMode::Windows {
        return Ok(linux);
    }
    let pairs: Vec<String> = values.iter().map(|(k, v)| format!("{} = {}", ps_quote(k), ps_quote(v.trim()))).collect();
    Ok(format!(
        r#"$ErrorActionPreference='Stop'
try {{
$e = Join-Path $env:LOCALAPPDATA 'siliconmetatrader5\runtime\.env'
if (-not (Test-Path -LiteralPath $e)) {{ throw '.env not found' }}
$lines = [System.Collections.Generic.List[string]]@([IO.File]::ReadAllText($e) -split "`r?`n")
$kv = [ordered]@{{ {p} }}
foreach ($k in $kv.Keys) {{
  $hit = $false
  for ($i = 0; $i -lt $lines.Count; $i++) {{ if ($lines[$i].StartsWith("$k=")) {{ $lines[$i] = "$k=$($kv[$k])"; $hit = $true }} }}
  if (-not $hit) {{ $lines.Add("$k=$($kv[$k])") }}
}}
[IO.File]::WriteAllText($e, (($lines -join "`n").TrimEnd() + "`n"), (New-Object Text.UTF8Encoding($false)))
}} catch {{
[Console]::Error.WriteLine($_.Exception.Message)
exit 1
}}"#,
        p = pairs.join("; ")
    ))
}

/// The script that prints the server's `.env`.
pub fn remote_env_script(mode: smt5_core::remote_setup::ServerMode) -> String {
    match mode {
        smt5_core::remote_setup::ServerMode::Windows => {
            "$ErrorActionPreference='Stop'\nGet-Content -LiteralPath (Join-Path $env:LOCALAPPDATA 'siliconmetatrader5\\runtime\\.env')".to_owned()
        }
        _ => "cat \"$HOME/siliconmetatrader5/runtime/.env\"".to_owned(),
    }
}

/// Generate a new desktop password or API token (`which` = password | token).
pub fn regenerate(rt: &RuntimeDir, which: &str) -> Result<InitReport, String> {
    let (key, bytes) = match which {
        "password" => ("SMT5_WEB_PASSWORD", 9),
        "token" => ("SMT5_API_TOKEN", 24),
        _ => return Err(format!("unknown secret {which}")),
    };
    let value = random_hex(bytes).map_err(|e| e.to_string())?;
    let r = setup::init(rt, &[(key, value.as_str())]).map_err(|e| e.to_string())?;
    Ok(InitReport { password: (which == "password").then_some(value), ..r })
}

pub fn reveal_password(rt: &RuntimeDir) -> Option<String> {
    rt.load_env().get_nonempty("SMT5_WEB_PASSWORD").map(str::to_owned)
}

pub fn init(rt: &RuntimeDir) -> Result<InitReport, String> {
    setup::init(rt, &[]).map_err(|e| e.to_string())
}

/// Is the runtime image (compose `image:` for the current .env) present locally?
pub fn image_present(rt: &RuntimeDir) -> Result<bool, String> {
    let c = ctx(rt);
    if c.provider.kind.is_native() {
        return Ok(native::facts(&RUNNER, &NativePaths::current()).installed);
    }
    Ok(c.docker().image_present(&image_name(&c.env)))
}

/// Mac yerel mode: bring the MT5 window to the front. Wine windows belong to
/// the terminal process the supervisor started (state/mt5.pid). macOS asks
/// once whether SiliconMetaTrader5 may control System Events.
pub fn focus_mt5() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return focus_mt5_linux();
    }
    let pid_file = NativePaths::current().state.join("mt5.pid");
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| l("MT5 çalışmıyor (yerel mod)", "MT5 is not running (native mode)").to_owned())?;
    let script = format!("tell application \"System Events\" to set frontmost of (first process whose unix id is {pid}) to true");
    let o = smt5_core::Runner::run(&RUNNER, &Cmd::new("osascript").args(["-e".to_owned(), script]));
    if o.ok() {
        Ok(())
    } else {
        Err(lf!("MT5 penceresi öne getirilemedi: {} (Dock'taki MT5 simgesine tıklayın)", "Could not bring the MT5 window to the front: {} (click the MT5 icon in the Dock)", o.stderr.trim()))
    }
}

/// Linux desktop: raise the MT5 window with wmctrl or xdotool (whichever exists).
fn focus_mt5_linux() -> Result<(), String> {
    let run = |cmd: Cmd| smt5_core::Runner::run(&RUNNER, &cmd);
    if run(Cmd::new("wmctrl").args(["-a", "MetaTrader 5"])).ok() {
        return Ok(());
    }
    if run(Cmd::new("sh").args(["-c", "xdotool search --name 'MetaTrader 5' | tail -1 | xargs -r xdotool windowactivate"])).ok() {
        return Ok(());
    }
    Err(l("MT5 penceresi öne getirilemedi: wmctrl ya da xdotool gerekli (sudo apt install wmctrl)", "Could not raise the MT5 window: wmctrl or xdotool is needed (sudo apt install wmctrl)").into())
}

pub fn bench(api: &ApiClient, symbol: &str, bars: u32) -> Result<smt5_core::api::Bench, String> {
    api.bench(symbol, bars).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct ModeInfo {
    pub id: String,
    pub label: String,
    pub current: bool,
    pub available: bool,
    pub hint: Option<String>,
}

pub fn modes(rt: &RuntimeDir) -> Vec<ModeInfo> {
    let env = rt.load_env();
    let cur = Mode::current(&env);
    let mac = cfg!(target_os = "macos");
    if !mac && std::env::consts::ARCH != "x86_64" {
        return vec![];
    }
    Mode::ALL
        .iter()
        .map(|m| {
            let (available, hint) = match m {
                Mode::Safe => (true, None),
                Mode::Native if !mac => (true, None),
                Mode::Native => {
                    let rosetta = std::env::consts::ARCH == "x86_64" || smt5_core::Runner::run(&RUNNER, &Cmd::new("arch").args(["-x86_64", "/usr/bin/true"])).ok();
                    (rosetta, (!rosetta).then(|| l("Rosetta 2 gerekli: softwareupdate --install-rosetta --agree-to-license", "Rosetta 2 is needed: softwareupdate --install-rosetta --agree-to-license").to_owned()))
                }
            };
            ModeInfo { id: m.id().into(), label: m.label_for(std::env::consts::OS).into(), current: *m == cur, available, hint }
        })
        .collect()
}

/// Switch `.env` to another mode. The data of each mode stays in its own engine.
pub fn set_mode(rt: &RuntimeDir, id: &str) -> Result<Vec<String>, String> {
    let m = Mode::parse(id).ok_or_else(|| lf!("bilinmeyen mod {id}", "unknown mode {id}"))?;
    if m == Mode::Native && !cfg!(target_os = "macos") && std::env::consts::ARCH != "x86_64" {
        return Err(l("Yerel mod yalnız macOS ve Linux x86_64'te", "The native mode needs macOS or Linux x86_64").into());
    }
    // Both modes use the same localhost ports: the running one must stop first.
    let c = ctx(rt);
    let running = if c.provider.kind.is_native() {
        native::agent_loaded(&RUNNER)
    } else {
        c.docker().container_state().map(|s| s.status == "running").unwrap_or(false)
    };
    if running && Mode::current(&c.env) != m {
        return Err(l("Runtime çalışıyor: önce durdurun (aynı portları kullanırlar)", "The runtime is running: stop it first (both modes use the same ports)").into());
    }
    // The wizard may pick the mode before the configuration exists: start from .env.example.
    let src = if rt.env_path().exists() { rt.env_path() } else { rt.env_example_path() };
    let mut env = EnvFile::load(&src).map_err(|e| e.to_string())?;
    let changed = m.apply(&mut env, std::env::consts::OS);
    if !changed.is_empty() || !rt.env_path().exists() {
        env.save(&rt.env_path()).map_err(|e| e.to_string())?;
    }
    Ok(changed)
}

pub fn market_watch(api: &ApiClient) -> Result<smt5_core::api::MarketWatch, String> {
    api.symbols().map_err(|e| e.to_string())
}

// -------------------------------------------------------- Server mode ----

use crate::servers::ServerConfig;
use crate::targets::{Link, Secrets};
use smt5_core::remote::{scp_cmd, PortBlock, SshOpts};
use smt5_core::remote_setup::{env_init_script, lifecycle_script, upload_script_for, Action, ServerMode, WIN_UPLOAD_NAME};

/// Ask the server's status API only when the tunnel is up and it is installed.
pub fn server_wants_status(link: &Link, secrets: Option<&Secrets>) -> bool {
    matches!(link, Link::Connected { .. }) && secrets.is_some_and(|s| s.version.is_some() && s.token.is_some())
}

/// `smt5.ps1 session` output (`session=yes|no task=yes|no`) →
/// (a user session is open, the supervisor task runs).
pub fn parse_session(out: &str) -> Option<(bool, bool)> {
    out.lines().rev().map(|l| l.trim_start_matches('\u{feff}')).find_map(|l| {
        let mut session = None;
        let mut task = None;
        for w in l.split_whitespace() {
            match w.split_once('=') {
                Some(("session", v)) => session = Some(v == "yes"),
                Some(("task", v)) => task = Some(v == "yes"),
                _ => {}
            }
        }
        Some((session?, task?))
    })
}

/// `smt5.ps1 autologon-status` output → automatic logon is on.
pub fn parse_autologon_status(out: &str) -> bool {
    out.lines().any(|l| l.trim_start_matches('\u{feff}').trim() == "on")
}

#[derive(Debug, Clone, Copy, Default)]
struct SessionEntry {
    /// When the stored answer was asked for (None: never answered).
    at: Option<std::time::Instant>,
    value: Option<(bool, bool)>,
    in_flight: bool,
}

/// The Windows session answer per server, asked at most once per TTL and
/// never twice at the same time; each question is bounded by a timeout.
#[derive(Debug, Clone, Default)]
pub struct SessionCache {
    inner: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, SessionEntry>>>,
}

impl SessionCache {
    /// The cached answer while younger than `ttl`; the last answer (or None)
    /// while a probe runs; otherwise `probe` on its own thread, waited for at
    /// most `timeout` (None after that: the probe still fills the cache when
    /// it ends).
    pub fn get<F>(&self, id: &str, now: std::time::Instant, ttl: std::time::Duration, timeout: std::time::Duration, probe: F) -> Option<(bool, bool)>
    where
        F: FnOnce() -> Option<(bool, bool)> + Send + 'static,
    {
        {
            let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let e = map.entry(id.to_owned()).or_default();
            if e.at.is_some_and(|at| now.saturating_duration_since(at) < ttl) || e.in_flight {
                return e.value;
            }
            e.in_flight = true;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let inner = self.inner.clone();
        let id = id.to_owned();
        std::thread::spawn(move || {
            let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(probe)).ok().flatten();
            let mut map = inner.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(id, SessionEntry { at: Some(now), value, in_flight: false });
            drop(map);
            let _ = tx.send(value);
        });
        rx.recv_timeout(timeout).ok().flatten()
    }
}

/// The snapshot of a server target (ports are the local tunnel ends).
/// lifecycle: connecting · unreachable · not_installed · installing ·
/// stopped (API not answering) · booting · supervisor phase; Windows also
/// `no_session` (nobody logged on, so the supervisor task cannot run), and
/// `not_installed` until `smt5.ps1 install` finished (`Secrets::installed`).
/// `session`: Windows only, asked when the API does not answer —
/// (session open, task running); None = not asked.
pub fn server_snapshot(
    s: &ServerConfig,
    link: Link,
    secrets: Option<&Secrets>,
    status: Option<Result<Status, String>>,
    session: Option<(bool, bool)>,
    local_digest: &str,
) -> Snapshot {
    let b = PortBlock(s.port_base);
    let windows = s.mode == "windows";
    let native = s.mode == "linux-native" || windows;
    let (status, status_error) = match status {
        Some(Ok(st)) => (Some(st), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let lifecycle = match (&link, secrets) {
        (Link::Connected { .. }, Some(sec)) => match (&sec.version, &sec.token, &status) {
            (None, _, _) => "not_installed".to_owned(),
            // uploaded but no .env yet (an install stopped half way): install again
            (Some(_), None, _) => "not_installed".to_owned(),
            // Windows: uploaded and .env made, but smt5.ps1 install did not finish
            (Some(_), Some(_), _) if windows && !sec.installed => "not_installed".to_owned(),
            (Some(_), Some(_), Some(st)) if !st.phase.is_empty() => st.phase.clone(),
            (Some(_), Some(_), Some(_)) => "booting".to_owned(),
            (Some(_), Some(_), None) => match session.filter(|_| windows) {
                Some((false, _)) => "no_session".to_owned(),
                Some((true, true)) => "booting".to_owned(),
                _ => "stopped".to_owned(),
            },
        },
        (Link::Failed { .. }, _) => "unreachable".to_owned(),
        _ => "connecting".to_owned(),
    };
    let mode_label = if windows {
        l("Sunucu · Windows", "Server · Windows")
    } else if native {
        l("Sunucu · Linux yerel", "Server · Linux native")
    } else {
        l("Sunucu · Docker", "Server · Docker")
    };
    // pseudo container (like native mode): pages check `container.status`
    let container = if status.is_some() || lifecycle == "booting" {
        Some(ContainerState { status: "running".into(), health: None })
    } else if lifecycle == "stopped" {
        Some(ContainerState { status: "exited".into(), health: None })
    } else {
        None
    };
    Snapshot {
        runtime_dir: if windows { "%LOCALAPPDATA%\\siliconmetatrader5\\runtime" } else { "~/siliconmetatrader5/runtime" }.to_owned(),
        provider: mode_label.to_owned(),
        provider_kind: s.mode.clone(),
        docker_context: None,
        global_context: String::new(),
        container,
        status,
        status_error,
        display: "novnc".to_owned(),
        display_port: b.display(),
        display_url: display_url(b.display(), secrets.and_then(|x| x.password.as_deref())),
        bridge_port: b.bridge(),
        status_port: b.status(),
        has_password: secrets.is_some_and(|x| x.password.is_some()),
        has_token: secrets.is_some_and(|x| x.token.is_some()),
        env_exists: secrets.is_some_and(|x| x.token.is_some()),
        image: if windows {
            "MT5 (Windows)"
        } else if native {
            l("Wine (sunucuda)", "Wine (on the server)")
        } else {
            l("siliconmt5 (sunucuda derlendi)", "siliconmt5 (built on the server)")
        }
        .to_owned(),
        lifecycle,
        manages_vm: false,
        mode: if native { "native" } else { "safe" }.to_owned(),
        mode_label: mode_label.to_owned(),
        engine_label: if windows { "Windows" } else if native { "Wine" } else { "Docker" }.to_owned(),
        native,
        native_home: None,
        target: s.id.clone(),
        target_name: s.name.clone(),
        update_needed: secrets.and_then(|x| x.version.as_deref()).is_some_and(|v| v != local_digest),
        linger: secrets.is_some_and(|x| x.linger),
        link: Some(link),
    }
}

/// One card on the Servers page (a server, or "local" = this computer).
#[derive(Debug, Clone, Serialize)]
pub struct ServerCard {
    pub id: String,
    pub name: String,
    pub ssh: String,
    pub mode: String,
    pub port_base: u16,
    /// This computer's port for the bridge (local runtime, or the tunnel end).
    pub bridge_port: u16,
    /// A server's own bridge port (127.0.0.1 on the server); None = local.
    pub remote_bridge_port: Option<u16>,
    pub link: Option<Link>,
    pub lifecycle: String,
    pub phase: Option<String>,
    pub account: Option<smt5_core::api::Account>,
    pub balance: Option<f64>,
    pub equity: Option<f64>,
    pub currency: Option<String>,
    pub today_pl: Option<f64>,
    pub latency_ms: Option<u64>,
    pub update_needed: bool,
    pub linger: bool,
    pub active: bool,
    pub error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn server_card(
    s: &ServerConfig,
    link: Link,
    secrets: Option<&Secrets>,
    status: Option<Result<Status, String>>,
    session: Option<(bool, bool)>,
    history: Option<serde_json::Value>,
    local_digest: &str,
    active: bool,
    latency_ms: Option<u64>,
) -> ServerCard {
    let snap = server_snapshot(s, link, secrets, status, session, local_digest);
    let mut card = card_from(&snap, active);
    card.ssh = s.ssh.clone();
    card.port_base = s.port_base;
    card.remote_bridge_port = Some(secrets.map(|x| x.remote.bridge).unwrap_or(smt5_core::remote::RemotePorts::default().bridge));
    card.latency_ms = latency_ms;
    card.today_pl = history.as_ref().and_then(|h| h.get("trading_pl")).and_then(|v| v.as_f64());
    card
}

/// The local runtime as a card ("Bu bilgisayar").
pub fn local_card(snap: &Snapshot, active: bool) -> ServerCard {
    let mut c = card_from(snap, active);
    c.id = "local".to_owned();
    c.name = l("Bu bilgisayar", "This computer").to_owned();
    c
}

fn card_from(snap: &Snapshot, active: bool) -> ServerCard {
    let account = snap.status.as_ref().and_then(|st| st.bridge.probe.account.clone());
    ServerCard {
        id: snap.target.clone(),
        name: snap.target_name.clone(),
        ssh: String::new(),
        mode: snap.provider_kind.clone(),
        port_base: 0,
        bridge_port: snap.bridge_port,
        remote_bridge_port: None,
        link: snap.link.clone(),
        lifecycle: snap.lifecycle.clone(),
        phase: snap.status.as_ref().map(|st| st.phase.clone()).filter(|p| !p.is_empty()),
        balance: account.as_ref().and_then(|a| a.balance),
        equity: account.as_ref().and_then(|a| a.equity),
        currency: account.as_ref().and_then(|a| a.currency.clone()),
        account,
        today_pl: None,
        latency_ms: None,
        update_needed: snap.update_needed,
        linger: snap.linger,
        active,
        error: snap.status_error.clone(),
    }
}

/// Restart on a server: mt5 / bridge through its status API, container =
/// the whole runtime over ssh.
pub fn server_restart(s: &ServerConfig, config_dir: &Path, api: Result<ApiClient, String>, target: &str) -> Result<Accepted, String> {
    let t = match target {
        "mt5" => RestartTarget::Mt5,
        "bridge" => RestartTarget::Bridge,
        "container" => {
            let mode = ServerMode::parse(&s.mode).ok_or_else(|| format!("unknown server mode {}", s.mode))?;
            let cmd = crate::servers::exec(s, &crate::servers::ssh_opts(config_dir, s), &lifecycle_script(mode, Action::Restart))?;
            let o = smt5_core::Runner::run(&RUNNER, &cmd);
            return if o.ok() { Ok(Accepted { accepted: true, target: "container".into() }) } else { Err(o.stderr.trim().to_owned()) };
        }
        other => return Err(format!("unknown restart target {other}")),
    };
    let api = api?;
    if !api.has_token() {
        return Err(l("Sunucuda API token'ı yok: runtime kurulu mu?", "No API token on the server: is the runtime installed?").into());
    }
    api.restart(t).map_err(|e| e.to_string())
}

/// Jobs on a server: server_install · server_update · start · stop ·
/// restart · server_uninstall. `tarball`/`digest`: the runtime to upload.
/// Windows: the tarball goes up with scp first (no stdin piping through
/// PowerShell), and an update stops the runtime before the folder swap
/// (Windows locks the folder while the supervisor runs).
pub fn server_job_plan(s: &ServerConfig, opts: &SshOpts, kind: &str, tarball: &Path, digest: &str) -> Result<(String, Vec<JobStep>), String> {
    let mode = ServerMode::parse(&s.mode).ok_or_else(|| format!("unknown server mode {}", s.mode))?;
    let win = mode == ServerMode::Windows;
    let t = s.target()?;
    let run = |script: &str| crate::servers::exec(s, opts, script).map(JobStep::Run);
    let life = |a: Action| run(&lifecycle_script(mode, a));
    let upload = || -> Result<Vec<JobStep>, String> {
        let script = upload_script_for(mode, digest);
        Ok(if win {
            vec![JobStep::Run(scp_cmd(&t, opts, tarball, WIN_UPLOAD_NAME)), run(&script)?]
        } else {
            vec![JobStep::Run(crate::servers::exec(s, opts, &script)?.stdin_file(tarball))]
        })
    };
    let (title, steps) = match kind {
        "server_install" => (
            lf!("{} sunucusuna kuruluyor", "Installing on {}", s.name),
            [upload()?, vec![run(&env_init_script(mode))?, life(Action::Install)?, life(Action::Start)?]].concat(),
        ),
        // Windows: [scp, stop, unpack, install, start]
        "server_update" if win => {
            let mut steps = upload()?;
            steps.insert(1, life(Action::Stop)?);
            steps.extend([life(Action::Install)?, life(Action::Start)?]);
            (lf!("{} güncelleniyor", "Updating {}", s.name), steps)
        }
        "server_update" => (
            lf!("{} güncelleniyor", "Updating {}", s.name),
            // Docker: rebuild the image with the new runtime; native: restart picks it up
            [upload()?, vec![life(if mode == ServerMode::Docker { Action::Install } else { Action::Restart })?]].concat(),
        ),
        "start" => (lf!("{} başlatılıyor", "Starting {}", s.name), vec![life(Action::Start)?]),
        // settings "apply" on Windows: install again (idempotent) between stop and
        // start, so a new display port reaches TightVNC and the Display task
        "up" if win => (
            lf!("{} yeni ayarlarla başlatılıyor", "Applying settings on {}", s.name),
            vec![life(Action::Stop)?, life(Action::Install)?, life(Action::Start)?],
        ),
        // settings "apply": compose recreates with the new .env; native needs a restart
        "up" => (
            lf!("{} yeni ayarlarla başlatılıyor", "Applying settings on {}", s.name),
            vec![life(if mode == ServerMode::Docker { Action::Start } else { Action::Restart })?],
        ),
        // Windows has no image: a restart
        "build" if win => (lf!("{} yeniden başlatılıyor", "Restarting {}", s.name), vec![life(Action::Restart)?]),
        "build" => (lf!("{} imajı derleniyor", "Building the image on {}", s.name), vec![life(Action::Install)?]),
        "stop" => (lf!("{} durduruluyor", "Stopping {}", s.name), vec![life(Action::Stop)?]),
        "restart" => (lf!("{} yeniden başlatılıyor", "Restarting {}", s.name), vec![life(Action::Restart)?]),
        "server_reset" => (lf!("{} sunucusunda MT5 verisi sıfırlanıyor", "Resetting MT5 data on {}", s.name), vec![life(Action::Reset)?]),
        "server_uninstall" => (lf!("{} sunucusundan kaldırılıyor", "Removing from {}", s.name), vec![life(Action::Uninstall)?]),
        other => return Err(format!("unknown server job {other}")),
    };
    Ok((title, steps))
}

// ------------------------------------------------------- setup inventory ----

/// One prerequisite of the chosen mode, for the wizard's "what is installed,
/// what will be installed" list.
#[derive(Debug, Clone, Serialize)]
pub struct Component {
    pub id: String,
    pub name: String,
    /// `ok` installed · `app` the app installs it · `user` the user must act (see `fix`).
    pub state: String,
    pub detail: String,
    pub fix: Option<String>,
}

fn comp(id: &str, name: &str, state: &str, detail: impl Into<String>, fix: Option<String>) -> Component {
    Component { id: id.into(), name: name.into(), state: state.into(), detail: detail.into(), fix }
}

/// Homebrew formulae the Güvenli mode needs (lima comes with colima).
pub const SAFE_FORMULAE: &[&str] = &["colima", "docker", "docker-compose", "qemu", "lima-additional-guestagents"];

pub fn components(rt: &RuntimeDir) -> Vec<Component> {
    let c = ctx(rt);
    let run = |cmd: Cmd| smt5_core::Runner::run(&RUNNER, &cmd);
    let mut out = Vec::new();
    let env_ok = rt.env_path().exists() && c.env.get_nonempty("SMT5_API_TOKEN").is_some();
    if c.provider.kind.is_native() {
        let paths = NativePaths::current();
        let f = native::facts(&RUNNER, &paths);
        let mac = cfg!(target_os = "macos");
        if mac {
            let rosetta = std::env::consts::ARCH == "x86_64" || run(Cmd::new("arch").args(["-x86_64", "/usr/bin/true"])).ok();
            out.push(if rosetta {
                comp("rosetta", "Rosetta 2", "ok", l("kurulu", "installed"), None)
            } else {
                comp("rosetta", "Rosetta 2", "user", l("x86_64 Wine için gerekli", "needed by x86_64 Wine"), Some("softwareupdate --install-rosetta --agree-to-license".into()))
            });
        }
        let py = if f.venv {
            true
        } else {
            // Debian/Ubuntu: venv works only with python3-venv (ensurepip)
            let o = run(Cmd::new("python3").args(["-c", "import sys, venv, ensurepip; sys.exit(0 if sys.version_info >= (3, 9) else 1)"]));
            o.ok() && (!mac || run(Cmd::new("xcode-select").arg("-p")).ok())
        };
        out.push(if py {
            comp("python", "Python 3.9+", "ok", l("durum API'si için", "for the status API"), None)
        } else if mac {
            comp("python", "Python 3.9+", "user", l("Apple'ın komut satırı araçlarıyla gelir", "comes with Apple's command line tools"), Some("xcode-select --install".into()))
        } else {
            comp("python", "Python 3.9+ (venv)", "user", l("durum API'si için", "for the status API"), Some("sudo apt install python3 python3-venv".into()))
        });
        if !mac {
            let desktop = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
            let has = |t: &str| run(Cmd::new("sh").args(["-c", &format!("command -v {t}")])).ok();
            let missing: Vec<&str> = ["Xvfb", "x11vnc", "websockify"].into_iter().filter(|t| !has(t)).collect();
            out.push(if desktop {
                comp("display", l("Ekran", "Display"), "ok", l("masaüstü oturumu: MT5 normal bir pencerede açılır", "desktop session: MT5 opens as a normal window"), None)
            } else if missing.is_empty() {
                comp("display", l("Ekran (noVNC)", "Display (noVNC)"), "ok", l("ekransız: Xvfb + noVNC, yalnız 127.0.0.1", "headless: Xvfb + noVNC, 127.0.0.1 only"), None)
            } else {
                comp("display", l("Ekran (noVNC)", "Display (noVNC)"), "user", lf!("ekran yok, eksik: {}", "no display, missing: {}", missing.join(", ")), Some("sudo apt install xvfb x11vnc websockify novnc openbox".into()))
            });
        }
        let app = |ok: bool| if ok { "ok" } else { "app" };
        let wine_dl = if mac { l("~215 MB indirilecek (Gcenx macOS derlemesi)", "~215 MB download (Gcenx macOS build)") } else { l("~100 MB indirilecek (Kron4ek Linux derlemesi)", "~100 MB download (Kron4ek Linux build)") };
        out.push(comp("wine", "Wine 11", app(f.wine.is_some()), if f.wine.is_some() { l("indirildi", "downloaded") } else { wine_dl }, None));
        out.push(comp("winpython", "Windows Python + MetaTrader5", app(f.windows_python), if f.windows_python { l("kurulu", "installed") } else { l("Wine içine kurulacak", "installed into Wine") }, None));
        let mt5_from = if f.mt5 {
            l("kurulu", "installed")
        } else if f.probe_prefix || {
            let mut d = Cmd::new("docker");
            if mac {
                d = d.args(["--context", "colima-x86"]);
            }
            run(d.args(["inspect", "-f", "{{.Id}}", smt5_core::CONTAINER])).ok()
        } {
            if mac { l("Güvenli moddan kopyalanacak (broker girişi dahil)", "copied from the Safe mode (broker login included)") } else { l("Docker modundan kopyalanacak (broker girişi dahil)", "copied from the Docker mode (broker login included)") }
        } else {
            l("MetaQuotes kurulum programıyla kurulacak", "installed with the MetaQuotes installer")
        };
        out.push(comp("mt5", "MetaTrader 5", app(f.mt5), mt5_from, None));
        out.push(comp("venv", l("Durum API'si", "Status API"), app(f.venv), if f.venv { l("hazır", "ready") } else { l("küçük bir Python ortamı oluşturulacak", "a small Python environment is created") }, None));
        out.push(comp("env", l("Yapılandırma", "Configuration"), app(env_ok), if env_ok { rt.env_path().display().to_string() } else { l("API token üretilecek", "an API token is generated").to_owned() }, None));
        return out;
    }

    let mac = cfg!(target_os = "macos");
    let d = c.docker();
    if mac {
        let brew = run(Cmd::new("brew").arg("--version")).ok();
        if !brew {
            out.push(comp("brew", "Homebrew", "user", l("Güvenli modun paketleri için", "for the Safe mode's packages"), Some("/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"".into())));
        }
        let listed = if brew { run(Cmd::new("brew").args(["list", "--formula", "-1"])).stdout } else { String::new() };
        let installed: Vec<&str> = listed.lines().map(str::trim).collect();
        let missing: Vec<&str> = SAFE_FORMULAE.iter().copied().filter(|f| !installed.contains(f)).collect();
        let docker_ok = d.client_version().is_some();
        let compose_ok = docker_ok && run(Cmd::new("docker").args(["compose", "version"])).ok();
        let st = if missing.is_empty() && docker_ok && compose_ok {
            "ok"
        } else if brew {
            "app"
        } else {
            "user"
        };
        let detail = if missing.is_empty() { SAFE_FORMULAE.join(", ") } else { lf!("eksik: {}", "missing: {}", missing.join(", ")) };
        out.push(comp("packages", l("Docker, Colima, QEMU (Homebrew)", "Docker, Colima, QEMU (Homebrew)"), st, detail, (!missing.is_empty()).then(|| format!("brew install {}", missing.join(" ")))));
        if missing.is_empty() && docker_ok && !compose_ok {
            out.push(comp("compose", "docker compose", "user", l("eklenti bağlı değil", "the plugin is not linked"), Some("mkdir -p ~/.docker/cli-plugins && ln -sfn \"$(brew --prefix)/opt/docker-compose/bin/docker-compose\" ~/.docker/cli-plugins/docker-compose".into())));
        }
        let vm = c.provider.vm_profile(&RUNNER);
        out.push(match &vm {
            Some(v) if v.running() => comp("vm", l("x86_64 sanal makine", "x86_64 virtual machine"), "ok", lf!("'{}' çalışıyor", "'{}' running", v.name), None),
            Some(v) => comp("vm", l("x86_64 sanal makine", "x86_64 virtual machine"), "app", lf!("'{}' kapalı, açılacak", "'{}' is off, will be started", v.name), None),
            None => comp("vm", l("x86_64 sanal makine", "x86_64 virtual machine"), "app", l("oluşturulacak: 4 CPU, 8 GB RAM, 60 GB disk (~5–10 dk)", "created: 4 CPU, 8 GB RAM, 60 GB disk (~5–10 min)"), None),
        });
    } else {
        let docker_ok = d.client_version().is_some();
        out.push(if docker_ok {
            comp("docker", "Docker Engine", "ok", l("kurulu", "installed"), None)
        } else {
            comp("docker", "Docker Engine", "user", l("Linux'ta sistem paketi", "a system package on Linux"), Some("curl -fsSL https://get.docker.com | sh && sudo usermod -aG docker $USER".into()))
        });
        if docker_ok && !d.engine_up() {
            out.push(comp("engine", l("Docker servisi", "Docker service"), "user", l("çalışmıyor ya da yetki yok", "not running or no permission"), Some("sudo systemctl enable --now docker && sudo usermod -aG docker $USER   # then log out and in".into())));
        }
    }
    let engine_up = d.engine_up();
    let image = engine_up && d.image_present(&image_name(&c.env));
    let how = if rt.packaged() { l("indirilecek (GHCR)", "pulled from GHCR") } else { l("derlenecek: emülasyon altında 30–90 dk", "built: 30–90 min under emulation") };
    out.push(comp("image", l("Runtime imajı", "Runtime image"), if image { "ok" } else { "app" }, if image { l("yerelde mevcut", "present") } else { how }, None));
    out.push(comp("env", l("Yapılandırma", "Configuration"), if env_ok { "ok" } else { "app" }, if env_ok { rt.env_path().display().to_string() } else { l("masaüstü şifresi ve API token üretilecek", "a desktop password and an API token are generated").to_owned() }, None));
    out
}

// --------------------------------------------------------------- jobs ----

/// One step of a long job.
#[derive(Debug, Clone, PartialEq)]
pub enum JobStep {
    /// Stop the job if it fails.
    Run(Cmd),
    /// Log a failure and continue (best-effort cleanup).
    Try(Cmd),
    /// Delete a file the app owns (`.env`, app config); missing is fine.
    RemoveFile(PathBuf),
    /// If the global docker context is `ctx` (just deleted), switch it to `default`.
    ResetGlobalContext(String),
}

/// Options of the reset / uninstall jobs (from the confirmation dialog).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct JobOpts {
    /// Also delete runtime/.env (desktop password, API token, settings).
    pub reset_env: bool,
    /// Also `brew uninstall` the packages SiliconMetaTrader5 needed (only if safe).
    pub remove_packages: bool,
    /// Server mode: run on "local" or this server id instead of the active target.
    pub target: Option<String>,
}

/// Homebrew formulae the setup wizard asks for (removed only on request).
pub const BREW_FORMULAE: &[&str] = &["colima", "lima-additional-guestagents", "lima", "qemu", "docker-compose", "docker"];

/// What the reset / uninstall screens show before anything is deleted, and
/// what the jobs re-check before they run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RemovalInfo {
    pub manages_vm: bool,
    pub profile: Option<ColimaProfile>,
    /// Colima profiles of other projects: never touched; they block package removal.
    pub other_profiles: Vec<String>,
    /// Containers in the SiliconMetaTrader5 engine that are not ours (deleted with the VM!).
    pub other_containers: Vec<String>,
    pub engine_up: bool,
    pub volumes: Vec<String>,
    pub images: Vec<String>,
    pub env_path: String,
    pub env_exists: bool,
    pub config_path: String,
    pub brew_formulae: Vec<String>,
    pub packages_allowed: bool,
    pub packages_reason: Option<String>,
    pub docker_desktop: bool,
    pub colima_delete_data: bool,
    /// Commands the user runs for anything the app will not remove itself.
    pub manual_steps: Vec<String>,
    /// The Mac yerel mode is selected (levels 1–2 act on it, not on the VM).
    pub native: bool,
    /// ~/Library/Application Support/SiliconMT5 and its size (Wine, prefix, venv).
    pub native_home: Option<String>,
    pub native_exists: bool,
    pub native_bytes: Option<u64>,
    /// Installed by Homebrew / a Linux package (no repository folder to keep).
    pub packaged: bool,
}

fn dir_bytes(path: &Path) -> Option<u64> {
    let o = smt5_core::Runner::run(&RUNNER, &Cmd::new("du").args(["-sk".to_owned(), path.display().to_string()]));
    o.stdout.split_whitespace().next()?.parse::<u64>().ok().map(|k| k * 1024)
}

/// Provider of the docker side: in the native mode, the Güvenli mode's VM
/// (the uninstall removes it too).
fn docker_provider(c: &Context) -> Provider {
    if c.provider.kind.is_native() {
        Provider::safe(&c.env, &c.host)
    } else {
        c.provider.clone()
    }
}

pub fn removal_info(rt: &RuntimeDir, config_path: &Path) -> RemovalInfo {
    let c = ctx(rt);
    let native = c.provider.kind.is_native();
    let provider = docker_provider(&c);
    let d = smt5_core::Docker::new(&RUNNER, &provider, rt);
    let manages_vm = provider.kind.uses_colima();
    let profile = provider.vm_profile(&RUNNER);
    // In the native mode, ask the VM's engine only if the VM runs (no hang on a dead context).
    let engine_up = if native { profile.as_ref().map(|p| p.running()).unwrap_or(false) && d.engine_up() } else { d.engine_up() };
    let other_profiles: Vec<String> = provider.other_vm_profiles(&RUNNER).into_iter().map(|p| p.name).collect();
    let paths = NativePaths::current();
    let native_exists = paths.base.is_dir();
    let docker_desktop = cfg!(target_os = "macos") && Path::new("/Applications/Docker.app").exists();
    let brew_formulae: Vec<String> = if cfg!(target_os = "macos") {
        let o = smt5_core::Runner::run(&RUNNER, &Cmd::new("brew").args(["list", "--formula", "-1"]));
        let installed: Vec<&str> = o.stdout.lines().map(str::trim).collect();
        BREW_FORMULAE
            .iter()
            .filter(|f| installed.contains(f))
            .filter(|f| !(docker_desktop && f.starts_with("docker")))
            .map(|f| (*f).to_owned())
            .collect()
    } else {
        vec![]
    };
    let packages_reason = if !cfg!(target_os = "macos") {
        Some(l("Linux'ta Docker Engine sistem paketidir; uygulama kaldırmaz", "On Linux Docker Engine is a system package; the app does not remove it").to_owned())
    } else if !other_profiles.is_empty() {
        Some(lf!("Başka Colima profilleri var ({}): colima ve docker başka projelerce kullanılıyor", "Other Colima profiles exist ({}): colima and docker are used by other projects", other_profiles.join(", ")))
    } else if brew_formulae.is_empty() {
        Some(l("Homebrew ile kurulmuş ilgili paket bulunamadı", "No related Homebrew package is installed").to_owned())
    } else {
        None
    };
    let help = if manages_vm { smt5_core::Runner::run(&RUNNER, &Cmd::new("colima").args(["delete", "--help"])) } else { Default::default() };
    let mut manual_steps = vec![];
    if rt.packaged() && cfg!(target_os = "macos") {
        manual_steps.push(l("Uygulamayı ve smt5 komutunu kaldırın: brew uninstall siliconmt5 (Uygulamalar'daki kopyayı da silin)", "Remove the app and the smt5 command: brew uninstall siliconmt5 (also delete the copy in Applications)").to_owned());
    } else if rt.packaged() {
        manual_steps.push(l("Uygulamayı kaldırın: sudo apt remove siliconmt5 ya da AppImage dosyasını silin", "Remove the app: sudo apt remove siliconmt5, or delete the AppImage").to_owned());
    } else {
        if cfg!(target_os = "macos") {
            manual_steps.push(l("SiliconMetaTrader5 uygulamasını Uygulamalar klasöründen Çöp Sepeti'ne taşıyın", "Move SiliconMetaTrader5 from Applications to the Trash").to_owned());
        }
        manual_steps.push(lf!("Repo klasörünü isterseniz silin: {}", "Delete the repository folder if you like: {}", rt.root.parent().unwrap_or(&rt.root).display()));
    }
    if !cfg!(target_os = "macos") {
        manual_steps.push(l("Docker'ı da kaldırmak isterseniz dağıtımınızın paket yöneticisini kullanın", "To remove Docker too, use your distribution's package manager").to_owned());
    }
    RemovalInfo {
        manages_vm,
        profile,
        other_containers: if engine_up { d.other_containers() } else { vec![] },
        volumes: if engine_up { d.project_volumes() } else { vec![] },
        images: if engine_up { d.runtime_images() } else { vec![] },
        engine_up,
        env_path: rt.env_path().display().to_string(),
        env_exists: rt.env_path().exists(),
        config_path: config_path.display().to_string(),
        packages_allowed: packages_reason.is_none(),
        packages_reason,
        brew_formulae,
        docker_desktop,
        colima_delete_data: help.stdout.contains("--data") || help.stderr.contains("--data"),
        other_profiles,
        manual_steps,
        native,
        native_home: Some(paths.base.display().to_string()),
        native_bytes: if native_exists { dir_bytes(&paths.base) } else { None },
        native_exists,
        packaged: rt.packaged(),
    }
}

/// Steps of a long job (`kind`), in order, plus a title for the UI.
pub fn job_plan(rt: &RuntimeDir, kind: &str, opts: &JobOpts, config_path: &Path) -> Result<(String, Vec<JobStep>), String> {
    use JobStep::*;
    let c = ctx(rt);
    if c.provider.kind.is_native() {
        return native_job_plan(rt, &c, kind, opts, config_path);
    }
    let d = c.docker();
    let vm = |cmd: Option<Cmd>| cmd.ok_or_else(|| lf!("{} için VM yönetimi yok", "{} has no VM to manage", c.provider.kind.label()));
    Ok(match kind {
        "vm_start" => (l("x86_64 VM başlatılıyor", "Starting the x86_64 VM").into(), vec![Run(vm(c.provider.vm_start_cmd(VmSize::default()))?)]),
        "vm_stop" => (l("x86_64 VM durduruluyor", "Stopping the x86_64 VM").into(), vec![Run(vm(c.provider.vm_stop_cmd())?)]),
        // A packaged app pulls the published image of its own version instead
        // of building 30–90 minutes (a checkout builds what it has).
        "build" | "install" if rt.packaged() => (l("İmaj indiriliyor", "Pulling the image").into(), vec![Run(d.compose(["pull"]))]),
        "build" | "install" => (l("İmaj derleniyor", "Building the image").into(), vec![Run(d.compose(["build"]))]),
        "pull" => (l("İmaj indiriliyor", "Pulling the image").into(), vec![Run(d.compose(["pull"]))]),
        // Start from any state: VM first if it is off (Colima), then compose up
        // (creates the container if it was removed, keeps the volumes).
        "start" | "up" => {
            let mut steps = Vec::new();
            if c.provider.kind.manages_engine() && !d.engine_up() {
                steps.push(Run(vm(c.provider.vm_start_cmd(VmSize::default()))?));
            }
            steps.push(Run(d.compose(["up", "-d"])));
            (l("Runtime başlatılıyor", "Starting the runtime").into(), steps)
        }
        // Stop keeps the container; `start` resumes it in seconds.
        "stop" => (l("Runtime durduruluyor", "Stopping the runtime").into(), vec![Run(d.compose(["stop"]))]),
        "down" => (l("Container kaldırılıyor (veriler korunur)", "Removing the container (data is kept)").into(), vec![Run(d.compose(["down"]))]),
        // Level 1: MT5 data only — image, VM and settings stay.
        "reset" => {
            if !d.engine_up() {
                return Err(l("Docker engine kapalı: önce runtime'ı başlatın (Başlat VM'i de açar)", "The Docker engine is off: start the runtime first (Start also starts the VM)").into());
            }
            (l("Runtime sıfırlanıyor (MT5 verisi siliniyor)", "Resetting the runtime (deleting MT5 data)").into(), vec![Run(d.compose(["down", "-v", "--remove-orphans"]))])
        }
        // Level 2: delete our Colima VM (images, volumes, build cache inside it)
        // and start the setup wizard again. Other profiles are never touched.
        "reset_vm" => {
            let info = removal_info(rt, config_path);
            let mut steps = Vec::new();
            if info.engine_up {
                steps.push(Try(d.compose(["down", "-v", "--remove-orphans"])));
            }
            steps.push(Run(vm(c.provider.vm_delete_cmd(info.colima_delete_data))?));
            if let Some(ctx_name) = &c.provider.docker_context {
                steps.push(ResetGlobalContext(ctx_name.clone()));
            }
            if opts.reset_env {
                steps.push(RemoveFile(rt.env_path()));
            }
            (l("x86_64 VM siliniyor", "Deleting the x86_64 VM").into(), steps)
        }
        // Wizard: everything the app can install for this mode, in order.
        "prepare" => {
            let mut steps = Vec::new();
            let comps = components(rt);
            if let Some(p) = comps.iter().find(|x| x.id == "packages" && x.state == "app") {
                let missing: Vec<&str> = p.fix.as_deref().and_then(|f| f.strip_prefix("brew install ")).map(|m| m.split(' ').collect()).unwrap_or_default();
                if !missing.is_empty() {
                    steps.push(Run(Cmd::new("brew").arg("install").args(missing)));
                }
            }
            if c.provider.kind.manages_engine() && !d.engine_up() {
                steps.push(Run(vm(c.provider.vm_start_cmd(VmSize::default()))?));
            }
            if comps.iter().any(|x| x.id == "image" && x.state == "app") {
                steps.push(Run(d.compose([if rt.packaged() { "pull" } else { "build" }])));
            }
            (l("Kurulum hazırlanıyor", "Preparing the installation").into(), steps)
        }
        "uninstall" => (l("SiliconMetaTrader5 kaldırılıyor", "Uninstalling SiliconMetaTrader5").into(), uninstall_steps(rt, &c, opts, config_path)?),
        other => return Err(format!("unknown job {other}")),
    })
}

/// Jobs of the Mac yerel mode: everything goes through `native.sh`.
fn native_job_plan(rt: &RuntimeDir, c: &Context, kind: &str, opts: &JobOpts, config_path: &Path) -> Result<(String, Vec<JobStep>), String> {
    use JobStep::*;
    let n = |sub: &str| native::cmd(rt, sub);
    Ok(match kind {
        "vm_start" | "vm_stop" => return Err(l("Yerel modda VM yok", "The native mode has no VM").into()),
        "build" | "pull" | "install" | "prepare" => (l("Yerel mod kuruluyor (Wine 11, Windows Python, MT5)", "Installing the native mode (Wine 11, Windows Python, MT5)").into(), vec![Run(n("install"))]),
        "start" | "up" => (l("Runtime başlatılıyor", "Starting the runtime").into(), vec![Run(n("start"))]),
        "stop" | "down" => (l("Runtime durduruluyor", "Stopping the runtime").into(), vec![Run(n("stop"))]),
        "import" => (l("MT5, Güvenli moddan kopyalanıyor", "Copying MT5 from the Safe mode").into(), vec![Run(n("import"))]),
        // Level 1: the Wine prefix (MT5, login, history); Wine and the venv stay.
        "reset" => (l("Runtime sıfırlanıyor (MT5 verisi siliniyor)", "Resetting the runtime (deleting MT5 data)").into(), vec![Run(n("reset"))]),
        // Level 2: every file of the native mode (Wine download, venv too).
        "reset_vm" => {
            let mut steps = vec![Run(n("reset-all"))];
            if opts.reset_env {
                steps.push(RemoveFile(rt.env_path()));
            }
            (l("Yerel modun dosyaları siliniyor", "Deleting the native mode's files").into(), steps)
        }
        "uninstall" => (l("SiliconMetaTrader5 kaldırılıyor", "Uninstalling SiliconMetaTrader5").into(), uninstall_steps(rt, c, opts, config_path)?),
        other => return Err(format!("unknown job {other}")),
    })
}

/// Level 3: everything SiliconMetaTrader5 created on this computer — the native
/// mode's folder, our Colima VM (or, on Linux, our container, volumes and
/// images), `.env`, the app config, and on request the Homebrew packages.
fn uninstall_steps(rt: &RuntimeDir, c: &Context, opts: &JobOpts, config_path: &Path) -> Result<Vec<JobStep>, String> {
    use JobStep::*;
    let info = removal_info(rt, config_path);
    if opts.remove_packages && !info.packages_allowed {
        return Err(info.packages_reason.unwrap_or_else(|| l("paketler kaldırılamaz", "the packages cannot be removed").into()));
    }
    let provider = docker_provider(c);
    let d = smt5_core::Docker::new(&RUNNER, &provider, rt);
    let mut steps = Vec::new();
    if info.native_exists || native::agent_loaded(&RUNNER) {
        steps.push(Run(native::cmd(rt, "uninstall")));
    }
    if provider.kind.uses_colima() {
        if info.engine_up {
            steps.push(Try(d.compose(["down", "-v", "--remove-orphans"])));
        }
        if info.profile.is_some() {
            let del = provider.vm_delete_cmd(info.colima_delete_data).ok_or_else(|| l("Colima profili silinemez", "the Colima profile cannot be deleted").to_owned())?;
            steps.push(Run(del));
        }
        if let Some(ctx_name) = &provider.docker_context {
            steps.push(ResetGlobalContext(ctx_name.clone()));
        }
    } else if info.engine_up {
        // Linux: the engine is shared, so remove only our container, volumes and images.
        steps.push(Run(d.compose(["down", "-v", "--rmi", "all", "--remove-orphans"])));
    }
    steps.push(RemoveFile(rt.env_path()));
    steps.push(RemoveFile(config_path.to_path_buf()));
    if opts.remove_packages {
        steps.push(Run(Cmd::new("brew").arg("uninstall").args(info.brew_formulae.clone())));
    }
    Ok(steps)
}

// ----------------------------------------------------------- clipboard ----

/// The X display of the noVNC target (entrypoint-novnc.sh).
const X_DISPLAY: &str = ":100";
pub const CLIP_MAX: usize = 1 << 20;

/// Put `text` on the container's X CLIPBOARD (Wine pastes it on Ctrl+V).
/// xclip forks to keep owning the selection; its output is detached so
/// `docker exec` returns at once.
pub fn mt5_clipboard_set(rt: &RuntimeDir, text: &str) -> Result<(), String> {
    use std::io::Write;
    let c = ctx(rt);
    let cmd = c.docker().cmd([
        "exec".to_owned(),
        "-i".to_owned(),
        smt5_core::CONTAINER.to_owned(),
        "sh".to_owned(),
        "-c".to_owned(),
        format!("DISPLAY={X_DISPLAY} xclip -selection clipboard -i >/dev/null 2>&1"),
    ]);
    let mut child = smt5_core::exec::build(&cmd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => Ok(()),
        Some(127) => Err(l("xclip yok: imajı yenileyin (smt5 build && smt5 up)", "xclip is missing: rebuild the image (smt5 build && smt5 up)").into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
    }
}

/// Text on the container's X CLIPBOARD (None when empty or not text).
/// A server clipboard call that failed, as the screen page shows it.
pub fn server_clipboard_error(e: smt5_core::api::ApiError) -> String {
    use smt5_core::api::ApiError;
    match e {
        ApiError::Http { status: 404, .. } => {
            l("Sunucudaki runtime pano eşitlemeyi bilmiyor; Sunucular sayfasından Güncelle.", "The server's runtime has no clipboard sync yet; Update it on the Servers page.").into()
        }
        ApiError::Unreachable(_) => l("Sunucudaki MT5'e ulaşılamıyor (runtime kapalı ya da Windows oturumu açık değil).", "Cannot reach MT5 on the server (runtime stopped or no Windows session).").into(),
        ApiError::Http { message, .. } => message,
        other => other.to_string(),
    }
}

pub fn mt5_clipboard_get(rt: &RuntimeDir) -> Result<Option<String>, String> {
    let c = ctx(rt);
    let o = smt5_core::Runner::run(
        &RUNNER,
        &c.docker().cmd([
            "exec".to_owned(),
            smt5_core::CONTAINER.to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            format!("command -v xclip >/dev/null || exit 127; DISPLAY={X_DISPLAY} xclip -selection clipboard -o -t UTF8_STRING 2>/dev/null || DISPLAY={X_DISPLAY} xclip -selection clipboard -o 2>/dev/null || true"),
        ]),
    );
    match o.status {
        0 => Ok((!o.stdout.is_empty() && o.stdout.len() <= CLIP_MAX).then_some(o.stdout)),
        127 => Err(l("xclip yok: imajı yenileyin (smt5 build && smt5 up)", "xclip is missing: rebuild the image (smt5 build && smt5 up)").into()),
        _ => Err(o.stderr.trim().to_owned()),
    }
}

/// Decide what to copy where. `last_*` are what each side held after the
/// previous sync, so a change on one side wins and nothing echoes back.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ClipSync {
    pub last_host: Option<String>,
    pub last_mt5: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum ClipAction {
    None,
    ToMt5(String),
    ToHost(String),
}

impl ClipSync {
    pub fn decide(&mut self, host: Option<String>, mt5: Option<String>) -> ClipAction {
        if host.is_some() && host != self.last_host {
            self.last_host = host.clone();
            self.last_mt5 = host.clone();
            return ClipAction::ToMt5(host.unwrap_or_default());
        }
        if mt5.is_some() && mt5 != self.last_mt5 {
            self.last_mt5 = mt5.clone();
            self.last_host = mt5.clone();
            return ClipAction::ToHost(mt5.unwrap_or_default());
        }
        ClipAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Server mode (Task 6)
    #[test]
    fn remote_env_update_rejects_bad_values() {
        let one = |k: &str, v: &str| {
            let mut m = BTreeMap::new();
            m.insert(k.to_owned(), v.to_owned());
            remote_env_update_script(&m)
        };
        assert!(one("SMT5_MAXBARS", "1;rm").is_err());
        assert!(one("SMT5_RESOLUTION", "1500x770\nX=1").is_err());
        assert!(one("SMT5_API_TOKEN", "abc").is_err(), "secrets are not editable remotely");
        assert!(one("SMT5_WEB_PASSWORD", "abc").is_err());
        assert!(one("SMT5_NOT_A_KEY", "1").is_err());
        // noVNC text size is a setting too (72–288 DPI)
        assert!(one("SMT5_DPI", "120").unwrap().contains("SMT5_DPI=120"));
        assert!(one("SMT5_DPI", "500").is_err() && one("SMT5_DPI", "abc").is_err());
        let ok = one("SMT5_RESOLUTION", "1500x770").unwrap();
        assert!(ok.contains("SMT5_RESOLUTION=1500x770") && ok.contains("siliconmetatrader5/runtime/.env"), "{ok}");
        let o = std::process::Command::new("bash").args(["-n", "-c", &ok]).output().unwrap();
        assert!(o.status.success());
        // applied to a fake server home: replaces an existing key, appends a new one
        let home = std::env::temp_dir().join(format!("smt5-renv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("siliconmetatrader5/runtime")).unwrap();
        std::fs::write(home.join("siliconmetatrader5/runtime/.env"), "SMT5_RESOLUTION=1366x768\nSMT5_API_TOKEN=t\n").unwrap();
        let mut m = BTreeMap::new();
        m.insert("SMT5_RESOLUTION".to_owned(), "1500x770".to_owned());
        m.insert("SMT5_MAXBARS".to_owned(), "200000".to_owned());
        let sc = remote_env_update_script(&m).unwrap();
        let st = std::process::Command::new("bash").args(["-c", &sc]).env("HOME", &home).status().unwrap();
        assert!(st.success());
        assert_eq!(std::fs::read_to_string(home.join("siliconmetatrader5/runtime/.env")).unwrap(), "SMT5_RESOLUTION=1500x770\nSMT5_API_TOKEN=t\nSMT5_MAXBARS=200000\n");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn server_card_marks_active_and_update() {
        let st: Status = serde_json::from_value(serde_json::json!({
            "phase": "ready",
            "bridge": {"probe": {"account": {"login_masked": "•••409", "server": "FTMO-Demo", "trade_mode": "demo", "balance": 25000.0, "equity": 25010.5, "currency": "USD"}}}
        }))
        .unwrap();
        let sec = Secrets { version: Some("old".into()), token: Some("t".into()), linger: true, ..Default::default() };
        let hist = serde_json::json!({"trading_pl": 12.5});
        let c = server_card(&srv(18000), Link::Connected { since: 1.0 }, Some(&sec), Some(Ok(st)), None, Some(hist), "new", true, Some(42));
        assert!(c.active && c.update_needed && c.linger);
        assert_eq!((c.phase.as_deref(), c.lifecycle.as_str(), c.latency_ms), (Some("ready"), "ready", Some(42)));
        assert_eq!((c.balance, c.equity, c.currency.as_deref(), c.today_pl), (Some(25000.0), Some(25010.5), Some("USD"), Some(12.5)));
        assert_eq!(c.account.as_ref().and_then(|a| a.server.as_deref()), Some("FTMO-Demo"));
        assert_eq!((c.bridge_port, c.port_base, c.remote_bridge_port), (18001, 18000, Some(8001)));
        let mut sec2 = sec.clone();
        sec2.remote.bridge = 9001;
        let c2 = server_card(&srv(18000), Link::Connected { since: 1.0 }, Some(&sec2), None, None, None, "new", false, None);
        assert_eq!(c2.remote_bridge_port, Some(9001), "the server's own port comes from its .env");
        let down = server_card(&srv(18000), Link::Failed { kind: "key_required".into(), detail: "".into(), at: 1.0 }, None, None, None, None, "new", false, None);
        assert_eq!((down.lifecycle.as_str(), down.active, down.update_needed, down.account.is_none()), ("unreachable", false, false, true));
    }

    // ---- Server mode (Task 5)
    use crate::servers::ServerConfig;
    use crate::targets::{Link, Secrets};

    fn srv(base: u16) -> ServerConfig {
        ServerConfig { id: "a1b2c3".into(), name: "VPS".into(), ssh: "u@h".into(), mode: "linux-native".into(), port_base: base, ..Default::default() }
    }
    fn up() -> Link {
        Link::Connected { since: 1.0 }
    }

    #[test]
    fn server_snapshot_not_installed() {
        let sec = Secrets { version: None, ..Default::default() };
        assert!(!server_wants_status(&up(), Some(&sec)), "no API call before install");
        let snap = server_snapshot(&srv(18000), up(), Some(&sec), None, None, "d");
        assert_eq!(snap.lifecycle, "not_installed");
        assert!(snap.status.is_none() && snap.status_error.is_none());
        assert_eq!((snap.target.as_str(), snap.target_name.as_str()), ("a1b2c3", "VPS"));
        let half = Secrets { version: Some("d".into()), token: None, ..Default::default() };
        assert!(!server_wants_status(&up(), Some(&half)));
        // uploaded but no .env yet: offer the install again, no endless spinner (Final review m2)
        assert_eq!(server_snapshot(&srv(18000), up(), Some(&half), None, None, "d").lifecycle, "not_installed");
    }

    #[test]
    fn server_snapshot_ports_and_display() {
        let sec = Secrets { version: Some("old".into()), token: Some("t".into()), password: Some("p w".into()), linger: true, ..Default::default() };
        assert!(server_wants_status(&up(), Some(&sec)));
        let snap = server_snapshot(&srv(18100), up(), Some(&sec), Some(Err("down".into())), None, "new");
        assert_eq!((snap.bridge_port, snap.status_port, snap.display_port), (18101, 18102, 18181));
        assert!(snap.display_url.starts_with("http://127.0.0.1:18181/vnc.html") && snap.display_url.ends_with("#password=p%20w"), "{}", snap.display_url);
        assert_eq!((snap.display.as_str(), snap.native, snap.provider_kind.as_str()), ("novnc", true, "linux-native"));
        assert_eq!(snap.lifecycle, "stopped");
        assert_eq!(snap.status_error.as_deref(), Some("down"));
        assert!(snap.update_needed && snap.linger && snap.has_token && snap.has_password);
        let docker = ServerConfig { mode: "docker".into(), ..srv(18100) };
        let s2 = server_snapshot(&docker, up(), Some(&sec), Some(Err("x".into())), None, "old");
        assert!(!s2.native && !s2.update_needed);
        // the UI's "running" checks read a pseudo container, like native mode
        assert_eq!(snap.container.as_ref().map(|c| c.status.as_str()), Some("exited"));
        let st: Status = serde_json::from_value(serde_json::json!({"phase": "waiting_login"})).unwrap();
        let on = server_snapshot(&srv(18100), up(), Some(&sec), Some(Ok(st)), None, "new");
        assert_eq!((on.lifecycle.as_str(), on.container.as_ref().map(|c| c.status.as_str())), ("waiting_login", Some("running")));
        assert!(server_snapshot(&srv(18100), up(), Some(&Secrets::default()), None, None, "new").container.is_none());
    }

    #[test]
    fn server_snapshot_unreachable() {
        let failed = Link::Failed { kind: "network".into(), detail: "timeout".into(), at: 1.0 };
        let sec = Secrets { version: Some("d".into()), token: Some("t".into()), ..Default::default() };
        assert!(!server_wants_status(&failed, Some(&sec)));
        let snap = server_snapshot(&srv(18000), failed.clone(), Some(&sec), None, None, "d");
        assert_eq!(snap.lifecycle, "unreachable");
        assert_eq!(snap.link, Some(failed));
        assert_eq!(server_snapshot(&srv(18000), Link::Connecting, None, None, None, "d").lifecycle, "connecting");
    }

    #[test]
    fn server_job_plan_install_steps() {
        let opts = smt5_core::remote::SshOpts { dir: "/c/ssh".into(), control_dir: "/tmp/s".into(), identity: None };
        let tgz = Path::new("/tmp/rt.tgz");
        let (title, steps) = server_job_plan(&srv(18000), &opts, "server_install", tgz, "d1g").unwrap();
        assert!(!title.is_empty());
        assert_eq!(steps.len(), 4);
        let cmds: Vec<&Cmd> = steps.iter().map(|s| match s { JobStep::Run(c) => c, other => panic!("{other:?}") }).collect();
        assert_eq!(cmds[0].stdin_file.as_deref(), Some(tgz));
        assert!(cmds[0].display().contains("runtime.new") && cmds[0].display().contains("d1g"));
        assert!(cmds[1].display().contains("SMT5_API_TOKEN"));
        assert!(cmds[2].display().contains("native.sh") && cmds[2].display().contains(" install"));
        assert!(cmds[3].display().contains(" start"));
        assert!(cmds.iter().all(|c| c.display().contains("BatchMode=yes")));
        let (_, upd) = server_job_plan(&srv(18000), &opts, "server_update", tgz, "d").unwrap();
        assert_eq!(upd.len(), 2);
        let docker = ServerConfig { mode: "docker".into(), ..srv(18000) };
        let (_, dupd) = server_job_plan(&docker, &opts, "server_update", tgz, "d").unwrap();
        assert!(matches!(&dupd[1], JobStep::Run(c) if c.display().contains("up -d --build")), "docker update rebuilds");
        // settings "apply" (up) and image build on a server (Final review I5)
        let run_of = |s: &ServerConfig, k: &str| match &server_job_plan(s, &opts, k, tgz, "d").unwrap().1[..] {
            [JobStep::Run(c)] => c.display(),
            other => panic!("{other:?}"),
        };
        assert!(run_of(&docker, "up").contains("up -d") && !run_of(&docker, "up").contains("--build"));
        assert!(run_of(&srv(18000), "up").contains("native.sh") && run_of(&srv(18000), "up").contains(" restart"));
        assert!(run_of(&docker, "build").contains("up -d --build"));
        let reset = run_of(&srv(18000), "server_reset");
        assert!(reset.contains("native.sh") && reset.contains(" reset"), "{reset}");
        for k in ["start", "stop", "restart", "server_reset", "server_uninstall"] {
            assert_eq!(server_job_plan(&srv(18000), &opts, k, tgz, "d").unwrap().1.len(), 1, "{k}");
        }
        assert!(server_job_plan(&srv(18000), &opts, "nope", tgz, "d").is_err());
        assert!(server_job_plan(&ServerConfig { mode: "weird".into(), ..srv(18000) }, &opts, "start", tgz, "d").is_err());
    }

    // ---- Windows server mode (Task 4)
    fn win() -> ServerConfig {
        ServerConfig { mode: "windows".into(), ssh: "admin@203.0.113.20".into(), ..srv(18200) }
    }
    fn run_cmds(steps: &[JobStep]) -> Vec<&Cmd> {
        steps.iter().map(|s| match s { JobStep::Run(c) => c, other => panic!("{other:?}") }).collect()
    }
    fn ps(c: &Cmd) -> &str {
        assert!(c.display().contains("powershell"), "{}", c.display());
        assert!(c.stdin_file.is_none());
        c.stdin_text.as_deref().expect("script on stdin")
    }

    #[test]
    fn windows_job_plan_uploads_with_scp() {
        let opts = smt5_core::remote::SshOpts { dir: "/c/ssh".into(), control_dir: "/tmp/s".into(), identity: None };
        let tgz = Path::new("/tmp/rt.tgz");
        let (_, steps) = server_job_plan(&win(), &opts, "server_install", tgz, "d1g").unwrap();
        let cmds = run_cmds(&steps);
        assert_eq!(cmds.len(), 5);
        assert_eq!(cmds[0].program, "scp");
        assert!(cmds[0].stdin_file.is_none() && cmds[0].stdin_text.is_none());
        assert!(cmds[0].args.iter().any(|a| a == "/tmp/rt.tgz"), "{:?}", cmds[0].args);
        assert_eq!(cmds[0].args.last().unwrap(), "admin@203.0.113.20:smt5-runtime.tgz");
        let up = ps(cmds[1]);
        assert!(up.contains("smt5-runtime.tgz") && up.contains("'d1g'"), "{up}");
        assert!(ps(cmds[2]).contains("SMT5_API_TOKEN") && ps(cmds[2]).contains("windows-native"));
        assert!(ps(cmds[3]).contains("smt5.ps1\" install"));
        assert!(ps(cmds[4]).contains("smt5.ps1\" start"));
        assert!(cmds[1..].iter().all(|c| c.program == "ssh" && c.display().contains("BatchMode=yes") && !c.args.iter().any(|a| a == "bash")));
        // settings "apply": stop, install (TightVNC port, Display task), start
        let (_, s) = server_job_plan(&win(), &opts, "up", tgz, "d").unwrap();
        let c = run_cmds(&s);
        assert_eq!(c.len(), 3, "[stop, install, start]");
        for (i, sub) in ["stop", "install", "start"].iter().enumerate() {
            assert!(ps(c[i]).contains(&format!("smt5.ps1\" {sub}")), "up step {i}");
        }
        // image build restarts the Windows runtime
        for k in ["build", "restart"] {
            let (_, s) = server_job_plan(&win(), &opts, k, tgz, "d").unwrap();
            let c = run_cmds(&s);
            assert_eq!(c.len(), 1, "{k}");
            assert!(ps(c[0]).contains("smt5.ps1\" restart"), "{k}");
        }
        for (k, sub) in [("start", "start"), ("stop", "stop"), ("server_reset", "reset"), ("server_uninstall", "uninstall")] {
            let (_, s) = server_job_plan(&win(), &opts, k, tgz, "d").unwrap();
            let c = run_cmds(&s);
            assert_eq!(c.len(), 1, "{k}");
            assert!(ps(c[0]).contains(&format!("smt5.ps1\" {sub}")), "{k}");
        }
    }

    #[test]
    fn windows_update_stops_before_upload() {
        let opts = smt5_core::remote::SshOpts { dir: "/c/ssh".into(), control_dir: "/tmp/s".into(), identity: None };
        let (_, steps) = server_job_plan(&win(), &opts, "server_update", Path::new("/tmp/rt.tgz"), "d2").unwrap();
        let cmds = run_cmds(&steps);
        assert_eq!(cmds.len(), 5, "[scp, stop, upload, install, start]");
        assert_eq!(cmds[0].program, "scp");
        // the runtime folder is locked while the supervisor runs: stop before the swap
        assert!(ps(cmds[1]).contains("smt5.ps1\" stop"));
        assert!(ps(cmds[2]).contains("smt5-runtime.tgz") && ps(cmds[2]).contains("'d2'"));
        assert!(ps(cmds[3]).contains("smt5.ps1\" install"));
        assert!(ps(cmds[4]).contains("smt5.ps1\" start"));
    }

    #[test]
    fn windows_snapshot_needs_the_install_marker() {
        // uploaded, .env made, install failed half way: VERSION + token exist, no state\installed
        let half = Secrets { version: Some("d".into()), token: Some("t".into()), password: Some("Ab3dEf7h".into()), installed: false, ..Default::default() };
        for session in [None, Some((false, false)), Some((true, false)), Some((true, true))] {
            let snap = server_snapshot(&win(), up(), Some(&half), Some(Err("refused".into())), session, "d");
            assert_eq!(snap.lifecycle, "not_installed", "{session:?}");
            assert!(snap.container.is_none());
        }
        let done = Secrets { installed: true, ..half.clone() };
        assert_eq!(server_snapshot(&win(), up(), Some(&done), Some(Err("refused".into())), None, "d").lifecycle, "stopped");
        // Linux servers never report INSTALLED: unchanged
        assert_eq!(server_snapshot(&srv(18000), up(), Some(&half), Some(Err("refused".into())), None, "d").lifecycle, "stopped");
    }

    #[test]
    fn autologon_status_line() {
        assert!(parse_autologon_status("on\r\n"));
        assert!(parse_autologon_status("\u{feff}on\r\n"));
        assert!(parse_autologon_status("WARNING: x\non\n"));
        assert!(!parse_autologon_status("off\n"));
        assert!(!parse_autologon_status(""));
    }

    #[test]
    fn server_snapshot_no_session() {
        let sec = Secrets { version: Some("d".into()), token: Some("t".into()), password: Some("Ab3dEf7h".into()), installed: true, ..Default::default() };
        let snap = |session| server_snapshot(&win(), up(), Some(&sec), Some(Err("refused".into())), session, "d");
        let ns = snap(Some((false, false)));
        assert_eq!(ns.lifecycle, "no_session");
        assert!(ns.container.is_none());
        assert_eq!(ns.status_error.as_deref(), Some("refused"));
        let stopped = snap(Some((true, false)));
        assert_eq!((stopped.lifecycle.as_str(), stopped.container.as_ref().map(|c| c.status.as_str())), ("stopped", Some("exited")));
        // the task runs but the API does not answer yet
        let starting = snap(Some((true, true)));
        assert_eq!((starting.lifecycle.as_str(), starting.container.as_ref().map(|c| c.status.as_str())), ("booting", Some("running")));
        assert_eq!(snap(None).lifecycle, "stopped", "not asked: as before");
        // labels
        assert!(["Sunucu · Windows", "Server · Windows"].contains(&ns.mode_label.as_str()), "{}", ns.mode_label);
        assert_eq!((ns.engine_label.as_str(), ns.image.as_str(), ns.provider_kind.as_str()), ("Windows", "MT5 (Windows)", "windows"));
        assert_eq!((ns.display.as_str(), ns.display_port), ("novnc", 18281));
        assert!(ns.display_url.ends_with("#password=Ab3dEf7h"));
        // Linux servers never use the session answer
        let lin = server_snapshot(&srv(18000), up(), Some(&sec), Some(Err("x".into())), Some((false, false)), "d");
        assert_eq!(lin.lifecycle, "stopped");
    }

    #[test]
    fn session_cache_ttl() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let c = SessionCache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = |v| {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                v
            }
        };
        let (ttl, to) = (Duration::from_secs(15), Duration::from_secs(2));
        let t0 = Instant::now();
        assert_eq!(c.get("a", t0, ttl, to, probe(Some((false, false)))), Some((false, false)));
        // within the TTL: the cached answer, no new probe
        assert_eq!(c.get("a", t0 + Duration::from_secs(5), ttl, to, probe(Some((true, true)))), Some((false, false)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // another server has its own entry
        assert_eq!(c.get("b", t0, ttl, to, probe(Some((true, false)))), Some((true, false)));
        // after the TTL: asked again
        assert_eq!(c.get("a", t0 + Duration::from_secs(16), ttl, to, probe(Some((true, true)))), Some((true, true)));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn session_cache_single_flight_and_timeout() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let c = SessionCache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let n = calls.clone();
        let t0 = Instant::now();
        let ttl = Duration::from_secs(15);
        // a hung probe: the caller gets None after the timeout
        let started = Instant::now();
        let got = c.get("a", t0, ttl, Duration::from_millis(100), move || {
            n.fetch_add(1, Ordering::SeqCst);
            let _ = release_rx.recv();
            Some((false, false))
        });
        assert_eq!(got, None);
        assert!(started.elapsed() < Duration::from_secs(2), "bounded by the timeout");
        // still in flight: no second probe, the last known value (none yet)
        let n2 = calls.clone();
        let again = c.get("a", t0 + Duration::from_secs(20), ttl, Duration::from_secs(2), move || {
            n2.fetch_add(1, Ordering::SeqCst);
            Some((true, true))
        });
        assert_eq!(again, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no re-entry while a probe runs");
        // the stale probe finishes in the background and fills the cache
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let n3 = calls.clone();
            let v = c.get("a", t0 + Duration::from_secs(1), ttl, Duration::from_secs(2), move || {
                n3.fetch_add(1, Ordering::SeqCst);
                Some((true, true))
            });
            if v == Some((false, false)) {
                break;
            }
            assert!(Instant::now() < deadline, "background result never cached: {v:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn parse_session_lines() {
        assert_eq!(parse_session("session=yes task=no\r\n"), Some((true, false)));
        assert_eq!(parse_session("WARNING: x\nsession=no task=no\n"), Some((false, false)));
        assert_eq!(parse_session("session=yes task=yes"), Some((true, true)));
        assert_eq!(parse_session("nothing"), None);
        assert_eq!(parse_session("\u{feff}session=yes task=yes\r\n"), Some((true, true)), "BOM from PowerShell");
        assert_eq!(parse_session(""), None);
    }

    #[test]
    fn windows_remote_settings() {
        let mut m = BTreeMap::new();
        m.insert("SMT5_RESOLUTION".to_owned(), "1500x770".to_owned());
        let lin = remote_env_update_script_for(ServerMode::LinuxNative, &m).unwrap();
        assert_eq!(lin, remote_env_update_script(&m).unwrap(), "Linux unchanged");
        let w = remote_env_update_script_for(ServerMode::Windows, &m).unwrap();
        assert!(w.contains("[ordered]@{ 'SMT5_RESOLUTION' = '1500x770' }") && w.contains("LOCALAPPDATA"), "{w}");
        m.insert("SMT5_MAXBARS".to_owned(), "200000".to_owned());
        let two = remote_env_update_script_for(ServerMode::Windows, &m).unwrap();
        assert!(two.contains("[ordered]@{ 'SMT5_MAXBARS' = '200000'; 'SMT5_RESOLUTION' = '1500x770' }"), "{two}");
        assert!(!w.contains("sed ") && !w.contains("set_kv"), "{w}");
        let mut bad = BTreeMap::new();
        bad.insert("SMT5_MAXBARS".to_owned(), "1';calc".to_owned());
        assert!(remote_env_update_script_for(ServerMode::Windows, &bad).is_err());
        let mut sec = BTreeMap::new();
        sec.insert("SMT5_API_TOKEN".to_owned(), "abc".to_owned());
        assert!(remote_env_update_script_for(ServerMode::Windows, &sec).is_err());
        // reading the .env
        assert_eq!(remote_env_script(ServerMode::Docker), "cat \"$HOME/siliconmetatrader5/runtime/.env\"");
        assert!(remote_env_script(ServerMode::Windows).contains("Get-Content") && remote_env_script(ServerMode::Windows).contains("LOCALAPPDATA"));
        // DPI is a noVNC/Xvfb setting: not shown for a Windows server
        let env = EnvFile::parse("SMT5_DPI=120\nSMT5_RESOLUTION=1500x770\n");
        let s = settings_from_env(&env, &EnvFile::parse(""), "x", true);
        assert!(!s.values.contains_key("SMT5_DPI") && s.values.contains_key("SMT5_RESOLUTION"));
        assert_eq!(settings_from_env(&env, &EnvFile::parse(""), "x", false).values.get("SMT5_DPI").map(String::as_str), Some("120"));
    }

    fn running(phase: Option<&str>) -> Facts {
        Facts {
            env_exists: true,
            engine_up: true,
            container: Some(ContainerState { status: "running".into(), health: None }),
            phase: phase.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn lifecycle_states() {
        assert_eq!(lifecycle(&running(Some("ready"))), "ready");
        assert_eq!(lifecycle(&running(None)), "booting");
        let exited = Facts { container: Some(ContainerState { status: "exited".into(), health: None }), ..running(None) };
        assert_eq!(lifecycle(&exited), "stopped");
        // after `compose down` the volumes remain: installed, just stopped
        let down = Facts { env_exists: true, engine_up: true, has_volumes: true, ..Default::default() };
        assert_eq!(lifecycle(&down), "stopped");
        assert_eq!(lifecycle(&Facts { env_exists: true, ..Default::default() }), "engine_down");
        let fresh = Facts { engine_up: true, ..Default::default() };
        assert_eq!(lifecycle(&fresh), "no_config");
        assert_eq!(lifecycle(&Facts { env_exists: true, ..fresh.clone() }), "no_image");
        assert_eq!(lifecycle(&Facts { env_exists: true, image_present: true, ..fresh }), "not_created");
    }

    #[test]
    fn known_accounts_most_recent_first() {
        let a = |login, server: &str| KnownAccount { login, server: server.into(), ..Default::default() };
        let mut list = vec![a(1, "S"), a(2, "S")];
        remember_account(&mut list, a(2, "S"));
        assert_eq!(list.iter().map(|x| x.login).collect::<Vec<_>>(), vec![2, 1]);
        remember_account(&mut list, a(2, "T")); // same login, other server: another account
        assert_eq!(list.len(), 3);
        let json = serde_json::json!({"login": 7, "server": "X", "trade_mode": "demo"});
        assert_eq!(KnownAccount::from_json(&json).map(|k| (k.login, k.trade_mode)), Some((7, "demo".into())));
        assert!(KnownAccount::from_json(&serde_json::json!(null)).is_none());
    }

    #[test]
    fn native_lifecycle_states() {
        let f = NativeFacts { installed: true, ..Default::default() };
        assert_eq!(native_lifecycle(true, &f, None), "stopped");
        assert_eq!(native_lifecycle(false, &f, None), "no_config");
        assert_eq!(native_lifecycle(true, &NativeFacts::default(), None), "not_installed");
        let up = NativeFacts { running: true, ..f };
        assert_eq!(native_lifecycle(true, &up, None), "booting");
        assert_eq!(native_lifecycle(true, &up, Some("ready".into())), "ready");
    }

    #[test]
    fn clipboard_sync_directions_without_echo() {
        let mut c = ClipSync::default();
        // first tick: the Mac clipboard goes to MT5
        assert_eq!(c.decide(Some("mac".into()), None), ClipAction::ToMt5("mac".into()));
        // MT5 now reports the same text: no echo
        assert_eq!(c.decide(Some("mac".into()), Some("mac".into())), ClipAction::None);
        // copied inside MT5 → goes to the Mac
        assert_eq!(c.decide(Some("mac".into()), Some("from mt5".into())), ClipAction::ToHost("from mt5".into()));
        assert_eq!(c.decide(Some("from mt5".into()), Some("from mt5".into())), ClipAction::None);
        // new copy on the Mac wins
        assert_eq!(c.decide(Some("new".into()), Some("from mt5".into())), ClipAction::ToMt5("new".into()));
        // image on the Mac clipboard (no text): nothing happens
        assert_eq!(c.decide(None, Some("new".into())), ClipAction::None);
    }

    #[test]
    fn reset_and_uninstall_need_safe_inputs() {
        let rt = tmp_runtime("u");
        let cfg = rt.root.join("config.json");
        assert!(job_plan(&rt, "nope", &JobOpts::default(), &cfg).is_err());
        let _ = std::fs::remove_dir_all(&rt.root);
    }

    #[test]
    fn display_urls() {
        let u = display_url(6082, Some("ab c"));
        assert!(u.starts_with("http://127.0.0.1:6082/vnc.html?autoconnect=1"));
        assert!(u.ends_with("#password=ab%20c"));
        assert!(!display_url(6081, None).contains('#'));
    }

    #[test]
    fn validators() {
        assert!(valid_port("6081"));
        assert!(!valid_port("80"));
        assert!(!valid_port("x"));
        assert!(valid_resolution("1366x768"));
        assert!(valid_resolution("1920x1080x24"));
        assert!(!valid_resolution("10x10"));
        assert!(!valid_resolution("1366"));
    }

    fn tmp_runtime(tag: &str) -> RuntimeDir {
        let dir = std::env::temp_dir().join(format!("smt5-desktop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("compose.yaml"), "name: siliconmt5\n").unwrap();
        std::fs::write(
            dir.join(".env.example"),
            "SMT5_MAXBARS=100000\nSMT5_DISPLAY_PORT=6081\nSMT5_BRIDGE_PORT=8001\nSMT5_STATUS_PORT=8002\nSMT5_WEB_PASSWORD=\nSMT5_API_TOKEN=\n",
        )
        .unwrap();
        RuntimeDir::new(dir)
    }

    #[test]
    fn settings_roundtrip_and_validation() {
        let rt = tmp_runtime("s");
        let s = settings(&rt);
        assert_eq!(s.values["SMT5_MAXBARS"], "100000");
        assert!(!s.env_exists);
        let mut v = BTreeMap::new();
        v.insert("SMT5_MAXBARS".to_owned(), "50000".to_owned());
        assert_eq!(save_settings(&rt, &v).unwrap(), vec!["SMT5_MAXBARS".to_owned()]);
        assert_eq!(settings(&rt).values["SMT5_MAXBARS"], "50000");
        v.insert("SMT5_MAXBARS".to_owned(), "12".to_owned());
        assert!(save_settings(&rt, &v).is_err());
        let mut v = BTreeMap::new();
        v.insert("SMT5_STATUS_PORT".to_owned(), "8001".to_owned());
        let e = save_settings(&rt, &v).unwrap_err();
        assert!(e.contains("farklı") || e.contains("different"), "{e}");
        let mut v = BTreeMap::new();
        v.insert("SMT5_API_TOKEN".to_owned(), "x".to_owned());
        assert!(save_settings(&rt, &v).is_err());
        let _ = std::fs::remove_dir_all(&rt.root);
    }

    #[test]
    fn regenerate_secrets() {
        let rt = tmp_runtime("r");
        init(&rt).unwrap();
        let before = reveal_password(&rt).unwrap();
        let r = regenerate(&rt, "password").unwrap();
        let after = reveal_password(&rt).unwrap();
        assert_ne!(before, after);
        assert_eq!(r.password.as_deref(), Some(after.as_str()));
        assert!(regenerate(&rt, "token").unwrap().password.is_none());
        assert!(regenerate(&rt, "nope").is_err());
        let _ = std::fs::remove_dir_all(&rt.root);
    }

    #[test]
    fn picking_repo_or_runtime_folder() {
        let rt = tmp_runtime("p");
        assert!(runtime_from_pick(&rt.root).is_ok());
        let parent = rt.root.parent().unwrap().to_path_buf();
        let repo = parent.join(format!("smt5-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::rename(&rt.root, repo.join("runtime")).unwrap();
        assert!(runtime_from_pick(&repo).is_ok());
        assert!(runtime_from_pick(&parent.join("does-not-exist-smt5")).is_err());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn server_clipboard_errors_say_what_to_do() {
        use smt5_core::api::ApiError;
        assert!(server_clipboard_error(ApiError::Http { status: 404, message: "not found".into() }).contains("Güncelle") || crate::i18n::is_en());
        let x = server_clipboard_error(ApiError::Http { status: 501, message: "xclip is not installed on the server (sudo apt install xclip)".into() });
        assert!(x.contains("xclip"));
        assert!(!server_clipboard_error(ApiError::Unreachable("connection refused".into())).is_empty());
    }
}
