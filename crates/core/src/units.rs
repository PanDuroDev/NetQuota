//! Byte units. NetQuota uses decimal units (KB = 1000 bytes) consistently in the
//! UI and in storage; this matches how Windows and ISPs report data usage.

pub const KB: u64 = 1_000;
pub const MB: u64 = 1_000 * KB;
pub const GB: u64 = 1_000 * MB;
pub const TB: u64 = 1_000 * GB;

/// Parse a human quota string such as `8192`, `500MB`, `1.5 GB`, `2 gb`.
pub fn parse_bytes(input: &str) -> Option<u64> {
    let s = input.trim().to_ascii_uppercase();
    if s.is_empty() {
        return None;
    }
    let (number, multiplier) = if let Some(v) = s.strip_suffix("TB") {
        (v, TB)
    } else if let Some(v) = s.strip_suffix("GB") {
        (v, GB)
    } else if let Some(v) = s.strip_suffix("MB") {
        (v, MB)
    } else if let Some(v) = s.strip_suffix("KB") {
        (v, KB)
    } else if let Some(v) = s.strip_suffix("B") {
        (v, 1)
    } else {
        (s.as_str(), 1)
    };
    let value: f64 = number.trim().parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * multiplier as f64).round() as u64)
}

/// Compact, stable formatting used across the UI and the extension.
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units() {
        assert_eq!(parse_bytes("500MB"), Some(500 * MB));
        assert_eq!(parse_bytes(" 1.5 gb "), Some(1_500 * MB));
        assert_eq!(parse_bytes("8192"), Some(8192));
        assert_eq!(parse_bytes("0"), Some(0));
        assert_eq!(parse_bytes("-1"), None);
        assert_eq!(parse_bytes("abc"), None);
    }

    #[test]
    fn formats_bounded_values() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(GB), "1.00 GB");
    }
}
