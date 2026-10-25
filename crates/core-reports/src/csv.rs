//! The one CSV cell writer both report surfaces encode with.
//!
//! Purpose: a report carries customer-supplied strings (client names, ticket
//! titles, group labels), and a cell that a spreadsheet evaluates is a code
//! execution channel, not a data delivery. Every CSV surface — the interactive
//! `?format=csv` lane in core-api and the scheduled/emailed lane in the worker
//! — encodes through this module, so a value cannot be neutralised on one path
//! and evaluated on the other.
//!
//! Two steps, in this order: a cell whose first visible character is a formula
//! introducer takes a leading apostrophe, then RFC 4180 quoting runs over the
//! result. The prefix therefore lands inside the quotes (`=A1,B1` becomes
//! `"'=A1,B1"`), and a reader that honours the text marker shows the value as
//! literal text.
//!
//! Non-purpose: no table or document assembly — row order, header emission and
//! the trailing newline belong to the caller that owns its response shape
//! (core-api's `csv_response`, the worker's delivered message). This module
//! owns the cell and nothing above it.

use serde_json::Value;

/// Neutralises a spreadsheet formula, then quotes per RFC 4180.
///
/// The check runs on the trimmed value, so a leading tab or space cannot hide
/// the introducer (`\t=1+1` is the same attack as `=1+1`, with the whitespace
/// only there to slip past a naive first-byte check). The prefix goes in front
/// of that whitespace, so the reader sees text before anything else.
pub fn cell(s: &str) -> String {
    let prefixed;
    let body: &str = if needs_text_prefix(s) {
        prefixed = format!("'{s}");
        &prefixed
    } else {
        s
    };
    if body.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", body.replace('"', "\"\""))
    } else {
        body.to_string()
    }
}

/// A cell from a JSON row value.
///
/// Absent and null render empty. A JSON string takes the full cell contract;
/// a number or boolean is a value the report engine produced, never a
/// customer-supplied label, so it is encoded as itself and keeps its sign
/// (`-120` stays a number rather than becoming text).
pub fn cell_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => cell(s),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => cell(&other.to_string()),
    }
}

/// Whether a spreadsheet would evaluate `s` as a formula.
fn needs_text_prefix(s: &str) -> bool {
    let candidate = s.trim();
    let Some(first) = candidate.chars().next() else {
        return false;
    };
    if !matches!(first, '=' | '+' | '-' | '@') {
        return false;
    }
    // Neither a metric nor a date is a formula: prefixing one would silently
    // turn a numeric or date column into text.
    !(is_plain_number(candidate) || is_iso_date(candidate))
}

/// `-1`, `-120.5`, `-1.5e-3` stay numbers. A leading `+` never qualifies —
/// spreadsheets read `+1` as a formula, not as the number 1.
fn is_plain_number(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    !mantissa.is_empty()
        && mantissa.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && mantissa.bytes().filter(|b| *b == b'.').count() <= 1
        && mantissa.bytes().any(|b| b.is_ascii_digit())
}

/// `YYYY-MM-DD`, optionally with a time and offset (`2026-10-22T08:30:00Z`).
///
/// An ISO date can never begin with an introducer, so this half of the
/// exemption is structural today; stating it keeps a future, wider introducer
/// set from quietly mangling date columns.
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    let date = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit);
    if !date {
        return false;
    }
    let rest = &b[10..];
    if rest.is_empty() {
        return true;
    }
    if rest.len() < 9 || rest[0] != b'T' {
        return false;
    }
    // HH:MM:SS, then any fraction, then Z or ±HH:MM.
    let clock = &rest[1..];
    let clock_ok = clock.len() >= 8
        && clock[..2].iter().all(u8::is_ascii_digit)
        && clock[2] == b':'
        && clock[3..5].iter().all(u8::is_ascii_digit)
        && clock[5] == b':'
        && clock[6..8].iter().all(u8::is_ascii_digit);
    if !clock_ok {
        return false;
    }
    let tail = &clock[8..];
    let tail = match tail.first() {
        None => return true,
        Some(b'.') => {
            let digits = tail[1..].iter().take_while(|b| b.is_ascii_digit()).count();
            if digits == 0 {
                return false;
            }
            &tail[1 + digits..]
        }
        Some(_) => tail,
    };
    match tail.first() {
        None => true,
        Some(b'Z') => tail.len() == 1,
        Some(b'+') | Some(b'-') => {
            tail.len() == 6
                && tail[1..3].iter().all(u8::is_ascii_digit)
                && tail[3] == b':'
                && tail[4..6].iter().all(u8::is_ascii_digit)
        }
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One row per rule in the escaping contract: the four formula
    /// introducers, the whitespace that tries to hide them, RFC 4180
    /// quoting, and the numbers and dates that must survive untouched.
    #[test]
    fn csv_cells_neutralise_formulas_but_keep_numbers_and_dates() {
        let cases: &[(&str, &str)] = &[
            // Formula introducers take a text prefix.
            ("=1+1", "'=1+1"),
            ("+1", "'+1"),
            ("-1+1", "'-1+1"),
            ("@SUM(A1)", "'@SUM(A1)"),
            ("=", "'="),
            (
                "=HYPERLINK(\"http://x\",\"y\")",
                "\"'=HYPERLINK(\"\"http://x\"\",\"\"y\"\")\"",
            ),
            // Leading whitespace is not a hiding place; the prefix lands
            // before it so the reader sees text first.
            ("\t=1+1", "'\t=1+1"),
            (" =1+1", "' =1+1"),
            (" \t-2+3", "' \t-2+3"),
            // RFC 4180 quoting still runs, after the prefix.
            ("=A1,B1", "\"'=A1,B1\""),
            ("report, \"final\"", "\"report, \"\"final\"\"\""),
            ("line\nbreak", "\"line\nbreak\""),
            // Numbers and dates stay numeric: they are data, not formulas.
            ("-1", "-1"),
            ("-120", "-120"),
            ("-120.5", "-120.5"),
            ("-1.5e-3", "-1.5e-3"),
            ("- 1", "'- 1"),
            ("-inf", "'-inf"),
            ("--1", "'--1"),
            ("2026-10-22", "2026-10-22"),
            ("2026-10-22T08:30:00Z", "2026-10-22T08:30:00Z"),
            (
                "2026-10-22T08:30:00.250+02:00",
                "2026-10-22T08:30:00.250+02:00",
            ),
            // Ordinary values are left alone.
            ("Acme", "Acme"),
            ("Acme, Inc", "\"Acme, Inc\""),
            ("", ""),
            ("  ", "  "),
        ];
        for (input, expected) in cases {
            assert_eq!(cell(input), *expected, "input {input:?}");
        }
    }

    /// The JSON layer renders metrics as numbers, so a negative metric never
    /// reaches the string path and keeps its leading minus, while a label in
    /// the same row is neutralised.
    #[test]
    fn json_strings_take_the_cell_contract_and_numbers_do_not() {
        assert_eq!(cell_value(Some(&json!("=1+1"))), "'=1+1");
        assert_eq!(cell_value(Some(&json!(-120))), "-120");
        assert_eq!(cell_value(Some(&json!(true))), "true");
        assert_eq!(cell_value(Some(&json!(null))), "");
        assert_eq!(cell_value(None), "");
    }
}
