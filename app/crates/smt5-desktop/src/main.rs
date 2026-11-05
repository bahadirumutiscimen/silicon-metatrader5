//! SiliconMetaTrader5 desktop app (Tauri v2).
//!
//! The window is a static HTML/JS UI (`ui/`, no Node build step). It talks to
//! the commands below, which call `backend` (→ `smt5-core`). Long jobs (VM
//! start, image build, compose up/down) stream their output as `job-line`
//! events. A tray / menu-bar icon shows the runtime phase.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod i18n;
mod servers;
mod targets;
#[cfg(test)]
mod e2e;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use smt5_core::api::{Accepted, DataTest, Inventory, Logs, PurgeResult, PurgeSpec};
use smt5_core::doctor::Check;
use smt5_core::setup::InitReport;
use smt5_core::{Cmd, Runner, RuntimeDir};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent, Wry};

use backend::{AppConfig, Resources, Settings, Snapshot};
use i18n::l;

const JOB_LINES: usize = 3000;

struct AppState {
    config_path: PathBuf,
    runtime: Mutex<Result<RuntimeDir, String>>,
    job: Mutex<JobView>,
    tray_status: Mutex<Option<MenuItem<Wry>>>,
    clip: Mutex<ClipState>,
    /// The tray icon was created (no tray: closing must not just hide the window).
    tray_ok: std::sync::atomic::AtomicBool,
    /// The UI asked to really exit (after its own stop sequence).
    exiting: std::sync::atomic::AtomicBool,
    /// Server mode: one ssh tunnel supervisor per server.
    targets: targets::Targets,
    /// Digest of this app's runtime folder (servers compare their VERSION).
    local_digest: Mutex<Option<String>>,
    /// Windows servers: "is a user session open" answers (cached, bounded).
    win_sessions: backend::SessionCache,
}

