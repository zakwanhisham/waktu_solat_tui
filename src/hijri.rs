//! Hijri date rendering (Milestone 8).
//!
//! The API returns Hijri dates as ISO-like strings (`"1448-06-11"`). This
//! module renders them in one of three styles:
//!
//! - `malay`   (default) — romanized JAKIM Takwim month names: `21 Syawal`
//! - `numeric` — the status-quo short numeric form: `21/06`
//! - `arabic`  — Arabic month names + Arabic-Indic digits: `٢١ شوال`
//!
//! Terminal bidi is inconsistent across emulators, so Arabic text is always
//! isolated in its own cell/line — never interpolated into LTR sentences.

/// Romanized Hijri month names, Malaysian spelling (index 0 = Muharram).
pub const MONTHS_MALAY: [&str; 12] = [
    "Muharram",
    "Safar",
    "Rabiulawal",
    "Rabiulakhir",
    "Jamadilawal",
    "Jamadilakhir",
    "Rejab",
    "Syaaban",
    "Ramadan",
    "Syawal",
    "Zulkaedah",
    "Zulhijah",
];

/// Arabic Hijri month names (index 0 = Muharram).
pub const MONTHS_ARABIC: [&str; 12] = [
    "محرم",
    "صفر",
    "ربيع الأول",
    "ربيع الآخر",
    "جمادى الأولى",
    "جمادى الآخرة",
    "رجب",
    "شعبان",
    "رمضان",
    "شوال",
    "ذو القعدة",
    "ذو الحجة",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HijriStyle {
    #[default]
    Malay,
    Numeric,
    Arabic,
}

impl HijriStyle {
    /// Parse a config value; unknown values fall back to the default.
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some("numeric") => Self::Numeric,
            Some("arabic") => Self::Arabic,
            _ => Self::Malay,
        }
    }
}

/// Parse an API Hijri string (`"1448-06-11"`) into `(year, month, day)`.
/// `None` for anything malformed.
pub fn parse(hijri: &str) -> Option<(i32, u32, u32)> {
    let mut parts = hijri.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // trailing component not allowed
    }
    if !(1..=12).contains(&month) || !(1..=30).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// Convert ASCII digits to Arabic-Indic digits.
pub fn to_arabic_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '0'..='9' => char::from_u32('٠' as u32 + (c as u32 - '0' as u32)).unwrap_or(c),
            other => other,
        })
        .collect()
}

/// Short form for a table cell, honoring the style.
/// Malformed input falls back to the raw API string.
pub fn format_cell(hijri: &str, style: HijriStyle) -> String {
    let Some((_, month, day)) = parse(hijri) else {
        return hijri.to_string();
    };
    match style {
        HijriStyle::Malay => format!("{day} {}", MONTHS_MALAY[month as usize - 1]),
        HijriStyle::Numeric => format!("{day:02}/{:02}", month),
        HijriStyle::Arabic => format!("{} {}", to_arabic_digits(&day.to_string()), MONTHS_ARABIC[month as usize - 1]),
    }
}

/// Long form for headers/CLI, honoring the style.
pub fn format_full(hijri: &str, style: HijriStyle) -> String {
    let Some((year, month, _day)) = parse(hijri) else {
        return hijri.to_string();
    };
    match style {
        HijriStyle::Malay => format!("{} {} {}", _day, MONTHS_MALAY[month as usize - 1], year),
        HijriStyle::Numeric => hijri.to_string(),
        HijriStyle::Arabic => format!(
            "{} {} {}",
            to_arabic_digits(&_day.to_string()),
            MONTHS_ARABIC[month as usize - 1],
            to_arabic_digits(&year.to_string())
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_strings() {
        assert_eq!(parse("1448-06-11"), Some((1448, 6, 11)));
        assert_eq!(parse("1448-01-01"), Some((1448, 1, 1)));
        assert_eq!(parse("1448-12-30"), Some((1448, 12, 30)));
    }

    #[test]
    fn rejects_malformed_strings() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("garbage"), None);
        assert_eq!(parse("1448-13-01"), None); // month out of range
        assert_eq!(parse("1448-06-31"), None); // day out of range
        assert_eq!(parse("1448-06"), None); // missing day
        assert_eq!(parse("1448-06-11-x"), None); // trailing component
    }

    #[test]
    fn malay_style_uses_month_names() {
        assert_eq!(format_cell("1448-10-21", HijriStyle::Malay), "21 Syawal");
        assert_eq!(format_cell("1448-01-01", HijriStyle::Malay), "1 Muharram");
        assert_eq!(format_full("1448-10-21", HijriStyle::Malay), "21 Syawal 1448");
    }

    #[test]
    fn numeric_style_is_status_quo() {
        assert_eq!(format_cell("1448-10-21", HijriStyle::Numeric), "21/10");
        assert_eq!(format_full("1448-06-11", HijriStyle::Numeric), "1448-06-11");
    }

    #[test]
    fn arabic_style_uses_arabic_names_and_digits() {
        assert_eq!(format_cell("1448-10-21", HijriStyle::Arabic), "٢١ شوال");
        assert_eq!(format_full("1448-10-21", HijriStyle::Arabic), "٢١ شوال ١٤٤٨");
    }

    #[test]
    fn arabic_digit_conversion() {
        assert_eq!(to_arabic_digits("1448"), "١٤٤٨");
        assert_eq!(to_arabic_digits("21"), "٢١");
        assert_eq!(to_arabic_digits(""), "");
        assert_eq!(to_arabic_digits("a1b2"), "a١b٢"); // non-digits preserved
    }

    #[test]
    fn malformed_input_falls_back_to_raw() {
        assert_eq!(format_cell("???", HijriStyle::Malay), "???");
        assert_eq!(format_full("???", HijriStyle::Arabic), "???");
    }

    #[test]
    fn config_parsing() {
        assert_eq!(HijriStyle::from_config(None), HijriStyle::Malay);
        assert_eq!(HijriStyle::from_config(Some("malay")), HijriStyle::Malay);
        assert_eq!(HijriStyle::from_config(Some("numeric")), HijriStyle::Numeric);
        assert_eq!(HijriStyle::from_config(Some("arabic")), HijriStyle::Arabic);
        assert_eq!(HijriStyle::from_config(Some("bogus")), HijriStyle::Malay);
    }
}
