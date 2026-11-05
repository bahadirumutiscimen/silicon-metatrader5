//! `.env` files: read and update values while keeping comments and order.

use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvFile {
    lines: Vec<String>,
}

fn split(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    if t.starts_with('#') {
        return None;
    }
    let (k, v) = t.split_once('=')?;
    let k = k.trim();
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((k, v))
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

impl EnvFile {
    pub fn parse(text: &str) -> Self {
        Self { lines: text.lines().map(str::to_owned).collect() }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self::parse(&fs::read_to_string(path)?))
    }

    /// Like [`load`](Self::load) but an absent file is an empty one.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// Value of the last `KEY=` line (compose semantics), unquoted. Empty values are `Some("")`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|l| match split(l) {
            Some((k, v)) if k == key => Some(unquote(v)),
            _ => None,
        })
    }

    /// Non-empty value or `None`.
    pub fn get_nonempty(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|v| !v.is_empty())
    }

    /// Replace every `KEY=` line (keeping its position) or append a new one.
    pub fn set(&mut self, key: &str, value: &str) {
        let mut found = false;
        for l in &mut self.lines {
            if matches!(split(l), Some((k, _)) if k == key) {
                *l = format!("{key}={value}");
                found = true;
            }
        }
        if !found {
            self.lines.push(format!("{key}={value}"));
        }
    }

    pub fn to_text(&self) -> String {
        let mut s = self.lines.join("\n");
        s.push('\n');
        s
    }

    /// Write atomically (temp file + rename) with mode 0600 on Unix.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("env.tmp");
        fs::write(&tmp, self.to_text())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# comment\nSMT5_DISPLAY=kasm\nSMT5_WEB_PASSWORD=\n  TZ = Europe/Istanbul\nQUOTED=\"a b\"\nSMT5_DISPLAY=novnc\n";

    #[test]
    fn get_semantics() {
        let e = EnvFile::parse(SAMPLE);
        assert_eq!(e.get("SMT5_DISPLAY"), Some("novnc")); // last one wins
        assert_eq!(e.get("SMT5_WEB_PASSWORD"), Some(""));
        assert_eq!(e.get_nonempty("SMT5_WEB_PASSWORD"), None);
        assert_eq!(e.get("TZ"), Some("Europe/Istanbul"));
        assert_eq!(e.get("QUOTED"), Some("a b"));
        assert_eq!(e.get("MISSING"), None);
        assert_eq!(e.get("# comment"), None);
    }

    #[test]
    fn set_keeps_comments_and_order() {
        let mut e = EnvFile::parse(SAMPLE);
        e.set("SMT5_WEB_PASSWORD", "s3cret");
        e.set("NEW_KEY", "1");
        let text = e.to_text();
        assert!(text.starts_with("# comment\n"));
        assert!(text.contains("\nSMT5_WEB_PASSWORD=s3cret\n"));
        assert!(text.ends_with("NEW_KEY=1\n"));
        assert_eq!(EnvFile::parse(&text).get("SMT5_WEB_PASSWORD"), Some("s3cret"));
    }

    #[test]
    fn save_roundtrip() {
        let dir = std::env::temp_dir().join(format!("smt5-env-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(".env");
        let mut e = EnvFile::parse(SAMPLE);
        e.set("SMT5_API_TOKEN", "abc");
        e.save(&p).unwrap();
        assert_eq!(EnvFile::load(&p).unwrap().get("SMT5_API_TOKEN"), Some("abc"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