#[derive(Default)]
struct ClipState {
    /// Bumped on every enable/disable; a sync thread exits when it changes.
    generation: u64,
    enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
struct JobView {
    id: u64,
    kind: String,
    title: String,
    running: bool,
    code: Option<i32>,
    started: f64,
    finished: Option<f64>,
    lines: Vec<String>,
    #[serde(skip)]
    pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct JobLine {
    id: u64,
    line: String,
}

#[derive(Debug, Clone, Serialize)]
struct AppInfo {
    version: &'static str,
    os: &'static str,
    arch: &'static str,
    runtime_dir: Option<String>,
    runtime_error: Option<String>,
    /// Installed by Homebrew / a Linux package (how updates arrive).
    packaged: bool,
    /// UI language ("tr" / "en") and whether the user picked it already.
    lang: String,
    lang_chosen: bool,
    /// First-run wizard finished (otherwise only the wizard is shown).
    setup_complete: bool,
    /// What closing the window does (None: ask) and whether a tray icon exists.
    close_action: Option<String>,
    tray: bool,
    /// Status API level this build expects (older runtime scripts: restart).
    api_level: u32,
    /// Server mode: "local" or the active server id, and how many servers.
    active_target: String,
    servers: usize,
    /// (id, name) of each server, for the target switcher.
    targets: Vec<(String, String)>,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn runtime(app: &AppHandle) -> Result<RuntimeDir, String> {
    app.state::<AppState>().runtime.lock().map_err(|e| e.to_string())?.clone()
}

/// Run blocking work off the async runtime and the main thread.
async fn blocking<T, F>(app: AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(RuntimeDir) -> Result<T, String> + Send + 'static,
{
    let rt = runtime(&app)?;
    tauri::async_runtime::spawn_blocking(move || f(rt)).await.map_err(|e| e.to_string())?
}

/// What the pages are about: this computer or a server (Server mode).
enum Active {
    Local,
    Server(servers::ServerConfig),
}

fn active(app: &AppHandle) -> Active {
    let cfg = load_config(app);
    match servers::find(&cfg, &cfg.active_target) {
        Some(s) => Active::Server(s.clone()),
        None => Active::Local,
    }
}

/// The status API of the active target (local runtime, or a server's tunnel).
fn active_api(app: &AppHandle) -> Result<smt5_core::ApiClient, String> {
    match active(app) {
        Active::Local => Ok(backend::ctx(&runtime(app)?).api),
        Active::Server(s) => app
            .state::<AppState>()
            .targets
            .api(&s)
            .ok_or_else(|| lf!("{} sunucusuna bağlı değil", "not connected to {}", s.name)),
    }
}

/// Like `blocking`, with the active target's API client.
async fn blocking_api<T, F>(app: AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(smt5_core::ApiClient) -> Result<T, String> + Send + 'static,
{
    let api = active_api(&app)?;
    tauri::async_runtime::spawn_blocking(move || f(api)).await.map_err(|e| e.to_string())?
}

/// The runtime digest of this app (what a server should run), computed once.
fn local_digest(app: &AppHandle) -> String {
    let st = app.state::<AppState>();
    let mut d = st.local_digest.lock().expect("digest lock");
    if d.is_none() {
        *d = runtime(app).ok().and_then(|rt| smt5_core::remote_setup::runtime_digest(&rt).ok());
    }
    d.clone().unwrap_or_default()
}

fn config_dir(app: &AppHandle) -> PathBuf {
    app.state::<AppState>().config_path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// `ask_session`: a Windows server whose API is down may be asked about its
/// user session (not needed when only the display URL is wanted).
fn server_snapshot(app: &AppHandle, s: &servers::ServerConfig, ask_session: bool) -> Snapshot {
    let targets = &app.state::<AppState>().targets;
    let link = targets.link(&s.id);
    let secrets = targets.secrets(&s.id);
    let status = backend::server_wants_status(&link, secrets.as_ref())
        .then(|| secrets.as_ref().map(|sec| targets::api_with(s, sec).status().map_err(|e| e.to_string())))
        .flatten();
    let session = if ask_session { win_session(&app.state::<AppState>().win_sessions, &config_dir(app), s, status.as_ref()) } else { None };
    backend::server_snapshot(s, link, secrets.as_ref(), status, session, &local_digest(app))
}

/// A Windows server whose status API does not answer: is a user session
/// open, does the supervisor task run? (None: not Windows, API answered,
/// the question failed or did not answer within 8 s.) Answers are cached
/// for 15 s and one question per server runs at a time, so a hung ssh never
/// stalls the snapshot loop or the Servers page.
fn win_session(cache: &backend::SessionCache, dir: &std::path::Path, s: &servers::ServerConfig, status: Option<&Result<smt5_core::api::Status, String>>) -> Option<(bool, bool)> {
    if s.mode != "windows" || !matches!(status, Some(Err(_))) {
        return None;
    }
    let mut cmd = servers::exec(s, &servers::ssh_opts(dir, s), &smt5_core::remote_setup::win_action_script("session")).ok()?;
    // a dead connection is noticed in ~10 s instead of never
    cmd.args.splice(0..0, ["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2"].map(str::to_owned));
    cache.get(&s.id, std::time::Instant::now(), Duration::from_secs(15), Duration::from_secs(8), move || {
        let o = backend::RUNNER.run(&cmd);
        o.ok().then(|| backend::parse_session(&o.stdout)).flatten()
    })
}

/// CPU / memory of the runtime on a server, sampled over the SSH connection
/// (one sample at a time: a slow server never piles up polls).
fn server_resources(app: &AppHandle, s: &servers::ServerConfig) -> Resources {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    let none = Resources { container: None, vm: None };
    let Some(mode) = smt5_core::remote_setup::ServerMode::parse(&s.mode) else { return none };
    let Ok(cmd) = servers::exec(s, &servers::ssh_opts(&config_dir(app), s), &smt5_core::remote_setup::stats_script(mode)) else { return none };
    if !matches!(app.state::<AppState>().targets.link(&s.id), targets::Link::Connected { .. }) || BUSY.swap(true, Ordering::AcqRel) {
        return none;
    }
    let out = backend::RUNNER.run(&cmd);
    BUSY.store(false, Ordering::Release);
    Resources { container: out.ok().then(|| smt5_core::remote_setup::parse_stats(&out.stdout)).flatten(), vm: None }
}

// ------------------------------------------------------------ commands ----

#[tauri::command]
fn app_info(app: AppHandle) -> AppInfo {
    let rt = runtime(&app);
    let cfg = load_config(&app);
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        runtime_dir: rt.as_ref().ok().map(|r| r.root.display().to_string()),
        packaged: rt.as_ref().map(|r| r.packaged()).unwrap_or(false),
        runtime_error: rt.err(),
        lang: if i18n::is_en() { "en".into() } else { "tr".into() },
        lang_chosen: cfg.lang.is_some(),
        setup_complete: cfg.setup_complete,
        close_action: cfg.close_action.clone(),
        tray: app.state::<AppState>().tray_ok.load(std::sync::atomic::Ordering::Relaxed),
        api_level: smt5_core::api::API_LEVEL,
        active_target: if servers::find(&cfg, &cfg.active_target).is_some() { cfg.active_target.clone() } else { "local".into() },
        servers: cfg.servers.len(),
        targets: cfg.servers.iter().map(|s| (s.id.clone(), s.name.clone())).collect(),
    }
}

fn load_config(app: &AppHandle) -> AppConfig {
    AppConfig::load(&app.state::<AppState>().config_path)
}

fn save_config(app: &AppHandle, cfg: &AppConfig) -> Result<(), String> {
    cfg.save(&app.state::<AppState>().config_path).map_err(|e| lf!("ayar kaydedilemedi: {e}", "could not save the settings: {e}"))
}

/// Pick the UI language (first wizard step, Settings).
#[tauri::command]
fn set_lang(app: AppHandle, lang: String) -> Result<(), String> {
    if !matches!(lang.as_str(), "tr" | "en") {
        return Err(format!("unknown language {lang}"));
    }
    let mut cfg = load_config(&app);
    cfg.lang = Some(lang.clone());
    save_config(&app, &cfg)?;
    i18n::set(&lang);
    let _ = rebuild_tray_menu(&app);
    Ok(())
}

/// The first-run wizard is finished (or, `false`, must run again).
#[tauri::command]
fn set_setup_complete(app: AppHandle, done: bool) -> Result<(), String> {
    let mut cfg = load_config(&app);
    cfg.setup_complete = done;
    save_config(&app, &cfg)
}

#[tauri::command]
fn set_runtime_dir(app: AppHandle, path: String) -> Result<String, String> {
    let rt = backend::runtime_from_pick(std::path::Path::new(&path))?;
    let state = app.state::<AppState>();
    let cfg = AppConfig { runtime_dir: Some(rt.root.clone()), ..AppConfig::load(&state.config_path) };
    cfg.save(&state.config_path).map_err(|e| lf!("ayar kaydedilemedi: {e}", "could not save the settings: {e}"))?;
    let shown = rt.root.display().to_string();
    *state.runtime.lock().map_err(|e| e.to_string())? = Ok(rt);
    // a different runtime folder has a different digest ("update needed" on servers)
    *state.local_digest.lock().map_err(|e| e.to_string())? = None;
    Ok(shown)
}

#[tauri::command]
async fn snapshot(app: AppHandle, target: Option<String>) -> Result<Snapshot, String> {
    // target "local": this computer's runtime even while a server is active
    if let (Active::Server(s), false) = (active(&app), target.as_deref() == Some("local")) {
        return tauri::async_runtime::spawn_blocking(move || Ok(server_snapshot(&app, &s, true))).await.map_err(|e| e.to_string())?;
    }
    blocking(app, |rt| Ok(backend::snapshot(&rt))).await
}

#[tauri::command]
async fn resources(app: AppHandle) -> Result<Resources, String> {
    if let Active::Server(s) = active(&app) {
        return tauri::async_runtime::spawn_blocking(move || Ok(server_resources(&app, &s))).await.map_err(|e| e.to_string())?;
    }
    blocking(app, |rt| Ok(backend::resources(&rt))).await
}

#[tauri::command]
async fn doctor(app: AppHandle) -> Result<Vec<Check>, String> {
    blocking(app, |rt| Ok(backend::doctor(&rt))).await
}

#[tauri::command]
async fn logs(app: AppHandle, source: String, lines: Option<u32>) -> Result<Logs, String> {
    blocking_api(app, move |api| backend::logs(&api, &source, lines.unwrap_or(300))).await
}

#[tauri::command]
async fn inventory(app: AppHandle) -> Result<Inventory, String> {
    blocking_api(app, |api| backend::inventory(&api)).await
}

#[tauri::command]
async fn purge(app: AppHandle, spec: PurgeSpec) -> Result<PurgeResult, String> {
    blocking_api(app, move |api| backend::purge(&api, &spec)).await
}

#[tauri::command]
async fn datatest(app: AppHandle, symbol: String, tf: String, count: Option<u32>, ticks: Option<f64>) -> Result<DataTest, String> {
    blocking_api(app, move |api| backend::datatest(&api, &symbol, &tf, count.unwrap_or(300), ticks.unwrap_or(5.0))).await
}

#[tauri::command]
async fn restart(app: AppHandle, target: String) -> Result<Accepted, String> {
    if let Active::Server(s) = active(&app) {
        let dir = config_dir(&app);
        let api = active_api(&app);
        return tauri::async_runtime::spawn_blocking(move || backend::server_restart(&s, &dir, api, &target)).await.map_err(|e| e.to_string())?;
    }
    blocking(app, move |rt| backend::restart(&rt, &target)).await
}

#[tauri::command]
async fn settings(app: AppHandle) -> Result<Settings, String> {
    if let Active::Server(s) = active(&app) {
        let dir = config_dir(&app);
        return blocking(app, move |rt| {
            let env = remote_env(&dir, &s)?;
            let windows = s.mode == "windows";
            let path = if windows { format!("{}:%LOCALAPPDATA%\\siliconmetatrader5\\runtime\\.env", s.ssh) } else { format!("{}:~/siliconmetatrader5/runtime/.env", s.ssh) };
            Ok(backend::settings_from_env(&env, &smt5_core::EnvFile::load_or_default(&rt.env_example_path()), &path, windows))
        })
        .await;
    }
    blocking(app, |rt| Ok(backend::settings(&rt))).await
}

/// The mode of a saved server.
fn server_mode(s: &servers::ServerConfig) -> Result<smt5_core::remote_setup::ServerMode, String> {
    smt5_core::remote_setup::ServerMode::parse(&s.mode).ok_or_else(|| format!("unknown server mode {}", s.mode))
}

/// The server's `.env` (read over ssh).
fn remote_env(dir: &std::path::Path, s: &servers::ServerConfig) -> Result<smt5_core::EnvFile, String> {
    let o = backend::RUNNER.run(&servers::exec(s, &servers::ssh_opts(dir, s), &backend::remote_env_script(server_mode(s)?))?);
    if !o.ok() {
        return Err(lf!("sunucudaki .env okunamadı: {}", "could not read the server's .env: {}", o.stderr.trim()));
    }
    Ok(smt5_core::EnvFile::parse(&o.stdout))
}

#[tauri::command]
async fn save_settings(app: AppHandle, values: BTreeMap<String, String>) -> Result<Vec<String>, String> {
    if let Active::Server(s) = active(&app) {
        let dir = config_dir(&app);
        let handle = app.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            let mut env = remote_env(&dir, &s)?;
            let changed = backend::apply_settings(&mut env, &values)?;
            let pick: BTreeMap<String, String> = values.into_iter().filter(|(k, _)| changed.contains(k)).collect();
            if !pick.is_empty() {
                let script = backend::remote_env_update_script_for(server_mode(&s)?, &pick)?;
                let o = backend::RUNNER.run(&servers::exec(&s, &servers::ssh_opts(&dir, &s), &script)?);
                if !o.ok() {
                    return Err(o.stderr.trim().to_owned());
                }
                // ports may have changed: reconnect the tunnel with the new ones
                let t = &handle.state::<AppState>().targets;
                t.stop(&s.id);
                t.sync(&load_config(&handle).servers);
            }
            Ok(changed)
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    blocking(app, move |rt| backend::save_settings(&rt, &values)).await
}

#[tauri::command]
async fn regenerate(app: AppHandle, which: String) -> Result<InitReport, String> {
    if matches!(active(&app), Active::Server(_)) {
        return Err(l("Sunucunun şifresi ve token'ı sunucuda üretilir; burada değiştirilemez.", "A server's password and token are made on the server; they cannot be changed here.").into());
    }
    blocking(app, move |rt| backend::regenerate(&rt, &which)).await
}

#[tauri::command]
async fn reveal_password(app: AppHandle) -> Result<Option<String>, String> {
    if let Active::Server(s) = active(&app) {
        return Ok(app.state::<AppState>().targets.secrets(&s.id).and_then(|x| x.password));
    }
    blocking(app, |rt| Ok(backend::reveal_password(&rt))).await
}

#[tauri::command]
async fn init_env(app: AppHandle) -> Result<InitReport, String> {
    blocking(app, move |rt| backend::init(&rt)).await
}

#[tauri::command]
async fn components(app: AppHandle) -> Result<Vec<backend::Component>, String> {
    blocking(app, |rt| Ok(backend::components(&rt))).await
}

#[tauri::command]
async fn image_present(app: AppHandle) -> Result<bool, String> {
    blocking(app, |rt| backend::image_present(&rt)).await
}

#[tauri::command]
async fn strategy_supertrend(app: AppHandle, symbol: String, tf: String, period: Option<u32>, multiplier: Option<f64>, bars: Option<u32>) -> Result<serde_json::Value, String> {
    blocking_api(app, move |api| backend::supertrend(&api, &symbol, &tf, period.unwrap_or(10), multiplier.unwrap_or(3.0), bars.unwrap_or(3000))).await
}

#[tauri::command]
async fn example_source(app: AppHandle, name: String) -> Result<String, String> {
    blocking(app, move |rt| backend::example_source(&rt, &name)).await
}

#[tauri::command]
async fn accounts(app: AppHandle) -> Result<backend::Accounts, String> {
    let cfg = app.state::<AppState>().config_path.clone();
    blocking_api(app, move |api| Ok(backend::accounts(&api, &cfg))).await
}

#[tauri::command]
async fn account_history(app: AppHandle, range: String) -> Result<serde_json::Value, String> {
    blocking_api(app, move |api| backend::account_history(&api, &range)).await
}

#[tauri::command]
fn account_forget(app: AppHandle, login: u64, server: String) -> Result<Vec<backend::KnownAccount>, String> {
    backend::account_forget(&app.state::<AppState>().config_path, login, &server)
}

#[tauri::command]
async fn market_watch(app: AppHandle) -> Result<smt5_core::api::MarketWatch, String> {
    blocking_api(app, |api| backend::market_watch(&api)).await
}

#[tauri::command]
async fn removal_info(app: AppHandle) -> Result<backend::RemovalInfo, String> {
    let cfg = app.state::<AppState>().config_path.clone();
    blocking(app, move |rt| Ok(backend::removal_info(&rt, &cfg))).await
}

#[tauri::command]
async fn clear_logs(app: AppHandle) -> Result<smt5_core::api::Cleared, String> {
    blocking_api(app, |api| backend::clear_logs(&api)).await
}

/// Keep the Mac/Linux clipboard and MT5's in sync while the MT5 screen is open
/// (noVNC): this computer's container (X CLIPBOARD), or the active server's
/// MT5 through its status API (Windows clipboard / xclip). The UI turns it on and off.
#[tauri::command]
fn clipboard_sync(app: AppHandle, enabled: bool) -> Result<(), String> {
    let generation = {
        let st = app.state::<AppState>();
        let mut clip = st.clip.lock().map_err(|e| e.to_string())?;
        if clip.enabled == enabled {
            return Ok(());
        }
        clip.enabled = enabled;
        clip.generation += 1;
        clip.generation
    };
    if enabled {
        std::thread::spawn(move || clipboard_loop(app, generation));
    }
    Ok(())
}

/// MT5's clipboard on the active target: this computer's container or a server.
enum ClipTarget {
    Local(RuntimeDir),
    Server(Result<smt5_core::ApiClient, String>),
}

impl ClipTarget {
    fn get(&self) -> Result<Option<String>, String> {
        match self {
            ClipTarget::Local(rt) => backend::mt5_clipboard_get(rt),
            ClipTarget::Server(api) => api.clone()?.clipboard_get().map_err(backend::server_clipboard_error),
        }
    }
    fn set(&self, text: &str) -> Result<(), String> {
        match self {
            ClipTarget::Local(rt) => backend::mt5_clipboard_set(rt, text),
            ClipTarget::Server(api) => api.clone()?.clipboard_set(text).map_err(backend::server_clipboard_error),
        }
    }
}

/// MT5's clipboard on the active target, with the key that tells targets apart.
fn clip_target(app: &AppHandle) -> (String, Option<ClipTarget>) {
    match active(app) {
        Active::Server(s) => (s.id.clone(), Some(ClipTarget::Server(active_api(app)))),
        Active::Local => ("local".to_owned(), runtime(app).ok().map(ClipTarget::Local)),
    }
}

/// ⌘V in the MT5 screen (viewer.js): the host clipboard reaches MT5 right
/// before the paste key instead of on the next sync tick.
#[tauri::command]
async fn clipboard_push(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        let Some(text) = app.clipboard().read_text().ok().filter(|t| !t.is_empty() && t.len() <= backend::CLIP_MAX) else {
            return Ok(());
        };
        match clip_target(&app).1 {
            Some(target) => target.set(&text),
            None => Ok(()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn clipboard_loop(app: AppHandle, generation: u64) {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let mut sync = backend::ClipSync::default();
    let mut last_error: Option<String> = None;
    let mut last_target = String::new();
    loop {
        let alive = app.state::<AppState>().clip.lock().map(|c| c.enabled && c.generation == generation).unwrap_or(false);
        if !alive {
            return;
        }
        let (key, target) = clip_target(&app);
        if key != last_target {
            // another MT5: start from its clipboard, don't replay the previous one's
            sync = backend::ClipSync::default();
            last_target = key;
        }
        if let Some(target) = target {
            let host = app.clipboard().read_text().ok().filter(|t| !t.is_empty() && t.len() <= backend::CLIP_MAX);
            let result = target.get().and_then(|mt5| match sync.decide(host, mt5) {
                backend::ClipAction::ToMt5(t) => target.set(&t).map(|_| Some(("mt5", t.chars().count()))),
                backend::ClipAction::ToHost(t) => {
                    let n = t.chars().count();
                    app.clipboard().write_text(t).map_err(|e| e.to_string()).map(|_| Some(("host", n)))
                }
                backend::ClipAction::None => Ok(None),
            });
            match result {
                Ok(Some((to, chars))) => {
                    last_error = None;
                    let _ = app.emit("clipboard", serde_json::json!({ "to": to, "chars": chars }));
                }
                Ok(None) => last_error = None,
                Err(e) => {
                    if last_error.as_deref() != Some(e.as_str()) {
                        let _ = app.emit("clipboard", serde_json::json!({ "error": e }));
                        last_error = Some(e);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1200));
    }
}

#[tauri::command]
async fn bench(app: AppHandle, symbol: String, bars: Option<u32>) -> Result<smt5_core::api::Bench, String> {
    blocking_api(app, move |api| backend::bench(&api, &symbol, bars.unwrap_or(50_000))).await
}

#[tauri::command]
async fn modes(app: AppHandle) -> Result<Vec<backend::ModeInfo>, String> {
    blocking(app, |rt| Ok(backend::modes(&rt))).await
}

#[tauri::command]
async fn set_mode(app: AppHandle, mode: String) -> Result<Vec<String>, String> {
    if app.state::<AppState>().job.lock().map_err(|e| e.to_string())?.running {
        return Err(l("bir işlem sürerken mod değiştirilemez", "the mode cannot change while a job runs").into());
    }
    blocking(app, move |rt| backend::set_mode(&rt, &mode)).await
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.state::<AppState>().exiting.store(true, std::sync::atomic::Ordering::Relaxed);
    app.exit(0);
}

/// Remember what closing the window does (None: ask every time).
#[tauri::command]
fn set_close_action(app: AppHandle, action: Option<String>) -> Result<(), String> {
    if let Some(a) = &action {
        if !matches!(a.as_str(), "tray" | "quit") {
            return Err(format!("unknown close action {a}"));
        }
    }
    let mut cfg = load_config(&app);
    cfg.close_action = action;
    save_config(&app, &cfg)
}

/// Hide the main window (the app keeps running in the menu bar / tray).
#[tauri::command]
fn hide_main(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    hide_panel(&app);
}

#[tauri::command]
fn show_main_window(app: AppHandle) {
    hide_panel(&app);
    show_main(&app);
}

/// Tray panel → main window actions ("start", "stop", "quit_full", "quit_keep").
#[tauri::command]
fn tray_action(app: AppHandle, action: String) {
    send_tray_action(&app, &action);
}

fn send_tray_action(app: &AppHandle, action: &str) {
    hide_panel(app);
    if action.starts_with("quit") {
        show_main(app);
    }
    let _ = app.emit_to("main", "tray-action", action);
}

#[tauri::command]
fn job_state(app: AppHandle) -> Result<JobView, String> {
    Ok(app.state::<AppState>().job.lock().map_err(|e| e.to_string())?.clone())
}

#[tauri::command]
fn start_job(app: AppHandle, kind: String, opts: Option<backend::JobOpts>) -> Result<u64, String> {
    let rt = runtime(&app)?;
    let state = app.state::<AppState>();
    let opts = opts.unwrap_or_default();
    let cfg = load_config(&app);
    let server = servers::job_target(&cfg, &kind, opts.target.as_deref()).cloned();
    let (title, cmds) = match &server {
        Some(s) => {
            let dir = config_dir(&app);
            let tgz = dir.join("ssh").join("runtime.tar.gz");
            let _ = std::fs::create_dir_all(dir.join("ssh"));
            let digest = if kind == "server_install" || kind == "server_update" {
                smt5_core::remote_setup::runtime_tarball(&rt, &tgz, &backend::RUNNER)?
            } else {
                String::new()
            };
            backend::server_job_plan(s, &servers::ssh_opts(&dir, s), &kind, &tgz, &digest)?
        }
        None => backend::job_plan(&rt, &kind, &opts, &state.config_path)?,
    };
    let id = {
        let mut job = state.job.lock().map_err(|e| e.to_string())?;
        if job.running {
            return Err(lf!("başka bir işlem sürüyor: {}", "another job is running: {}", job.title));
        }
        *job = JobView { id: job.id + 1, kind: kind.clone(), title, running: true, started: now(), ..Default::default() };
        job.id
    };
    let handle = app.clone();
    let server_id = server.map(|s| s.id);
    std::thread::spawn(move || run_job(handle, id, kind, cmds, server_id));
    Ok(id)
}


#[tauri::command]
fn cancel_job(app: AppHandle) -> Result<bool, String> {
    let pid = app.state::<AppState>().job.lock().map_err(|e| e.to_string())?.pid;
    match pid {
        Some(pid) => Ok(backend::RUNNER.run(&Cmd::new("kill").args(["-TERM", &pid.to_string()])).ok()),
        None => Ok(false),
    }
}

/// Homebrew install: is the copy in Applications missing, a link or older?
#[tauri::command]
async fn app_copy_status() -> smt5_core::macapp::CopyStatus {
    tauri::async_runtime::spawn_blocking(smt5_core::macapp::status).await.unwrap_or_default()
}

/// Copy the Homebrew bundle into Applications; with `relaunch`, start the
/// new copy and quit this one.
#[tauri::command]
async fn app_copy_install(app: AppHandle, relaunch: bool) -> Result<String, String> {
    let dest = tauri::async_runtime::spawn_blocking(|| smt5_core::macapp::install(&backend::RUNNER).map_err(|e| format!("{e:#}")))
        .await
        .map_err(|e| e.to_string())??;
    if relaunch {
        std::process::Command::new("/usr/bin/open").arg("-n").arg(&dest).spawn().map_err(|e| e.to_string())?;
        app.exit(0);
    }
    Ok(dest.display().to_string())
}

/// Mac yerel mode: bring the MT5 window to the front.
#[tauri::command]
async fn focus_mt5(app: AppHandle) -> Result<(), String> {
    blocking(app, |_| backend::focus_mt5()).await
}

/// Open the noVNC desktop in its own window.
#[tauri::command]
async fn open_display_window(app: AppHandle) -> Result<(), String> {
    // the app's own viewer page (noVNC inside the app, ⌘ → Ctrl on a Mac); it
    // follows the active target by itself
    if let Some(w) = app.get_webview_window("mt5") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "mt5", WebviewUrl::App("index.html?viewer=1".into()))
        .title("SiliconMetaTrader5 — MetaTrader 5")
        .inner_size(1400.0, 860.0)
        .min_inner_size(800.0, 500.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Open a URL or a folder with the system handler (`open` / `xdg-open`).
#[tauri::command]
fn open_external(target: String) -> Result<(), String> {
    let allowed = target.starts_with("http://127.0.0.1:") || target.starts_with("http://localhost:") || target.starts_with("https://") || target.starts_with('/');
    if !allowed {
        return Err(l("bu adres açılamaz", "this address cannot be opened").into());
    }
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(program).arg(&target).spawn().map(|_| ()).map_err(|e| format!("{program}: {e}"))
}

// ----------------------------------------------------------------- jobs ----

fn push_line(app: &AppHandle, id: u64, line: String) {
    // remote Windows steps: PowerShell's CLIXML progress noise out, its errors as plain text
    let Some(line) = smt5_core::remote::ps_clixml_line(&line) else { return };
    if let Ok(mut job) = app.state::<AppState>().job.lock() {
        if job.id == id {
            if job.lines.len() >= JOB_LINES {
                job.lines.drain(..JOB_LINES / 4);
            }
            job.lines.push(line.clone());
        }
    }
    let _ = app.emit("job-line", JobLine { id, line });
}

fn run_job(app: AppHandle, id: u64, kind: String, steps: Vec<backend::JobStep>, server_id: Option<String>) {
    use backend::JobStep;
    let mut code = 0;
    let context_before = smt5_core::docker::global_context();
    let stream = |cmd: &Cmd| {
        push_line(&app, id, format!("$ {}", cmd.display()));
        let a = app.clone();
        let b = app.clone();
        smt5_core::exec::stream(
            cmd,
            |line| push_line(&a, id, line.to_owned()),
            |pid| {
                if let Ok(mut job) = b.state::<AppState>().job.lock() {
                    job.pid = Some(pid);
                }
                // keep the Mac awake while the job's process lives (image builds take long)
                if cfg!(target_os = "macos") {
                    let _ = std::process::Command::new("caffeinate").args(["-i", "-w", &pid.to_string()]).spawn();
                }
            },
        )
    };
    for step in steps {
        match step {
            JobStep::Run(cmd) => {
                code = stream(&cmd);
                if code != 0 {
                    break;
                }
            }
            JobStep::Try(cmd) => {
                let c = stream(&cmd);
                if c != 0 {
                    push_line(&app, id, lf!("(çıkış {c}, devam ediliyor)", "(exit {c}, continuing)"));
                }
            }
            JobStep::RemoveFile(path) => match std::fs::remove_file(&path) {
                Ok(()) => push_line(&app, id, lf!("silindi: {}", "deleted: {}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => push_line(&app, id, lf!("silinemedi: {} ({e})", "could not delete: {} ({e})", path.display())),
            },
            JobStep::ResetGlobalContext(deleted) => {
                if smt5_core::docker::global_context() == deleted {
                    let ok = backend::RUNNER.run(&Cmd::new("docker").args(["context", "use", "default"])).ok();
                    push_line(&app, id, lf!("genel docker context '{deleted}' silindiği için 'default' yapıldı{}", "the global docker context '{deleted}' was deleted, switched to 'default'{}", if ok { "" } else { l(" (başarısız)", " (failed)") }));
                }
            }
        }
    }
    if let Some(sid) = &server_id {
        let st = app.state::<AppState>();
        if code == 0 && kind == "server_uninstall" {
            let mut cfg = load_config(&app);
            cfg.servers.retain(|s| &s.id != sid);
            if &cfg.active_target == sid {
                cfg.active_target = "local".to_owned();
            }
            let _ = save_config(&app, &cfg);
            st.targets.sync(&cfg.servers);
        } else {
            st.targets.refresh_secrets(sid);
        }
    }
    if server_id.is_none() && code == 0 && (kind == "vm_start" || kind == "start" || kind == "up") {
        // `colima start` switches the global docker context to its profile.
        let after = smt5_core::docker::global_context();
        if after != context_before {
            let ok = backend::RUNNER.run(&Cmd::new("docker").args(["context", "use", context_before.as_str()])).ok();
            push_line(
                &app,
                id,
                if ok {
                    lf!("Genel docker context '{context_before}' olarak geri alındı (colima '{after}' yapmıştı).", "Restored the global docker context to '{context_before}' (colima had switched it to '{after}').")
                } else {
                    lf!("Uyarı: genel docker context '{after}' oldu; `docker context use {context_before}` çalıştırın.", "Warning: the global docker context is now '{after}'; run `docker context use {context_before}`.")
                },
            );
        }
    }
    if code == 0 && kind == "reset_vm" {
        // Level 2 reset: the setup wizard runs again from the first step.
        let mut cfg = load_config(&app);
        cfg.setup_complete = false;
        let _ = save_config(&app, &cfg);
    }
    push_line(&app, id, if code == 0 { l("✓ Tamamlandı", "✓ Done").into() } else { lf!("✗ Çıkış kodu {code}", "✗ Exit code {code}") });
    let view = {
        let st = app.state::<AppState>();
        let mut job = st.job.lock().expect("job lock");
        job.running = false;
        job.code = Some(code);
        job.finished = Some(now());
        job.pid = None;
        job.clone()
    };
    let _ = app.emit("job-done", view);
}

// ---------------------------------------------------------- Server mode ----

#[derive(Debug, Clone, Serialize)]
struct Probe {
    dest: String,
    host: String,
    port: u16,
    user: Option<String>,
    keys: Vec<smt5_core::remote::HostKey>,
    /// From the SSH banner: "windows" | "linux" | None (unknown).
    os: Option<String>,
}

/// Resolve an SSH target (aliases via `ssh -G`) and fetch its host keys.
fn probe_keys(ssh: &str) -> Result<Probe, String> {
    use smt5_core::remote::{host_keys, keyscan_cmd, parse_ssh_g, resolve_cmd, SshTarget};
    let t = SshTarget::parse(ssh).map_err(|_| lf!("geçersiz SSH hedefi: {}", "invalid SSH target: {}", ssh))?;
    let g = backend::RUNNER.run(&resolve_cmd(&t));
    let (host, port, user) = if g.ok() { parse_ssh_g(&g.stdout) } else { (t.host.clone(), t.port.unwrap_or(22), t.user.clone()) };
    let host = if host.is_empty() { t.host.clone() } else { host };
    let scan = backend::RUNNER.run(&keyscan_cmd(&host, port));
    let keys = host_keys(&scan.stdout, &backend::RUNNER);
    if keys.is_empty() {
        return Err(lf!("{host}:{port} adresine ulaşılamadı ya da SSH anahtarı alınamadı", "could not reach {host}:{port} or read its SSH host key"));
    }
    let banner = smt5_core::remote::read_banner(&host, port, Duration::from_secs(3));
    let os = servers::os_from_banner(banner.as_deref()).map(str::to_owned);
    Ok(Probe { dest: t.dest(), host, port, user: t.user.clone().or(user), keys, os })
}

#[tauri::command]
async fn server_probe(ssh: String) -> Result<Probe, String> {
    tauri::async_runtime::spawn_blocking(move || probe_keys(&ssh)).await.map_err(|e| e.to_string())?
}

/// Accept the host key `sha256` and save the server (not active yet).
#[tauri::command]
async fn server_add(app: AppHandle, name: String, ssh: String, identity: Option<String>, sha256: String) -> Result<servers::ServerConfig, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let probe = probe_keys(&ssh)?;
        let mut cfg = load_config(&app);
        let identity = match identity.filter(|p| !p.trim().is_empty()) {
            Some(p) => Some(servers::prepare_identity(&p)?),
            None => None,
        };
        let s = servers::new_server(&cfg, &name, &ssh, identity, &sha256)?;
        servers::remember_host_key(&servers::ssh_opts(&config_dir(&app), &s), &probe.keys, &sha256)?;
        cfg.servers.push(s.clone());
        save_config(&app, &cfg)?;
        app.state::<AppState>().targets.sync(&cfg.servers);
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn server_by_id(app: &AppHandle, id: &str) -> Result<servers::ServerConfig, String> {
    servers::find(&load_config(app), id).cloned().ok_or_else(|| lf!("sunucu bulunamadı: {id}", "server not found: {id}"))
}

#[derive(Debug, Clone, Serialize)]
struct SshCheck {
    ok: bool,
    kind: String,
    detail: String,
    /// The OS seen after the key login: "linux" | "windows" | None.
    os: Option<String>,
    /// `os` is not what the server was saved as (the wizard warns).
    os_mismatch: bool,
}

/// Can the app log in with a key (BatchMode)? Which OS answers: `uname -s`
/// (bash), otherwise PowerShell's `[Environment]::OSVersion`.
#[tauri::command]
async fn server_test(app: AppHandle, id: String) -> Result<SshCheck, String> {
    use smt5_core::remote::{classify_ssh_error, exec_cmd, exec_shell, Shell};
    tauri::async_runtime::spawn_blocking(move || {
        let s = server_by_id(&app, &id)?;
        let (t, opts) = (s.target()?, servers::ssh_opts(&config_dir(&app), &s));
        let ok = |os: Option<&str>| SshCheck { ok: true, kind: String::new(), detail: String::new(), os: os.map(str::to_owned), os_mismatch: servers::os_mismatch(&s.mode, os) };
        let fail = |stderr: &str| {
            let e = classify_ssh_error(stderr);
            SshCheck { ok: false, kind: e.kind().to_owned(), detail: e.detail(), os: None, os_mismatch: false }
        };
        // WSL's bash on a Windows server also says "Linux": it is marked
        let o = backend::RUNNER.run(&exec_cmd(&t, &opts, "uname -s; if grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null; then echo wsl; fi"));
        if o.ok() && servers::uname_os(&o.stdout).is_some() {
            return Ok(ok(Some("linux")));
        }
        if o.status == 255 || o.status < 0 {
            return Ok(fail(&o.stderr)); // ssh itself failed (key, host key, network)
        }
        // logged in, but no Linux bash (none, WSL, Git Bash, …): Windows?
        let w = backend::RUNNER.run(&exec_shell(&t, &opts, Shell::PowerShell, "[Environment]::OSVersion.Platform"));
        if w.ok() {
            return Ok(ok(w.stdout.contains("Win32NT").then_some("windows")));
        }
        Ok(if o.ok() { ok(None) } else { fail(&o.stderr) })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// After "host key changed": accept the server's new key.
#[tauri::command]
async fn server_rekey(app: AppHandle, id: String, sha256: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cfg = load_config(&app);
        let s = servers::find(&cfg, &id).cloned().ok_or("server not found")?;
        let probe = probe_keys(&s.ssh)?;
        servers::remember_host_key(&servers::ssh_opts(&config_dir(&app), &s), &probe.keys, &sha256)?;
        if let Some(x) = cfg.servers.iter_mut().find(|x| x.id == id) {
            x.host_key = sha256;
        }
        save_config(&app, &cfg)?;
        let t = &app.state::<AppState>().targets;
        t.stop(&id);
        t.sync(&cfg.servers);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn server_set_ports(app: AppHandle, id: String, base: u16) -> Result<(), String> {
    let mut cfg = load_config(&app);
    servers::check_port_base(&cfg, &id, base)?;
    let s = cfg.servers.iter_mut().find(|x| x.id == id).ok_or("server not found")?;
    s.port_base = base;
    save_config(&app, &cfg)?;
    app.state::<AppState>().targets.sync(&cfg.servers);
    Ok(())
}

#[tauri::command]
fn server_set_mode(app: AppHandle, id: String, mode: String) -> Result<(), String> {
    smt5_core::remote_setup::ServerMode::parse(&mode).ok_or_else(|| format!("unknown mode {mode}"))?;
    let mut cfg = load_config(&app);
    let s = cfg.servers.iter_mut().find(|x| x.id == id).ok_or("server not found")?;
    s.mode = mode;
    save_config(&app, &cfg)?;
    app.state::<AppState>().targets.sync(&cfg.servers);
    Ok(())
}

/// What [`server_keygen`] set up: the public key file and the line it ran in
/// Terminal (shown in the wizard so it can be copied).
#[derive(Debug, Clone, Serialize)]
struct KeySetup {
    pub_path: String,
    line: String,
}

/// Make (if needed) ~/.ssh/siliconmt5_ed25519, use it for this server and
/// open Terminal with ssh-copy-id (Windows: the key-install PowerShell line);
/// the server password is typed there.
#[tauri::command]
async fn server_keygen(app: AppHandle, id: String) -> Result<KeySetup, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let key = PathBuf::from(&home).join(".ssh").join("siliconmt5_ed25519");
        if !key.exists() {
            let _ = std::fs::create_dir_all(key.parent().unwrap_or(&key));
            let o = backend::RUNNER.run(&Cmd::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "siliconmetatrader5", "-f"]).arg(key.display().to_string()));
            if !o.ok() {
                return Err(format!("ssh-keygen: {}", o.stderr.trim()));
            }
        }
        let mut cfg = load_config(&app);
        let s = cfg.servers.iter_mut().find(|x| x.id == id).ok_or("server not found")?;
        s.identity = Some(key.clone());
        let s = s.clone();
        save_config(&app, &cfg)?;
        app.state::<AppState>().targets.sync(&cfg.servers);
        let t = s.target()?;
        let o = servers::ssh_opts(&config_dir(&app), &s);
        let pubkey = PathBuf::from(format!("{}.pub", key.display()));
        let line = if s.mode == "windows" {
            let text = std::fs::read_to_string(&pubkey).map_err(|e| lf!("{} okunamadı: {e}", "could not read {}: {e}", pubkey.display()))?;
            smt5_core::remote::copy_id_line_windows(&t, &o, &text)
        } else {
            smt5_core::remote::copy_id_line(&t, &o, &pubkey)
        };
        open_terminal(&line)?;
        Ok(KeySetup { pub_path: pubkey.display().to_string(), line })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn open_terminal(line: &str) -> Result<(), String> {
    let o = backend::RUNNER.run(&smt5_core::remote::open_terminal_cmd(std::env::consts::OS, line));
    if o.ok() {
        Ok(())
    } else {
        Err(lf!("Terminal açılamadı; bu komutu kendin çalıştır:\n{line}", "Could not open a terminal; run this yourself:\n{line}"))
    }
}

#[derive(Debug, Clone, Serialize)]
struct Precheck {
    /// Linux servers (`preflight.sh`); None for Windows.
    preflight: Option<smt5_core::remote_setup::Preflight>,
    /// Windows servers (`preflight.ps1`).
    win: Option<smt5_core::remote_setup::WinPreflight>,
    problems: Vec<smt5_core::remote_setup::Problem>,
    fix: Option<String>,
}

fn precheck(app: &AppHandle, id: &str, mode: &str) -> Result<Precheck, String> {
    use smt5_core::remote::{control_exit_cmd, exec_cmd, exec_shell, Shell};
    use smt5_core::remote_setup::{fix_script, parse_preflight, parse_win_preflight, problems, win_problems, ServerMode, PREFLIGHT_PS1, PREFLIGHT_SH};
    let m = ServerMode::parse(mode).ok_or_else(|| format!("unknown mode {mode}"))?;
    let s = server_by_id(app, id)?;
    let (t, o) = (s.target()?, servers::ssh_opts(&config_dir(app), &s));
    let failed = |stderr: &str| {
        let e = smt5_core::remote::classify_ssh_error(stderr);
        format!("{}: {}", e.kind(), e.detail())
    };
    if m == ServerMode::Windows {
        let out = backend::RUNNER.run(&exec_shell(&t, &o, Shell::PowerShell, PREFLIGHT_PS1));
        if !out.ok() {
            return Err(failed(&out.stderr));
        }
        let p = parse_win_preflight(&out.stdout)?;
        return Ok(Precheck { preflight: None, problems: win_problems(&p), win: Some(p), fix: None });
    }
    // a new docker group membership needs a fresh login
    let _ = backend::RUNNER.run(&control_exit_cmd(&t, &o));
    let out = backend::RUNNER.run(&exec_cmd(&t, &o, PREFLIGHT_SH));
    if !out.ok() {
        return Err(failed(&out.stderr));
    }
    let p = parse_preflight(&out.stdout)?;
    Ok(Precheck { problems: problems(&p, m), fix: fix_script(&p, m), preflight: Some(p), win: None })
}

#[tauri::command]
async fn server_precheck(app: AppHandle, id: String, mode: String) -> Result<Precheck, String> {
    tauri::async_runtime::spawn_blocking(move || precheck(&app, &id, &mode)).await.map_err(|e| e.to_string())?
}

/// Open Terminal with the fix command the precheck produced (sudo asks there).
/// The command is always built here, never taken from the UI.
#[tauri::command]
async fn server_fix(app: AppHandle, id: String, mode: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let nothing = || l("Düzeltilecek bir şey yok ya da bu sunucuda apt yok.", "Nothing to fix, or this server has no apt.").to_owned();
        if mode == "windows" {
            return Err(nothing()); // Windows problems are fixed by the user, no apt
        }
        let pc = precheck(&app, &id, &mode)?;
        let fix = pc.fix.ok_or_else(nothing)?;
        let s = server_by_id(&app, &id)?;
        let line = smt5_core::remote::terminal_ssh_line(&s.target()?, &servers::ssh_opts(&config_dir(&app), &s), &fix);
        open_terminal(&line)?;
        Ok(fix)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A Windows server, or an error saying the action is Windows only.
fn windows_server(app: &AppHandle, id: &str) -> Result<servers::ServerConfig, String> {
    let s = server_by_id(app, id)?;
    if s.mode != "windows" {
        return Err(lf!("{} bir Windows sunucusu değil", "{} is not a Windows server", s.name));
    }
    Ok(s)
}

/// Run `smt5.ps1 <action>` on a Windows server; its output.
fn win_action(app: &AppHandle, s: &servers::ServerConfig, action: &str) -> Result<String, String> {
    win_script(app, s, &smt5_core::remote_setup::win_action_script(action), &format!("smt5.ps1 {action}"))
}

/// Run a PowerShell script on a Windows server; its output, or an error
/// naming `what`.
fn win_script(app: &AppHandle, s: &servers::ServerConfig, script: &str, what: &str) -> Result<String, String> {
    let cmd = servers::exec(s, &servers::ssh_opts(&config_dir(app), s), script)?;
    let o = backend::RUNNER.run(&cmd);
    if o.ok() {
        return Ok(o.stdout);
    }
    if o.status == 255 || o.status < 0 {
        let e = smt5_core::remote::classify_ssh_error(&o.stderr);
        return Err(format!("{}: {}", e.kind(), e.detail()));
    }
    let detail = o.stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or_default().to_owned();
    Err(lf!("{what} başarısız: {detail}", "{what} failed: {detail}"))
}

/// Open Terminal with `smt5.ps1 autologon` on a Windows server: the Windows
/// password is typed there (Read-Host on the server); the app never sees it.
#[tauri::command]
async fn server_autologon(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = windows_server(&app, &id)?;
        let script = smt5_core::remote_setup::win_action_script("autologon");
        let line = smt5_core::remote::terminal_ps_line(&s.target()?, &servers::ssh_opts(&config_dir(&app), &s), &script);
        open_terminal(&line)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Is automatic logon on (`smt5.ps1 autologon-status` → `on`)?
#[tauri::command]
async fn server_autologon_status(app: AppHandle, id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = windows_server(&app, &id)?;
        Ok(backend::parse_autologon_status(&win_action(&app, &s, "autologon-status")?))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Restart a Windows server in 5 s (returns before ssh closes). A script of
/// its own, not `smt5.ps1 reboot`: the precheck offers it before the runtime
/// is on the server.
#[tauri::command]
async fn server_reboot(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = windows_server(&app, &id)?;
        win_script(&app, &s, &smt5_core::remote_setup::win_reboot_script(), l("Yeniden başlatma", "Restart"))?;
        app.state::<AppState>().targets.refresh_secrets(&id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Drop a server from the list without touching it (unreachable or gone):
/// its record, tunnel and known_hosts lines. Nothing runs on the server.
#[tauri::command]
async fn server_forget(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cfg = load_config(&app);
        let s = servers::find(&cfg, &id).cloned().ok_or("server not found")?;
        let t = s.target()?;
        let g = backend::RUNNER.run(&smt5_core::remote::resolve_cmd(&t));
        let field = if g.ok() {
            let (host, port, _) = smt5_core::remote::parse_ssh_g(&g.stdout);
            servers::known_host_field(&smt5_core::remote::SshTarget { user: None, host, port: Some(port) })
        } else {
            servers::known_host_field(&t)
        };
        servers::forget_host_key(&servers::ssh_opts(&config_dir(&app), &s), &field);
        cfg.servers.retain(|x| x.id != id);
        if cfg.active_target == id {
            cfg.active_target = "local".to_owned();
        }
        save_config(&app, &cfg)?;
        app.state::<AppState>().targets.stop(&id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn set_active_target(app: AppHandle, id: String) -> Result<(), String> {
    let mut cfg = load_config(&app);
    if id != "local" && servers::find(&cfg, &id).is_none() {
        return Err(lf!("sunucu bulunamadı: {id}", "server not found: {id}"));
    }
    cfg.active_target = id;
    save_config(&app, &cfg)
}

/// Cards for the Servers page (servers in parallel, plus this computer).
#[tauri::command]
async fn servers(app: AppHandle) -> Result<Vec<backend::ServerCard>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = load_config(&app);
        let digest = local_digest(&app);
        let dir = config_dir(&app);
        let sessions = app.state::<AppState>().win_sessions.clone();
        let active_id = if servers::find(&cfg, &cfg.active_target).is_some() { cfg.active_target.clone() } else { "local".into() };
        let mut cards = Vec::new();
        if let Ok(rt) = runtime(&app) {
            if rt.env_path().exists() {
                cards.push(backend::local_card(&backend::snapshot(&rt), active_id == "local"));
            }
        }
        let handles: Vec<_> = cfg
            .servers
            .iter()
            .cloned()
            .map(|s| {
                let targets = app.state::<AppState>().targets.clone();
                let digest = digest.clone();
                let dir = dir.clone();
                let sessions = sessions.clone();
                let active = s.id == active_id;
                std::thread::spawn(move || {
                    let link = targets.link(&s.id);
                    let secrets = targets.secrets(&s.id);
                    let (mut status, mut history, mut latency) = (None, None, None);
                    if let Some(sec) = secrets.as_ref().filter(|sec| backend::server_wants_status(&link, Some(sec))) {
                        let api = targets::api_with(&s, sec);
                        let t0 = std::time::Instant::now();
                        let st = api.status().map_err(|e| e.to_string());
                        latency = Some(t0.elapsed().as_millis() as u64);
                        if st.is_ok() {
                            history = api.account_history("today").ok();
                        }
                        status = Some(st);
                    }
                    let session = win_session(&sessions, &dir, &s, status.as_ref());
                    backend::server_card(&s, link, secrets.as_ref(), status, session, history, &digest, active, latency)
                })
            })
            .collect();
        cards.extend(handles.into_iter().filter_map(|h| h.join().ok()));
        Ok(cards)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ----------------------------------------------------------------- tray ----

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// When the panel last hid on blur: a click on the tray icon blurs it first,
/// and that same click must close it, not reopen it.
static PANEL_HIDDEN_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

const PANEL_W: f64 = 340.0;
const PANEL_H: f64 = 430.0;

fn hide_panel(app: &AppHandle) {
    if let Some(p) = app.get_webview_window("panel") {
        let _ = p.hide();
    }
}

/// The small status panel under (macOS) or above (Windows/Linux taskbar) the
/// tray icon: runtime state, Start/Stop, MT5 window, open the app, quit.
fn toggle_panel(app: &AppHandle, click: tauri::PhysicalPosition<f64>) {
    if now_ms().saturating_sub(PANEL_HIDDEN_AT.load(std::sync::atomic::Ordering::Relaxed)) < 400 {
        return;
    }
    let panel = match app.get_webview_window("panel") {
        Some(p) => {
            if p.is_visible().unwrap_or(false) {
                let _ = p.hide();
                return;
            }
            p
        }
        None => match WebviewWindowBuilder::new(app, "panel", WebviewUrl::App("index.html?panel=1".into()))
            .title("SiliconMetaTrader5")
            .inner_size(PANEL_W, PANEL_H)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("tray panel: {e}");
                show_main(app);
                return;
            }
        },
    };
    let scale = panel.scale_factor().unwrap_or(1.0);
    let (w, h) = (PANEL_W * scale, PANEL_H * scale);
    let (mut x, mut y) = (click.x - w / 2.0, click.y + 14.0 * scale);
    if let Ok(Some(m)) = panel.current_monitor().or_else(|_| app.primary_monitor()) {
        let (mx, my) = (m.position().x as f64, m.position().y as f64);
        let (mw, mh) = (m.size().width as f64, m.size().height as f64);
        if click.y - my > mh / 2.0 {
            y = click.y - h - 14.0 * scale; // taskbar at the bottom
        }
        x = x.clamp(mx + 8.0, mx + mw - w - 8.0);
        y = y.clamp(my + 8.0, my + mh - h - 8.0);
    }
    let _ = panel.set_position(tauri::PhysicalPosition::new(x, y));
    let _ = panel.show();
    let _ = panel.set_focus();
    let _ = panel.emit("panel-shown", ());
}

fn phase_label(snap: &Snapshot) -> String {
    match snap.lifecycle.as_str() {
        "engine_down" => lf!("{} kapalı", "{} is off", snap.engine_label),
        "no_config" | "no_image" | "not_created" | "not_installed" => l("Kurulum tamamlanmadı", "Setup not finished").into(),
        "stopped" => l("Durduruldu", "Stopped").into(),
        "booting" | "preparing_prefix" | "installing_mt5" | "starting_terminal" | "starting_bridge" => l("Başlatılıyor", "Starting").into(),
        "ready" => l("Hazır", "Ready").into(),
        "waiting_login" => l("Broker girişi bekleniyor", "Waiting for broker login").into(),
        "error" => l("Hata", "Error").into(),
        "maintenance" => l("Bakım", "Maintenance").into(),
        "connecting" => l("Bağlanıyor", "Connecting").into(),
        "unreachable" => l("Sunucuya ulaşılamıyor", "Server unreachable").into(),
        "installing" => l("Kuruluyor", "Installing").into(),
        "no_session" => l("Windows oturumu yok", "No Windows session").into(),
        "stopping" => l("Duruyor", "Stopping").into(),
        other => other.to_owned(),
    }
}

/// Tray menu in the current language; the status line is kept in AppState.
fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let status = MenuItem::with_id(app, "status", "SiliconMetaTrader5 — …", false, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", l("SiliconMetaTrader5'i göster", "Show SiliconMetaTrader5"), true, None::<&str>)?;
    let screen = MenuItem::with_id(app, "screen", l("MT5 ekranını aç", "Open the MT5 screen"), true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart_mt5", l("MT5'i yeniden başlat", "Restart MT5"), true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", l("Runtime'ı başlat", "Start the runtime"), true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", l("Runtime'ı durdur", "Stop the runtime"), true, None::<&str>)?;
    let quit_full = MenuItem::with_id(app, "quit_full", l("Tamamen kapat (MT5'i de durdur)", "Quit and stop everything"), true, None::<&str>)?;
    let quit_keep = MenuItem::with_id(app, "quit_keep", l("Uygulamadan çık (MT5 çalışmaya devam eder)", "Quit the app (MT5 keeps running)"), true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &sep1, &show, &screen, &restart, &sep2, &start, &stop, &sep3, &quit_full, &quit_keep])?;
    *app.state::<AppState>().tray_status.lock().expect("tray lock") = Some(status);
    Ok(menu)
}

fn rebuild_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(tray_menu(app)?))?;
    }
    Ok(())
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = tray_menu(app)?;

    let (icon, template) = if cfg!(target_os = "macos") {
        (tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?, true)
    } else {
        (tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?, false)
    };
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(template)
        .tooltip("SiliconMetaTrader5")
        .menu(&menu)
        // left click: the small status panel; right click: this menu
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "screen" => {
                // a server's screen is always noVNC in the window; only this computer's native MT5 is a window of its own
                let native = matches!(active(app), Active::Local)
                    && runtime(app).map(|rt| smt5_core::mode::Mode::current(&rt.load_env()) == smt5_core::mode::Mode::Native).unwrap_or(false);
                if !(native && backend::focus_mt5().is_ok()) {
                    show_main(app);
                    let _ = app.emit("navigate", "screen");
                }
            }
            "restart_mt5" => {
                let a = app.clone();
                std::thread::spawn(move || {
                    let r = match active(&a) {
                        Active::Server(s) => backend::server_restart(&s, &config_dir(&a), active_api(&a), "mt5"),
                        Active::Local => runtime(&a).and_then(|rt| backend::restart(&rt, "mt5")),
                    };
                    let _ = a.emit("toast", r.map(|_| l("MT5 yeniden başlatılıyor", "Restarting MT5").to_owned()).unwrap_or_else(|e| e));
                });
            }
            id @ ("start" | "stop" | "quit_full" | "quit_keep") => send_tray_action(app, id),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, position, .. } = event {
                toggle_panel(tray.app_handle(), position);
            }
        })
        .build(app)?;

    // Keep the tray line and tooltip current.
    let handle = app.clone();
    std::thread::spawn(move || loop {
        // the same target the window and the tray actions use
        let (snap, who) = match active(&handle) {
            Active::Server(s) => (Some(server_snapshot(&handle, &s, false)), format!("{} · ", s.name)),
            Active::Local => (runtime(&handle).ok().map(|rt| backend::snapshot(&rt)), String::new()),
        };
        if let Some(snap) = snap {
            let label = format!("{who}{}", phase_label(&snap));
            let account = snap
                .status
                .as_ref()
                .and_then(|s| s.bridge.probe.account.as_ref())
                .and_then(|a| a.server.clone())
                .map(|s| format!(" · {s}"))
                .unwrap_or_default();
            let text = format!("SiliconMetaTrader5 — {label}{account}");
            if let Some(item) = handle.state::<AppState>().tray_status.lock().ok().and_then(|g| g.clone()) {
                let _ = item.set_text(&text);
            }
            if let Some(tray) = handle.tray_by_id("main") {
                let _ = tray.set_tooltip(Some(&text));
            }
        }
        std::thread::sleep(Duration::from_secs(8));
    });
    Ok(())
}

// ----------------------------------------------------------------- main ----

fn main() {
    backend::extend_path();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let config_path = app.path().app_config_dir().map(|d| d.join("config.json")).unwrap_or_else(|_| PathBuf::from("siliconmt5-config.json"));
            let cfg = AppConfig::load(&config_path);
            i18n::set(&cfg.lang.clone().unwrap_or_else(|| i18n::system_default().to_owned()));
            // Packaged app: the runtime folder ships as a bundle resource.
            let bundled: Vec<std::path::PathBuf> = app.path().resource_dir().ok().map(|d| d.join("runtime")).into_iter().collect();
            let runtime = backend::find_runtime(&cfg, &bundled);
            let config_dir = config_path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let targets = targets::Targets::new(config_dir);
            targets.sync(&cfg.servers);
            app.manage(AppState {
                config_path,
                runtime: Mutex::new(runtime),
                job: Mutex::new(JobView::default()),
                tray_status: Mutex::new(None),
                clip: Mutex::new(ClipState::default()),
                tray_ok: Default::default(),
                exiting: Default::default(),
                targets,
                local_digest: Mutex::new(None),
                win_sessions: backend::SessionCache::default(),
            });
            match setup_tray(app.handle()) {
                Ok(()) => app.state::<AppState>().tray_ok.store(true, std::sync::atomic::Ordering::Relaxed),
                Err(e) => eprintln!("tray icon unavailable: {e}"),
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // The tray panel closes when it loses focus, like a menu.
            WindowEvent::Focused(false) if window.label() == "panel" => {
                if window.is_visible().unwrap_or(false) {
                    PANEL_HIDDEN_AT.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
                }
                let _ = window.hide();
            }
            // Closing the main window: menu bar, full stop, or ask (the UI decides,
            // because a full stop runs the stop job and shows its progress).
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                let app = window.app_handle();
                if app.state::<AppState>().exiting.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                api.prevent_close();
                let tray = app.state::<AppState>().tray_ok.load(std::sync::atomic::Ordering::Relaxed);
                match load_config(app).close_action.as_deref() {
                    Some("tray") if tray => {
                        let _ = window.hide();
                    }
                    Some("quit") => {
                        let _ = app.emit_to("main", "tray-action", "quit_full");
                    }
                    _ => {
                        let _ = app.emit_to("main", "close-requested", ());
                    }
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            server_probe,
            server_add,
            server_test,
            server_rekey,
            server_set_ports,
            server_set_mode,
            server_keygen,
            server_precheck,
            server_fix,
            server_autologon,
            server_autologon_status,
            server_reboot,
            set_active_target,
            server_forget,
            servers,
            set_lang,
            set_setup_complete,
            set_runtime_dir,
            snapshot,
            resources,
            doctor,
            logs,
            inventory,
            purge,
            datatest,
            strategy_supertrend,
            accounts,
            account_forget,
            account_history,
            example_source,
            restart,
            settings,
            save_settings,
            regenerate,
            reveal_password,
            init_env,
            image_present,
            components,
            removal_info,
            clear_logs,
            clipboard_sync,
            clipboard_push,
            quit_app,
            set_close_action,
            hide_main,
            show_main_window,
            tray_action,
            bench,
            modes,
            set_mode,
            market_watch,
            job_state,
            start_job,
            cancel_job,
            open_display_window,
            focus_mt5,
            app_copy_status,
            app_copy_install,
            open_external,
        ])
        .build(tauri::generate_context!())
        .expect("error while building SiliconMetaTrader5");

    app.run(|handle, event| match event {
        // Cmd+Q / Dock › Quit: ask too (unless the UI already decided to exit).
        tauri::RunEvent::ExitRequested { api, code: None, .. } if !handle.state::<AppState>().exiting.load(std::sync::atomic::Ordering::Relaxed) => {
            api.prevent_exit();
            show_main(handle);
            let _ = handle.emit_to("main", "quit-requested", ());
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => show_main(handle),
        // tunnels are children of the app: do not leave them behind
        tauri::RunEvent::Exit => handle.state::<AppState>().targets.stop_all(),
        _ => {}
    });
}
