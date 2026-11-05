//! Typed client for the in-container status API (runtime/status/smt5_status.py).
//!
//! Every struct is lenient (`#[serde(default)]`) so a newer runtime with extra
//! or missing fields never breaks an older CLI/app.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::env::EnvFile;

// ---------------------------------------------------------------- errors ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// Nothing answered on the port (runtime stopped or still starting).
    Unreachable(String),
    /// The API answered with an error status (`{"error": …}`).
    Http { status: u16, message: String },
    /// The answer was not the JSON we expected.
    Invalid(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(e) => write!(f, "status API not reachable: {e}"),
            Self::Http { status, message } => write!(f, "status API error {status}: {message}"),
            Self::Invalid(e) => write!(f, "unexpected status API response: {e}"),
        }
    }
}

impl std::error::Error for ApiError {}

// ----------------------------------------------------------------- types ----

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub version: String,
    pub display: String,
    pub phase: String,
    pub detail: String,
    pub supervisor: Supervisor,
    pub mt5: Mt5Info,
    pub bridge: BridgeInfo,
    pub host: HostTranslation,
    pub disk: Disk,
    pub api: ApiFlags,
    pub time: f64,
}

impl Status {
    /// Seconds since the supervisor entered its current phase.
    pub fn phase_elapsed(&self) -> Option<f64> {
        self.supervisor.updated.map(|u| (self.time - u).max(0.0))
    }

