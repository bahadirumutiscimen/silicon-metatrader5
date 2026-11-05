//! Small helpers shared by the CLI and the app.

use std::fs::File;
use std::io::Read;

/// Human readable size ("419.2 MB").
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

/// `bytes` random bytes as lowercase hex, from the OS (macOS and Linux).
pub fn random_hex(bytes: usize) -> std::io::Result<String> {
    let mut buf = vec![0u8; bytes];
    File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Minutes and seconds, "3 dk" style used by the CLI ("12 min", "2 h 5 min").
pub fn human_duration(secs: f64) -> String {
    let m = (secs.max(0.0) / 60.0).floor() as u64;
    if m < 120 {
        format!("{m} min")
    } else {
        format!("{} h {} min", m / 60, m % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(439_538_073), "419.2 MB");
    }

    #[test]
    fn hex() {
        let a = random_hex(24).unwrap();
        assert_eq!(a.len(), 48);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, random_hex(24).unwrap());
    }

    #[test]
    fn durations() {
        assert_eq!(human_duration(59.0), "0 min");
        assert_eq!(human_duration(6.0 * 60.0), "6 min");
        assert_eq!(human_duration(391.0 * 60.0), "6 h 31 min");
    }
}
