//! External commands behind a trait, so providers and doctor checks can be
//! tested without Docker or Colima.

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    /// Feed this file as stdin (default: /dev/null).
    pub stdin_file: Option<PathBuf>,
    /// Feed this text as stdin, then close it (wins over `stdin_file`).
    pub stdin_text: Option<String>,
}

impl Cmd {
    pub fn new(program: &str) -> Self {
        Self { program: program.to_owned(), ..Default::default() }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(it.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, k: &str, v: &str) -> Self {
        self.env.push((k.to_owned(), v.to_owned()));
        self
    }

    pub fn envs(mut self, kv: &[(String, String)]) -> Self {
        self.env.extend(kv.iter().cloned());
        self
    }

    pub fn stdin_file(mut self, p: impl Into<PathBuf>) -> Self {
        self.stdin_file = Some(p.into());
        self
    }

    pub fn stdin_text(mut self, s: impl Into<String>) -> Self {
        self.stdin_text = Some(s.into());
        self
    }

    pub fn cwd(mut self, p: impl Into<PathBuf>) -> Self {
        self.cwd = Some(p.into());
        self
    }

    /// `program arg1 arg2` (for logs and error messages).
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// Exit code; -1 when the program could not be started or was killed.
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == 0
    }

    pub fn stdout_trimmed(&self) -> &str {
        self.stdout.trim()
    }
}

pub trait Runner: Send + Sync {
    /// Run to completion, capturing output. A missing program is not an
    /// error: it yields `status == -1` with the reason in `stderr`.
    fn run(&self, cmd: &Cmd) -> Output;

    /// Run attached to the terminal (inherited stdio); returns the exit code.
    fn run_interactive(&self, cmd: &Cmd) -> i32;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

/// stdin for `cmd`: a pipe for `stdin_text`, else its `stdin_file`, else /dev/null.
fn stdin_of(cmd: &Cmd) -> Stdio {
    if cmd.stdin_text.is_some() {
        return Stdio::piped();
    }
    match cmd.stdin_file.as_ref().and_then(|p| std::fs::File::open(p).ok()) {
        Some(f) => Stdio::from(f),
        None => Stdio::null(),
    }
}

/// The `std::process::Command` for `cmd` (env and cwd applied).
pub fn build(cmd: &Cmd) -> Command {
    let mut c = Command::new(&cmd.program);
    c.args(&cmd.args);
    for (k, v) in &cmd.env {
        c.env(k, v);
    }
    if let Some(d) = &cmd.cwd {
        c.current_dir(d);
    }
    c
}

/// Write `cmd.stdin_text` (if any) to the child's stdin from a thread, so a
/// child that answers before reading everything cannot deadlock us; stdin is
/// closed when the thread finishes.
fn feed_stdin(cmd: &Cmd, child: &mut std::process::Child) -> Option<std::thread::JoinHandle<()>> {
    let text = cmd.stdin_text.clone()?;
    let mut stdin = child.stdin.take()?;
    Some(std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(text.as_bytes());
    }))
}

impl Runner for SystemRunner {
    fn run(&self, cmd: &Cmd) -> Output {
        let res = build(cmd).stdin(stdin_of(cmd)).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().and_then(
            |mut child| {
                let feeder = feed_stdin(cmd, &mut child);
                let out = child.wait_with_output();
                if let Some(f) = feeder {
                    let _ = f.join();
                }
                out
            },
        );
        match res {
            Ok(o) => Output {
                status: o.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Output { status: -1, stdout: String::new(), stderr: format!("{}: {e}", cmd.program) },
        }
    }

    fn run_interactive(&self, cmd: &Cmd) -> i32 {
        match build(cmd).status() {
            Ok(s) => s.code().unwrap_or(-1),
            Err(e) => {
                eprintln!("{}: {e}", cmd.program);
                -1
            }
        }
    }
}

/// Run `cmd` with stdout and stderr merged line by line into `on_line`;
/// returns the exit code (-1 if it could not start or was killed). Used for
/// long jobs (VM start, image build) whose progress the desktop app streams.
pub fn stream(cmd: &Cmd, mut on_line: impl FnMut(&str), on_spawn: impl FnOnce(u32)) -> i32 {
    use std::io::{BufRead, BufReader};
    let mut child = match build(cmd).stdin(stdin_of(cmd)).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            on_line(&format!("{}: {e}", cmd.program));
            return -1;
        }
    };
    on_spawn(child.id());
    let feeder = feed_stdin(cmd, &mut child);
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }));
    }
    drop(tx);
    for line in rx {
        // docker/colima redraw progress with \r; keep the last state of the line
        let last = line.rsplit('\r').find(|s| !s.trim().is_empty()).unwrap_or("");
        on_line(last);
    }
    for r in readers.into_iter().chain(feeder) {
        let _ = r.join();
    }
    child.wait().ok().and_then(|s| s.code()).unwrap_or(-1)
}

