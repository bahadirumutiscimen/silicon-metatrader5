//! UI language of the Rust side (job titles, errors, tray menu). The web UI
//! has its own dictionary (`ui/js/i18n.js`); both follow AppConfig.lang.

use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

pub fn set(lang: &str) {
    EN.store(lang == "en", Ordering::Relaxed);
}

pub fn is_en() -> bool {
    EN.load(Ordering::Relaxed)
}

/// `tr` or `en` for the current language.
pub fn l(tr: &'static str, en: &'static str) -> &'static str {
    if is_en() {
        en
    } else {
        tr
    }
}

/// `format!` with a Turkish and an English template.
#[macro_export]
macro_rules! lf {
    ($tr:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if $crate::i18n::is_en() { format!($en $(, $a)*) } else { format!($tr $(, $a)*) }
    };
}

/// Default language for a new install: the system locale (LANG / AppleLanguages).
pub fn system_default() -> &'static str {
    let from_env = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX"));
    let tag = from_env.or_else(|| {
        if cfg!(target_os = "macos") {
            std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            None
        }
    });
    match tag {
        Some(t) if t.to_lowercase().replace(['(', '"', '\n', ' '], "").trim_start_matches(',').starts_with("tr") => "tr",
        Some(_) => "en",
        None => "tr",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch() {
        set("en");
        assert_eq!(l("Hazır", "Ready"), "Ready");
        let n = 3;
        assert_eq!(lf!("çıkış {n}", "exit {n}"), "exit 3");
        set("tr");
        assert_eq!(lf!("çıkış {}", "exit {}", n), "çıkış 3");
    }
}
