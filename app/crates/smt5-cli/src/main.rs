//! `smt5` — command line for the SiliconMetaTrader5 runtime.
//!
//! Every docker command is pinned to the SiliconMetaTrader5 docker context for that
//! process only; the global `docker context use …` of other projects is never
//! changed. In the Mac yerel mode (`smt5 mode native`) build / up / down /
//! restart drive `runtime/macos/native.sh` instead of docker compose.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use smt5_core::api::{ApiClient, ApiError, DataTest, Inventory, PurgeItem, PurgeKind, PurgeResult, PurgeSpec, RestartTarget, Status};
use smt5_core::doctor::{self, Check, Level};
use smt5_core::provider::VmSize;
use smt5_core::util::{human_bytes, human_duration};
use smt5_core::{Cmd, Runner, RuntimeDir, SystemRunner, CONTAINER};

#[derive(Parser)]
#[command(name = "smt5", version, about = "Manage the SiliconMetaTrader5 runtime (MetaTrader 5 + Python bridge)")]
struct Cli {
    /// Path to the runtime folder (default: found from the current directory).
    #[arg(long, global = true)]
    runtime_dir: Option<PathBuf>,
    /// Machine-readable JSON output (doctor, status, data, purge, test).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check prerequisites and runtime health (exit 1 on failures).
    Doctor,
    /// Create runtime/.env and fill in a desktop password and API token.
    Init,
    /// Manage the Colima x86_64 VM of the Güvenli mode (macOS).
    Vm {
        #[command(subcommand)]
        action: VmAction,
    },
    /// Build the runtime image (Mac yerel: install Wine 11, Windows Python, MT5). Keeps the Mac awake.
    Build {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Pull the prebuilt image from GHCR.
    Pull,
    /// Start (or recreate) the runtime container (Mac yerel: the launchd agent).
    Up,
    /// Stop the runtime container (Mac yerel: the launchd agent).
    Down {
        /// Also delete the volumes (Mac yerel: the Wine prefix): MT5 installation, broker login and history.
        #[arg(short, long)]
        volumes: bool,
        /// Confirm --volumes.
        #[arg(long)]
        yes: bool,
    },
    /// Restart the container, or only MT5 / the bridge (needs the API token).
    Restart {
        #[arg(value_enum, default_value_t = RestartWhat::Container)]
        what: RestartWhat,
    },
    /// One-line runtime status.
    Status,
    /// Show the status every few seconds (keeps the Mac awake). Ctrl+C to stop.
    Watch {
        #[arg(long, default_value_t = 15)]
        interval: u64,
    },
    /// Show a runtime log.
    Logs {
        #[arg(value_enum, default_value_t = LogSource::Supervisor)]
        source: LogSource,
        #[arg(short = 'n', long, default_value_t = 80)]
        lines: u32,
    },
    /// MT5 history / tick / tester storage per server and symbol.
    Data,
    /// Delete MT5 history (dry run unless --yes). MT5 is stopped and restarted.
    Purge(PurgeArgs),
    /// Data test through the bridge: does real market data arrive?
    Test {
        #[arg(default_value = "EURUSD")]
        symbol: String,
        #[arg(long, default_value = "M5")]
        tf: String,
        #[arg(long, default_value_t = 100)]
        count: u32,
        /// Seconds of live ticks to sample.
        #[arg(long, default_value_t = 5.0)]
        ticks: f64,
    },
    /// Symbols in MT5's Market Watch.
    Symbols,
    /// Show or switch the runtime mode (macOS): safe (Colima x86_64 QEMU VM) | native (macOS Wine 11 + Rosetta 2).
    Mode {
        /// safe | native (omit to show the current mode)
        mode: Option<String>,
    },
    /// Mac yerel mode: copy the MT5 folder (incl. broker login and history)
    /// from the Güvenli mode container again; the current one is kept as a backup.
    Mt5Import {
        /// docker context of the source runtime
        #[arg(long, default_value = "colima-x86")]
        from: String,
    },
    /// macOS + Homebrew: copy SiliconMetaTrader5.app into Applications (Spotlight and
    /// Launchpad ignore links). Run again after `brew upgrade`.
    AppInstall,
    /// Speed test: Windows-side CPU, copy_rates_from_pos, bridge transfer and latency, boot time.
    Bench {
        #[arg(default_value = "EURUSD")]
        symbol: String,
        #[arg(long, default_value_t = 50_000)]
        bars: u32,
    },
    /// Open a shell inside the container.
    Shell,
    /// Run any docker compose command against the runtime.
    Compose {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum VmAction {
    /// Create or start the x86_64 VM (also enlarges the disk of an existing one).
    Start {
        #[arg(long, default_value_t = 4)]
        cpus: u32,
        #[arg(long, default_value_t = 8)]
        memory: u32,
        #[arg(long, default_value_t = 60)]
        disk: u32,
    },
    Stop,
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
enum RestartWhat {
    Container,
    Mt5,
    Bridge,
}

#[derive(Clone, Copy, ValueEnum)]
enum LogSource {
    Supervisor,
    Mt5,
    Bridge,
    Installer,
    Display,
    Status,
}

impl LogSource {
    fn name(self) -> &'static str {
        match self {
            Self::Supervisor => "supervisor",
            Self::Mt5 => "mt5",
            Self::Bridge => "bridge",
            Self::Installer => "installer",
            Self::Display => "display",
            Self::Status => "status",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    History,
    Ticks,
    Cache,
    Tester,
}

#[derive(Args)]
struct PurgeArgs {
    #[arg(long, value_enum)]
    kind: Kind,
    /// Broker server folder (e.g. FTMO-Demo), default all.
    #[arg(long)]
    server: Option<String>,
    /// Symbol (e.g. EURUSD), default all.
    #[arg(long)]
    symbol: Option<String>,
    /// History: delete these years (comma separated).
    #[arg(long, value_delimiter = ',')]
    years: Option<Vec<u32>>,
    /// History: delete years before this one.
    #[arg(long)]
    before_year: Option<u32>,
    /// Ticks: delete these months (YYYYMM, comma separated).
    #[arg(long, value_delimiter = ',')]
    months: Option<Vec<u32>>,
    /// Ticks: delete months before this one (YYYYMM).
    #[arg(long)]
    before_month: Option<u32>,
    /// Delete everything of this kind for the selected server/symbol.
    #[arg(long)]
    all: bool,
    /// Really delete (otherwise only show what would be deleted).
    #[arg(long)]
    yes: bool,
}

// ------------------------------------------------------------------ output --

struct Ui {
    color: bool,
}

impl Ui {
    fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_owned()
        }
    }

    fn level(&self, l: Level) -> String {
        match l {
            Level::Ok => self.paint("32", "✔"),
            Level::Warn => self.paint("33", "!"),
            Level::Fail => self.paint("31", "✘"),
            Level::Info => self.paint("34", "·"),
        }
    }

    fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
}

fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn status_line(s: &Status) -> String {
    let mut line = s.phase.clone();
    let detail = if s.detail.is_empty() { &s.supervisor.detail } else { &s.detail };
    if !detail.is_empty() {
        line.push_str(&format!(" - {detail}"));
    }
    if let Some(e) = s.phase_elapsed() {
        line.push_str(&format!("  ({})", human_duration(e)));
    }
    let probe = &s.bridge.probe;
    if let Some(a) = &probe.account {
        line.push_str(&format!(
            "  [{} {} {}]",
            a.server.as_deref().unwrap_or("?"),
            a.login_masked.as_deref().unwrap_or(""),
            a.trade_mode.as_deref().unwrap_or("")
        ));
    } else if s.phase == "waiting_login" {
        let why = probe.error.clone().unwrap_or_else(|| match &probe.terminal {
            Some(t) if !t.connected => "terminal not connected to broker".into(),
            _ => "no account yet".into(),
        });
        line.push_str(&format!("  [probe: {why}]"));
    }
    line
}

fn print_inventory(ui: &Ui, inv: &Inventory) {
    println!("MT5 data folder: {}", inv.root);
    println!(
        "Total {}  (bases {}, tester {})",
        human_bytes(inv.total_bytes),
        human_bytes(inv.bases_bytes),
        human_bytes(inv.tester_bytes)
    );
    if inv.servers.is_empty() {
        println!("No history found yet.");
    }
    for s in &inv.servers {
        let note = if s.is_default() { ui.dim("  MT5 default server, recreated on start") } else { String::new() };
        println!(
            "\n{}  {}  (bars {}, ticks {}, cache {}){note}",
            ui.paint("1", &s.name),
            human_bytes(s.bytes),
            human_bytes(s.history_bytes),
            human_bytes(s.ticks_bytes),
            human_bytes(s.cache_bytes)
        );
        for h in &s.history {
            let years: Vec<String> = h.years.iter().map(|(y, b)| format!("{y}:{}", human_bytes(*b))).collect();
            let cache = if h.cache_bytes > 0 { format!("   cache:{}", human_bytes(h.cache_bytes)) } else { String::new() };
            println!("  {:<12} {:>9}   {}{cache}", h.symbol, human_bytes(h.bytes), years.join("  "));
        }
        for t in &s.ticks {
            let mut parts = Vec::new();
            if let (Some(a), Some(b)) = (t.months.keys().next(), t.months.keys().last()) {
                parts.push(format!("{} monthly file(s) {a}..{b}", t.months.len()));
            }
            if t.other_bytes > 0 {
                parts.push(format!("live cache {}", human_bytes(t.other_bytes)));
            }
            let what = if parts.is_empty() { "empty".to_owned() } else { parts.join(", ") };
            println!("  {:<12} {:>9}   ticks: {what}", t.symbol, human_bytes(t.bytes));
        }
    }
}

fn print_purge(r: &PurgeResult) {
    let verb = if r.dry_run { "would delete" } else { "deleted" };
    println!("{verb} {} file(s), {}", r.removed.unwrap_or(r.files), human_bytes(r.freed_bytes.unwrap_or(r.bytes)));
    for s in r.sample.iter().take(12) {
        println!("  {s}");
    }
    if r.sample.len() > 12 || r.files as usize > r.sample.len().min(12) {
        println!("  … ({} in total)", r.files);
    }
    for f in &r.failed {
        println!("  failed: {f}");
    }
    if r.dry_run && r.files > 0 {
        println!("\nDry run. Re-run with --yes to delete (MT5 is stopped and restarted, ~1-2 min).");
    }
}

fn print_datatest(ui: &Ui, r: &DataTest) {
    for c in &r.checks {
        let mark = if c.ok { ui.level(Level::Ok) } else { ui.level(Level::Fail) };
        println!("  {mark} {:<32} {}  {}", c.title, c.detail, ui.dim(&format!("({} ms)", c.ms)));
    }
    if let Some(now) = &r.server_now {
        println!("  broker time: {now} ({})", r.server_offset.as_deref().unwrap_or("?"));
    }
    if let Some(t) = &r.ticks {
        if let (Some(b), Some(a)) = (t.bid, t.ask) {
            println!("  bid {b}  ask {a}  spread {} pts", t.spread_points.map(|s| s.to_string()).unwrap_or_else(|| "?".into()));
        }
    }
    let verdict = r.verdict.to_uppercase();
    let verdict = if r.passed() { ui.paint("32", &verdict) } else { ui.paint("31", &verdict) };
    println!("\n{verdict}: {}", r.message);
}

// ------------------------------------------------------------------ helpers --

/// Keep a Mac awake while this process lives (no-op elsewhere).
fn keep_awake() {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("caffeinate")
            .args(["-i", "-w", &std::process::id().to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

fn api_err(e: ApiError) -> anyhow::Error {
    match &e {
        ApiError::Http { status: 403, message } if message.contains("SMT5_API_TOKEN") => {
            anyhow::anyhow!("{e}\nRun `smt5 init` to create a token, then `smt5 up` to apply it.")
        }
        ApiError::Unreachable(_) => anyhow::anyhow!("{e}\nIs the runtime running? Try `smt5 doctor`."),
        _ => anyhow::anyhow!(e),
    }
}

fn exit(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1))
}

// --------------------------------------------------------------------- main --

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("smt5: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let ui = Ui { color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() };
    let runtime = RuntimeDir::discover(cli.runtime_dir.as_deref())?;
    let runner = SystemRunner;
    let ctx = doctor::Context::new(&runner, runtime.clone());
    let docker = ctx.docker();
    let api: &ApiClient = &ctx.api;
    let native = ctx.provider.kind.is_native();
    let native_cmd = |sub: &str| smt5_core::native::cmd(&runtime, sub);

    match cli.command {
        Command::Doctor => {
            let checks: Vec<Check> = doctor::run(&ctx);
            if cli.json {
                print_json(&checks)?;
            } else {
                println!("SiliconMetaTrader5 doctor  {}", ui.dim(&format!("(runtime: {})", runtime.root.display())));
                for c in &checks {
                    println!("  {} {:<28} {}", ui.level(c.level), c.title, c.detail);
                    if let Some(f) = &c.fix {
                        println!("      {} {}", ui.dim("fix:"), f);
                    }
                }
            }
            Ok(if doctor::summary(&checks) == Level::Fail { ExitCode::FAILURE } else { ExitCode::SUCCESS })
        }

        Command::Init => {
            let r = smt5_core::setup::init(&runtime, &[]).context("runtime/.env(.example) not readable")?;
            if cli.json {
                print_json(&r)?;
                return Ok(ExitCode::SUCCESS);
            }
            let what = match r.outcome {
                smt5_core::setup::InitOutcome::Created => "created",
                smt5_core::setup::InitOutcome::Updated => "updated",
                smt5_core::setup::InitOutcome::Unchanged => "unchanged",
            };
            println!("{what} {}", r.path);
            if r.generated.is_empty() {
                println!("  password and API token were already set");
            } else {
                println!("  generated: {}", r.generated.join(", "));
            }
            if let Some(p) = &r.password {
                println!("  desktop password: {p}   (also stored in {})", r.path);
            }
            if ctx.docker().container_state().is_some() && !r.generated.is_empty() {
                println!("Run `smt5 up` to apply the new values to the running container.");
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Vm { action } => {
            let p = &ctx.provider;
            if !p.kind.manages_engine() {
                bail!("this host uses {} — no VM to manage", p.kind.label());
            }
            match action {
                VmAction::Start { cpus, memory, disk } => {
                    let cmd = p.vm_start_cmd(VmSize { cpus, memory_gib: memory, disk_gib: disk }).expect("colima provider");
                    println!("$ {}", cmd.display());
                    // `colima start` makes its profile the global docker context;
                    // put back whatever other projects were using before.
                    let before = smt5_core::docker::global_context();
                    let code = runner.run_interactive(&cmd);
                    let after = smt5_core::docker::global_context();
                    if after != before {
                        let restore = Cmd::new("docker").args(["context", "use", before.as_str()]);
                        if runner.run(&restore).ok() {
                            println!("Restored the global docker context to '{before}' (colima had switched it to '{after}').");
                        } else {
                            eprintln!("warning: colima switched the global docker context to '{after}'; run `docker context use {before}`");
                        }
                    }
                    Ok(exit(code))
                }
                VmAction::Stop => {
                    let cmd = p.vm_stop_cmd().expect("colima provider");
                    println!("$ {}", cmd.display());
                    Ok(exit(runner.run_interactive(&cmd)))
                }
                VmAction::Status => {
                    match p.vm_profile(&runner) {
                        Some(vm) if cli.json => print_json(&vm)?,
                        Some(vm) => println!(
                            "{}: {} · {} · {} CPU · {} GiB RAM · {} GiB disk",
                            vm.name,
                            vm.status,
                            vm.arch,
                            vm.cpus,
                            vm.memory >> 30,
                            vm.disk >> 30
                        ),
                        None => println!("profile '{}' does not exist — `smt5 vm start` creates it", p.colima_profile.as_deref().unwrap_or("x86")),
                    }
                    Ok(ExitCode::SUCCESS)
                }
            }
        }

        Command::Build { args } => {
            keep_awake();
            if native {
                if !args.is_empty() {
                    bail!("the native mode has no image: `smt5 build` installs it and takes no arguments");
                }
                return Ok(exit(runner.run_interactive(&native_cmd("install"))));
            }
            let mut a = vec!["build".to_owned()];
            a.extend(args);
            Ok(exit(runner.run_interactive(&docker.compose(a))))
        }
        Command::Pull if native => bail!("the native mode has no image: use `smt5 build`"),
        Command::Pull => Ok(exit(runner.run_interactive(&docker.compose(["pull"])))),
        Command::Up if native => Ok(exit(runner.run_interactive(&native_cmd("start")))),
        Command::Up => Ok(exit(runner.run_interactive(&docker.compose(["up", "-d"])))),
        Command::Down { volumes, yes } => {
            if volumes && !yes {
                bail!("--volumes deletes the MT5 installation, broker login and all history; add --yes to confirm");
            }
            if native {
                return Ok(exit(runner.run_interactive(&native_cmd(if volumes { "reset" } else { "stop" }))));
            }
            let args: Vec<&str> = if volumes { vec!["down", "-v"] } else { vec!["down"] };
            Ok(exit(runner.run_interactive(&docker.compose(args))))
        }
        Command::Restart { what } => match what {
            RestartWhat::Container if native => Ok(exit(runner.run_interactive(&native_cmd("restart")))),
            RestartWhat::Container => Ok(exit(runner.run_interactive(&docker.compose(["restart"])))),
            RestartWhat::Mt5 | RestartWhat::Bridge => {
                let t = if matches!(what, RestartWhat::Mt5) { RestartTarget::Mt5 } else { RestartTarget::Bridge };
                let r = api.restart(t).map_err(api_err)?;
                println!("restart of {} requested — follow it with `smt5 watch`", r.target);
                Ok(ExitCode::SUCCESS)
            }
        },

        Command::Status => match api.status() {
            Ok(s) if cli.json => {
                print_json(&s)?;
                Ok(ExitCode::SUCCESS)
            }
            Ok(s) => {
                println!("{}", status_line(&s));
                Ok(ExitCode::SUCCESS)
            }
            Err(e) => {
                println!("{e}");
                Ok(ExitCode::FAILURE)
            }
        },
        Command::Watch { interval } => {
            keep_awake();
            loop {
                let now = Cmd::new("date").arg("+%H:%M:%S");
                let t = runner.run(&now).stdout_trimmed().to_owned();
                match api.status() {
                    Ok(s) => println!("{t}  {}", status_line(&s)),
                    Err(e) => println!("{t}  {e}"),
                }
                std::thread::sleep(Duration::from_secs(interval.max(1)));
            }
        }
        Command::Logs { source, lines } => {
            let l = api.logs(source.name(), lines).map_err(api_err)?;
            for line in l.lines {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Data => {
            let inv = api.data().map_err(api_err)?;
            if cli.json {
                print_json(&inv)?;
            } else {
                print_inventory(&ui, &inv);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Purge(a) => {
            let mut item = PurgeItem::new(match a.kind {
                Kind::History => PurgeKind::History,
                Kind::Ticks => PurgeKind::Ticks,
                Kind::Cache => PurgeKind::Cache,
                Kind::Tester => PurgeKind::Tester,
            });
            item.server = a.server;
            item.symbol = a.symbol;
            item.years = a.years;
            item.before_year = a.before_year;
            item.months = a.months;
            item.before_month = a.before_month;
            item.all = a.all.then_some(true);
            if !api.has_token() {
                bail!("SMT5_API_TOKEN is empty: run `smt5 init`, then `smt5 up`");
            }
            let r = api.purge(&PurgeSpec { items: vec![item], dry_run: !a.yes }).map_err(api_err)?;
            if cli.json {
                print_json(&r)?;
            } else {
                print_purge(&r);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Test { symbol, tf, count, ticks } => {
            if !cli.json {
                println!("Data test: {symbol} {tf} through the bridge (10-30 s under emulation)…");
            }
            let r = api.datatest(&symbol, &tf.to_uppercase(), count, ticks).map_err(api_err)?;
            if cli.json {
                print_json(&r)?;
            } else {
                print_datatest(&ui, &r);
            }
            Ok(if r.passed() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Command::Mode { mode } => {
            use smt5_core::mode::Mode;
            let path = runtime.env_path();
            let mut env = runtime.load_env();
            let current = Mode::current(&env);
            let Some(name) = mode else {
                if cli.json {
                    print_json(&serde_json::json!({ "mode": current, "label": current.label() }))?;
                } else {
                    println!("{} ({})", current.id(), current.label());
                    for m in Mode::ALL {
                        println!("  {} {:<10} {}", if m == current { "*" } else { " " }, m.id(), m.label());
                    }
                }
                return Ok(ExitCode::SUCCESS);
            };
            let Some(m) = Mode::parse(&name) else { bail!("unknown mode '{name}': safe | native") };
            if m == Mode::Native && std::env::consts::OS != "macos" && std::env::consts::ARCH != "x86_64" {
                bail!("the native mode needs macOS or Linux x86_64");
            }
            if !path.exists() {
                bail!("runtime/.env missing — run `smt5 init` first");
            }
            // Both modes bind the same localhost ports: stop the current one first,
            // otherwise the status API and the desktop still reach the old runtime.
            if m != current {
                if native {
                    if smt5_core::native::agent_loaded(&runner) {
                        println!("Stopping the native runtime first (same ports; its data stays)…");
                        if runner.run_interactive(&native_cmd("stop")) != 0 {
                            bail!("could not stop the native runtime; run `smt5 down`, then retry");
                        }
                    }
                } else if docker.container_state().map(|s| s.status == "running").unwrap_or(false) {
                    println!("Stopping the {} runtime first (same ports; its data stays)…", current.id());
                    if runner.run_interactive(&docker.compose(["stop"])) != 0 {
                        bail!("could not stop the current runtime; stop it with `smt5 compose stop`, then retry");
                    }
                }
            }
            let changed = m.apply(&mut env, std::env::consts::OS);
            env.save(&path)?;
            println!("mode: {} ({})", m.id(), m.label());
            if changed.is_empty() {
                println!("  already selected");
            } else {
                println!("  .env: {}", changed.join(", "));
            }
            if m == Mode::Native {
                println!("  Data stays separate (~/Library/Application Support/SiliconMT5). The first `smt5 build`");
                println!("  copies MT5 with its broker login from the Güvenli mode container when it exists.");
                println!("  Next: smt5 build · smt5 up · smt5 watch · smt5 bench");
            } else {
                println!("  Next: smt5 vm start · smt5 up");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::AppInstall => {
            if std::env::consts::OS != "macos" {
                bail!("app-install is for macOS Homebrew installs");
            }
            let before = smt5_core::macapp::status();
            let dest = smt5_core::macapp::install(&runner)?;
            println!(
                "SiliconMetaTrader5 {} → {}{}",
                before.brew_version.as_deref().unwrap_or("?"),
                dest.display(),
                before.copy_version.as_deref().map(|v| format!(" (was {v})")).unwrap_or_default()
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Mt5Import { from } => {
            if !native {
                bail!("mt5-import copies MT5 from the Güvenli/Docker mode into the native mode: run `smt5 mode native` first");
            }
            Ok(exit(runner.run_interactive(&native_cmd("import").env("SMT5_SAFE_CONTEXT", &from))))
        }
        Command::Bench { symbol, bars } => {
            if !cli.json {
                println!("Speed test: {symbol}, {bars} M1 bars (30-120 s under emulation)…");
            }
            let r = api.bench(&symbol, bars).map_err(api_err)?;
            if cli.json {
                print_json(&r)?;
            } else {
                let f = |v: Option<f64>, unit: &str| v.map(|x| format!("{x} {unit}")).unwrap_or_else(|| "—".into());
                println!("  mode            {} · wine {} {} · cpu {}", ctx.provider.kind.label(), r.wine_flavor, r.wine_version, r.cpu_translation.as_deref().unwrap_or("?"));
                println!("  boot → running  {}", f(r.boot_s, "s"));
                println!("  python cpu      {}", f(r.cpu_s, "s"));
                println!("  copy_rates      {} ({} rows)", f(r.rates_s, "s"), r.rates_rows.unwrap_or(0));
                println!("  10k bars xfer   {}", f(r.transfer_s, "s"));
                println!("  bridge rtt      {}", f(r.tick_ms, "ms"));
                if let Some(e) = &r.error {
                    println!("  error           {e}");
                }
            }
            Ok(if r.ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Command::Symbols => {
            let r = api.symbols().map_err(api_err)?;
            if cli.json {
                print_json(&r)?;
            } else {
                println!("Market Watch: {} of {} broker symbols", r.symbols.len(), r.total);
                println!("  {}", r.symbols.join("  "));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Shell | Command::Compose { .. } if native => {
            bail!("the native mode has no container; its files are in {}", smt5_core::native::NativePaths::current().base.display())
        }
        Command::Shell => Ok(exit(runner.run_interactive(&docker.cmd(["exec", "-it", CONTAINER, "bash"])))),
        Command::Compose { args } => Ok(exit(runner.run_interactive(&docker.compose(args)))),
    }
}
