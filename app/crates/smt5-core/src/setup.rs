//! First-run configuration shared by `smt5 init` and the setup wizard.

use std::io;

use serde::Serialize;

use crate::runtime::RuntimeDir;
use crate::util::random_hex;
use crate::EnvFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InitOutcome {
    Created,
    Updated,
    Unchanged,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitReport {
    pub outcome: InitOutcome,
    pub path: String,
    /// Keys that were generated this time.
    pub generated: Vec<String>,
    /// The new desktop password, only when it was generated now.
    pub password: Option<String>,
}

/// Create `runtime/.env` from `.env.example` if missing, then fill an empty
/// desktop password and API token with random values. `overrides` are written
/// as given (e.g. a value chosen in the wizard).
pub fn init(runtime: &RuntimeDir, overrides: &[(&str, &str)]) -> io::Result<InitReport> {
    let path = runtime.env_path();
    let existed = path.exists();
    let mut env = if existed { EnvFile::load(&path)? } else { EnvFile::load(&runtime.env_example_path())? };
    let mut generated = Vec::new();
    let mut changed = false;
    let mut password = None;
    for (k, v) in overrides {
        if env.get(k) != Some(*v) {
            env.set(k, v);
            changed = true;
        }
    }
    if env.get_nonempty("SMT5_WEB_PASSWORD").is_none() {
        let p = random_hex(9)?;
        env.set("SMT5_WEB_PASSWORD", &p);
        password = Some(p);
        generated.push("SMT5_WEB_PASSWORD".to_owned());
    }
    if env.get_nonempty("SMT5_API_TOKEN").is_none() {
        env.set("SMT5_API_TOKEN", &random_hex(24)?);
        generated.push("SMT5_API_TOKEN".to_owned());
    }
    changed |= !generated.is_empty();
    if !existed || changed {
        env.save(&path)?;
    }
    let outcome = if !existed {
        InitOutcome::Created
    } else if changed {
        InitOutcome::Updated
    } else {
        InitOutcome::Unchanged
    };
    Ok(InitReport { outcome, path: path.display().to_string(), generated, password })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_runtime(tag: &str) -> RuntimeDir {
        let dir = std::env::temp_dir().join(format!("smt5-setup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env.example"), "# example\nSMT5_DISPLAY=kasm\nSMT5_WEB_PASSWORD=\nSMT5_API_TOKEN=\n").unwrap();
        RuntimeDir::new(dir)
    }

    #[test]
    fn creates_then_unchanged_then_override() {
        let rt = tmp_runtime("a");
        let r = init(&rt, &[]).unwrap();
        assert_eq!(r.outcome, InitOutcome::Created);
        assert_eq!(r.generated.len(), 2);
        assert_eq!(r.password.as_ref().map(String::len), Some(18));
        let r = init(&rt, &[]).unwrap();
        assert_eq!(r.outcome, InitOutcome::Unchanged);
        assert!(r.password.is_none());
        let r = init(&rt, &[("SMT5_DISPLAY", "novnc")]).unwrap();
        assert_eq!(r.outcome, InitOutcome::Updated);
        assert_eq!(rt.load_env().get("SMT5_DISPLAY"), Some("novnc"));
        let _ = std::fs::remove_dir_all(&rt.root);
    }
}