    pub fn is_ready(&self) -> bool {
        self.phase == "ready"
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Supervisor {
    pub phase: String,
    pub detail: String,
    pub updated: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mt5Info {
    pub installed: bool,
    pub running: bool,
    pub maxbars: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BridgeInfo {
    pub port: u16,
    pub listening: bool,
    pub probe: Probe,
    pub probe_fresh: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Probe {
    pub ok: bool,
    pub error: Option<String>,
    pub latency_ms: Option<f64>,
    pub terminal: Option<Terminal>,
    pub account: Option<Account>,
    pub positions: Option<i64>,
    pub orders: Option<i64>,
    pub checked_at: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terminal {
    pub connected: bool,
    pub build: Option<i64>,
    pub trade_allowed: bool,
    /// Values MT5 really uses (compare with `.env`).
    pub maxbars: Option<i64>,
    pub dlls_allowed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub login_masked: Option<String>,
    pub server: Option<String>,
    pub company: Option<String>,
    pub currency: Option<String>,
    pub trade_mode: Option<String>,
    pub leverage: Option<i64>,
    pub trade_expert: Option<bool>,
    pub balance: Option<f64>,
    pub equity: Option<f64>,
    pub profit: Option<f64>,
    pub margin: Option<f64>,
    pub margin_free: Option<f64>,
    pub margin_level: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostTranslation {
    pub cpu_translation: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Disk {
    pub path: String,
    pub free_mb: Option<i64>,
    pub total_mb: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiFlags {
    pub mutations_enabled: bool,
    /// Endpoint generation of the runtime's scripts (0: older than this field).
    pub level: u32,
}

/// The API level this build expects (runtime/status/smt5_status.py API_LEVEL).
pub const API_LEVEL: u32 = 3;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Logs {
    pub source: String,
    pub lines: Vec<String>,
    /// Bytes per log source, rotated copy included.
    #[serde(default)]
    pub sizes: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cleared {
    pub cleared: bool,
    pub freed_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inventory {
    pub root: String,
    pub install_dir: Option<String>,
    pub exists: bool,
    pub total_bytes: u64,
    pub bases_bytes: u64,
    pub tester_bytes: u64,
    pub servers: Vec<ServerData>,
    pub scanned_at: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerData {
    pub name: String,
    pub bytes: u64,
    pub history_bytes: u64,
    pub ticks_bytes: u64,
    pub cache_bytes: u64,
    pub history: Vec<SymbolHistory>,
    pub ticks: Vec<SymbolTicks>,
}

impl ServerData {
    /// MT5's own sample/offline server; recreated by the terminal on start.
    pub fn is_default(&self) -> bool {
        self.name.eq_ignore_ascii_case("default")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SymbolHistory {
    pub symbol: String,
    pub bytes: u64,
    pub years: BTreeMap<String, u64>,
    pub cache_bytes: u64,
    pub other_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SymbolTicks {
    pub symbol: String,
    pub bytes: u64,
    pub months: BTreeMap<String, u64>,
    pub other_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PurgeKind {
    History,
    Ticks,
    Cache,
    Tester,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeItem {
    pub kind: PurgeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub years: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub months: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_month: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all: Option<bool>,
}

impl PurgeItem {
    pub fn new(kind: PurgeKind) -> Self {
        Self { kind, server: None, symbol: None, years: None, before_year: None, months: None, before_month: None, all: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeSpec {
    pub items: Vec<PurgeItem>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PurgeResult {
    pub dry_run: bool,
    pub files: u64,
    pub bytes: u64,
    pub sample: Vec<String>,
    pub deleted: bool,
    pub removed: Option<u64>,
    pub freed_bytes: Option<u64>,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataTest {
    pub symbol: String,
    pub timeframe: String,
    pub checks: Vec<DataTestCheck>,
    pub methods: BTreeMap<String, DataTestMethod>,
    pub bars: Vec<DataTestBar>,
    pub ticks: Option<DataTestTicks>,
    pub account: Option<Account>,
    pub server_offset: Option<String>,
    pub server_offset_s: Option<i64>,
    pub server_now: Option<String>,
    /// ok | stale | market_closed | no_data | error
    pub verdict: String,
    pub message: String,
}

impl DataTest {
    pub fn passed(&self) -> bool {
        matches!(self.verdict.as_str(), "ok" | "market_closed")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataTestCheck {
    pub id: String,
    pub title: String,
    pub ok: bool,
    pub detail: String,
    pub ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataTestMethod {
    pub rows: u64,
    pub first: Option<String>,
    pub last: Option<String>,
    pub fresh_min: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataTestBar {
    pub t: String,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataTestTicks {
    pub count: u64,
    pub seconds: f64,
    pub digits: Option<i64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub spread_points: Option<f64>,
    pub samples: Vec<DataTestTick>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataTestTick {
    pub t: String,
    pub bid: f64,
    pub ask: f64,
}

/// Speed test (`GET /bench`): lower is better everywhere.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bench {
    pub ok: bool,
    pub error: Option<String>,
    pub symbol: String,
    /// Pure-Python loop inside Windows Python (Wine + CPU translation speed).
    pub cpu_s: Option<f64>,
    pub rates_rows: Option<u64>,
    /// copy_rates_from_pos(M1, bars) measured inside Windows Python.
    pub rates_s: Option<f64>,
    pub transfer_rows: Option<u64>,
    /// 10 000 M1 bars through the bridge to the Linux side.
    pub transfer_s: Option<f64>,
    /// One bridge round trip (symbol_info_tick), average.
    pub tick_ms: Option<f64>,
    /// Container start → MT5 + bridge running.
    pub boot_s: Option<f64>,
    pub wine_flavor: String,
    pub wine_version: String,
    pub cpu_translation: Option<String>,
    pub at: Option<f64>,
}

/// Symbols shown in MT5's Market Watch (`GET /symbols`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MarketWatch {
    pub symbols: Vec<String>,
    /// All symbols the broker offers.
    pub total: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accepted {
    pub accepted: bool,
    pub target: String,
}

// ---------------------------------------------------------------- client ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartTarget {
    Mt5,
    Bridge,
}

#[derive(Debug, Clone)]
pub struct ApiClient {
    pub base: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base: impl Into<String>, token: Option<String>) -> Self {
        Self { base: base.into().trim_end_matches('/').to_owned(), token }
    }

    /// From `.env`: `SMT5_STATUS_PORT` (default 8002) and `SMT5_API_TOKEN`.
    pub fn from_env(env: &EnvFile) -> Self {
        let port = env.get_nonempty("SMT5_STATUS_PORT").and_then(|p| p.parse::<u16>().ok()).unwrap_or(8002);
        Self::new(format!("http://127.0.0.1:{port}"), env.get_nonempty("SMT5_API_TOKEN").map(str::to_owned))
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn agent(timeout: Duration) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(3)).timeout(timeout).build()
    }

    fn handle<T: DeserializeOwned>(res: Result<ureq::Response, ureq::Error>) -> Result<T, ApiError> {
        match res {
            Ok(resp) => resp.into_json::<T>().map_err(|e| ApiError::Invalid(e.to_string())),
            Err(ureq::Error::Status(status, resp)) => {
                let message = resp
                    .into_json::<serde_json::Value>()
                    .ok()
                    .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
                    .unwrap_or_else(|| "request failed".to_owned());
                Err(ApiError::Http { status, message })
            }
            Err(e) => Err(ApiError::Unreachable(e.to_string())),
        }
    }

    fn get<T: DeserializeOwned>(&self, path: &str, timeout: Duration) -> Result<T, ApiError> {
        Self::handle(Self::agent(timeout).get(&format!("{}{path}", self.base)).call())
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value, timeout: Duration) -> Result<T, ApiError> {
        let mut req = Self::agent(timeout).post(&format!("{}{path}", self.base));
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        Self::handle(req.send_json(body.clone()))
    }

    pub fn health(&self) -> bool {
        self.get::<serde_json::Value>("/health", Duration::from_secs(5)).is_ok()
    }

    pub fn status(&self) -> Result<Status, ApiError> {
        self.get("/status", Duration::from_secs(10))
    }

    pub fn logs(&self, source: &str, lines: u32) -> Result<Logs, ApiError> {
        self.get(&format!("/logs?source={source}&lines={lines}"), Duration::from_secs(10))
    }

    pub fn data(&self) -> Result<Inventory, ApiError> {
        self.get("/data", Duration::from_secs(60))
    }

    /// Real purges stop and restart MT5: allow several minutes.
    pub fn purge(&self, spec: &PurgeSpec) -> Result<PurgeResult, ApiError> {
        let body = serde_json::to_value(spec).map_err(|e| ApiError::Invalid(e.to_string()))?;
        self.post("/data/purge", &body, Duration::from_secs(300))
    }

    pub fn datatest(&self, symbol: &str, tf: &str, count: u32, tick_seconds: f64) -> Result<DataTest, ApiError> {
        let sym: String = symbol.chars().filter(|c| c.is_ascii_alphanumeric() || "._#+&!-".contains(*c)).collect();
        self.get(&format!("/datatest?symbol={sym}&tf={tf}&count={count}&ticks={tick_seconds}"), Duration::from_secs(200))
    }

    pub fn bench(&self, symbol: &str, bars: u32) -> Result<Bench, ApiError> {
        let sym: String = symbol.chars().filter(|c| c.is_ascii_alphanumeric() || "._#+&!-".contains(*c)).collect();
        self.get(&format!("/bench?symbol={sym}&bars={bars}"), Duration::from_secs(330))
    }

    /// Supertrend backtest on closed bars (runtime/status/supertrend_strategy.py).
    pub fn supertrend(&self, symbol: &str, tf: &str, period: u32, multiplier: f64, bars: u32) -> Result<serde_json::Value, ApiError> {
        let sym: String = symbol.chars().filter(|c| c.is_ascii_alphanumeric() || "._#+&!-".contains(*c)).collect();
        let tf: String = tf.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        self.get(&format!("/strategy/supertrend?symbol={sym}&tf={tf}&period={period}&multiplier={multiplier}&bars={bars}"), Duration::from_secs(100))
    }

    /// The account MT5 is logged in to (MT5 is the source of truth).
    pub fn account(&self) -> Result<serde_json::Value, ApiError> {
        self.get("/account", Duration::from_secs(80))
    }

    /// Deal history, balance curve and P/L of the active account
    /// (`range`: today, 7d, 30d, 90d, all).
    pub fn account_history(&self, range: &str) -> Result<serde_json::Value, ApiError> {
        let r: String = range.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        self.get(&format!("/account/history?range={r}"), Duration::from_secs(80))
    }

    pub fn clear_logs(&self) -> Result<Cleared, ApiError> {
        self.post("/logs/clear", &serde_json::json!({}), Duration::from_secs(10))
    }

    pub fn symbols(&self) -> Result<MarketWatch, ApiError> {
        self.get("/symbols", Duration::from_secs(40))
    }

    /// MT5's clipboard text on a server (None: empty or not text). Token-protected.
    pub fn clipboard_get(&self) -> Result<Option<String>, ApiError> {
        let v: serde_json::Value = self.post("/clipboard/get", &serde_json::json!({}), Duration::from_secs(10))?;
        Ok(v.get("text").and_then(|t| t.as_str()).filter(|t| !t.is_empty()).map(str::to_owned))
    }

    /// Put `text` on MT5's clipboard on a server.
    pub fn clipboard_set(&self, text: &str) -> Result<(), ApiError> {
        let _: serde_json::Value = self.post("/clipboard/set", &serde_json::json!({ "text": text }), Duration::from_secs(10))?;
        Ok(())
    }

    pub fn restart(&self, target: RestartTarget) -> Result<Accepted, ApiError> {
        let path = match target {
            RestartTarget::Mt5 => "/runtime/restart-mt5",
            RestartTarget::Bridge => "/runtime/restart-bridge",
        };
        self.post(path, &serde_json::json!({}), Duration::from_secs(10))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// One-shot HTTP server: answers `status` + `body`, returns the raw request.
    fn serve_once(status: &str, body: &str) -> (String, mpsc::Receiver<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let (status, body) = (status.to_owned(), body.to_owned());
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = vec![0u8; 65536];
            let mut req = Vec::new();
            loop {
                let n = s.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).to_string();
                if let Some(idx) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if req.len() >= idx + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            write!(s, "HTTP/1.0 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            tx.send(String::from_utf8_lossy(&req).to_string()).unwrap();
        });
        (base, rx)
    }

    #[test]
    fn status_parses_real_payload() {
        let body = r#"{"version":"1.3.0","display":"novnc","phase":"ready","detail":"","supervisor":{"phase":"running","detail":"","updated":100.0},
          "mt5":{"installed":true,"running":true,"maxbars":100000},
          "bridge":{"port":8001,"listening":true,"probe":{"ok":true,"latency_ms":1.9,"terminal":{"connected":true,"build":6295,"trade_allowed":true},
          "account":{"login_masked":"•••409","server":"FTMO-Demo","company":"FTMO Global Markets Ltd","currency":"USD","trade_mode":"demo"},"checked_at":150.0},"probe_fresh":true},
          "host":{"cpu_translation":"none"},"disk":{"path":"/config","free_mb":40000,"total_mb":58000},"api":{"mutations_enabled":true},"time":160.0,"future_field":1}"#;
        let (base, _rx) = serve_once("200 OK", body);
        let s = ApiClient::new(base, None).status().unwrap();
        assert!(s.is_ready());
        assert_eq!(s.phase_elapsed(), Some(60.0));
        assert_eq!(s.bridge.probe.account.unwrap().server.as_deref(), Some("FTMO-Demo"));
        assert_eq!(s.mt5.maxbars, Some(100000));
    }

    #[test]
    fn clipboard_get_and_set_send_the_token() {
        let (base, rx) = serve_once("200 OK", r#"{"text":"şimdi"}"#);
        assert_eq!(ApiClient::new(base, Some("tok".into())).clipboard_get().unwrap().as_deref(), Some("şimdi"));
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /clipboard/get") && req.contains("Bearer tok"), "{req}");
        let (base, _rx) = serve_once("200 OK", r#"{"text":null}"#);
        assert_eq!(ApiClient::new(base, Some("tok".into())).clipboard_get().unwrap(), None);
        let (base, rx) = serve_once("200 OK", r#"{"ok":true,"chars":3}"#);
        ApiClient::new(base, Some("tok".into())).clipboard_set("çok").unwrap();
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /clipboard/set") && req.contains("\"text\":\"çok\""), "{req}");
        // an old runtime without the endpoint: a clear HTTP error, not a panic
        let (base, _rx) = serve_once("404 Not Found", r#"{"error":"not found"}"#);
        assert!(matches!(ApiClient::new(base, Some("tok".into())).clipboard_get(), Err(ApiError::Http { status: 404, .. })));
    }

    #[test]
    fn purge_sends_token_and_spec() {
        let (base, rx) = serve_once("200 OK", r#"{"dry_run":true,"files":16,"bytes":103502396,"sample":["Bases/Default/History/EURUSD/2023.hcc"],"deleted":false}"#);
        let mut item = PurgeItem::new(PurgeKind::History);
        item.server = Some("Default".into());
        item.all = Some(true);
        let r = ApiClient::new(base, Some("tok".into())).purge(&PurgeSpec { items: vec![item], dry_run: true }).unwrap();
        assert_eq!((r.files, r.bytes, r.deleted), (16, 103502396, false));
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /data/purge"));
        assert!(req.contains("Authorization: Bearer tok") || req.contains("authorization: Bearer tok"));
        assert!(req.contains(r#""kind":"history""#) && req.contains(r#""server":"Default""#) && req.contains(r#""all":true"#));
        assert!(!req.contains("before_year"), "None fields must not be sent");
    }

    #[test]
    fn http_errors_carry_the_api_message() {
        let (base, _rx) = serve_once("401 Unauthorized", r#"{"error":"missing or invalid bearer token"}"#);
        let e = ApiClient::new(base, None).restart(RestartTarget::Mt5).unwrap_err();
        assert_eq!(e, ApiError::Http { status: 401, message: "missing or invalid bearer token".into() });
    }

    #[test]
    fn unreachable() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(); // freed right away
        let e = ApiClient::new(format!("http://127.0.0.1:{port}"), None).status().unwrap_err();
        assert!(matches!(e, ApiError::Unreachable(_)));
    }

    #[test]
    fn from_env() {
        let c = ApiClient::from_env(&EnvFile::parse("SMT5_STATUS_PORT=9002\nSMT5_API_TOKEN=\n"));
        assert_eq!(c.base, "http://127.0.0.1:9002");
        assert!(!c.has_token());
    }
}