/// Scripted runner for tests: the first rule whose `program args…` prefix
/// matches the command answers it. Unmatched commands fail with status 127.
#[derive(Default)]
pub struct MockRunner {
    rules: Vec<(String, Output)>,
    pub calls: std::sync::Mutex<Vec<Cmd>>,
}

impl MockRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on(mut self, prefix: &str, status: i32, stdout: &str) -> Self {
        self.rules.push((prefix.to_owned(), Output { status, stdout: stdout.to_owned(), stderr: String::new() }));
        self
    }

    pub fn called(&self, prefix: &str) -> bool {
        self.calls.lock().unwrap().iter().any(|c| c.display().starts_with(prefix))
    }
}

impl Runner for MockRunner {
    fn run(&self, cmd: &Cmd) -> Output {
        self.calls.lock().unwrap().push(cmd.clone());
        let line = cmd.display();
        self.rules
            .iter()
            .find(|(p, _)| line.starts_with(p.as_str()))
            .map(|(_, o)| o.clone())
            .unwrap_or(Output { status: 127, stdout: String::new(), stderr: format!("mock: no rule for `{line}`") })
    }

    fn run_interactive(&self, cmd: &Cmd) -> i32 {
        self.run(cmd).status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_runner_reports_missing_program() {
        let o = SystemRunner.run(&Cmd::new("definitely-not-a-real-program-smt5"));
        assert_eq!(o.status, -1);
        assert!(!o.stderr.is_empty());
    }

    #[test]
    fn system_runner_env_and_output() {
        let o = SystemRunner.run(&Cmd::new("sh").args(["-c", "printf %s \"$SMT5_X\""]).env("SMT5_X", "hi"));
        assert!(o.ok());
        assert_eq!(o.stdout, "hi");
    }

    #[test]
    fn stream_merges_output_and_keeps_last_progress_state() {
        let mut lines = Vec::new();
        let mut pid = 0;
        let code = stream(
            &Cmd::new("sh").args(["-c", "echo one; printf 'a\\rb\\n' >&2; exit 3"]),
            |l| lines.push(l.to_owned()),
            |p| pid = p,
        );
        assert_eq!(code, 3);
        assert!(pid > 0);
        lines.sort();
        assert_eq!(lines, vec!["b".to_owned(), "one".to_owned()]);
    }

    #[test]
    fn stdin_file_feeds_run_and_stream() {
        let f = std::env::temp_dir().join(format!("smt5-stdin-{}", std::process::id()));
        std::fs::write(&f, "from file\n").unwrap();
        let o = SystemRunner.run(&Cmd::new("cat").stdin_file(&f));
        assert_eq!(o.stdout, "from file\n");
        let mut lines = Vec::new();
        assert_eq!(stream(&Cmd::new("cat").stdin_file(&f), |l| lines.push(l.to_owned()), |_| {}), 0);
        assert_eq!(lines, vec!["from file".to_owned()]);
        assert_eq!(SystemRunner.run(&Cmd::new("cat")).stdout, "", "default stays /dev/null");
        let _ = std::fs::remove_file(f);
    }

    #[test]
    fn stdin_text_feeds_run_and_stream() {
        let o = SystemRunner.run(&Cmd::new("cat").stdin_text("hello ş\n"));
        assert_eq!(o.stdout, "hello ş\n");
        let mut lines = Vec::new();
        assert_eq!(stream(&Cmd::new("cat").stdin_text("a\nb\n"), |l| lines.push(l.to_owned()), |_| {}), 0);
        assert_eq!(lines, vec!["a".to_owned(), "b".to_owned()]);
        let big = "x".repeat(1 << 20);
        let o = SystemRunner.run(&Cmd::new("cat").stdin_text(big.clone()));
        assert_eq!(o.stdout.len(), big.len());
    }

    #[test]
    fn mock_prefix_matching() {
        let m = MockRunner::new().on("docker info", 0, "x86_64\n");
        assert_eq!(m.run(&Cmd::new("docker").args(["info", "--format", "x"])).stdout_trimmed(), "x86_64");
        assert_eq!(m.run(&Cmd::new("colima").arg("list")).status, 127);
        assert!(m.called("docker info"));
    }
}
