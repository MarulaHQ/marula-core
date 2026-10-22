//! First-party Marula reference plugin (Wave 34 lane 4, §4.6).
//!
//! The field validator this plugin registers lives in `manifest.json`
//! (`surface_profile.field_validators`): a JSON-Schema check on any custom field
//! wired to it. This module is the WASM entry (ADR 0023) and answers with a
//! verdict the core turns into a field-level error.
//!
//! The schema is `{"value": string, 6..=32}`. Extracting one string field is
//! twenty lines of byte scanning; pulling in a JSON parser for it would make the
//! reference plugin heavier than the runtime it demonstrates.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use marula_pdk::host;

const MIN_LEN: usize = 6;
const MAX_LEN: usize = 32;

/// The value of the first `"key"` string property, unescaped only far enough to
/// count it. `None` when the key is absent or its value is not a string.
fn string_field<'a>(input: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut at = 0;
    while let Some(offset) = input[at..].windows(key.len()).position(|w| w == key) {
        let mut cursor = at + offset + key.len();
        while input
            .get(cursor)
            .copied()
            .is_some_and(|byte| byte.is_ascii_whitespace() || byte == b':')
        {
            cursor += 1;
        }
        if input.get(cursor) != Some(&b'"') {
            at = cursor;
            continue;
        }
        let start = cursor + 1;
        let mut end = start;
        while let Some(&byte) = input.get(end) {
            match byte {
                b'"' => return Some(&input[start..end]),
                b'\\' => end += 2,
                _ => end += 1,
            }
        }
        return None;
    }
    None
}

fn handle(input: &[u8]) -> Vec<u8> {
    let verdict = match string_field(input, b"\"value\"") {
        Some(value) if value.len() >= MIN_LEN && value.len() <= MAX_LEN => None,
        Some(value) => Some(format!(
            "asset_tag must be {} to {} bytes, got {}",
            MIN_LEN,
            MAX_LEN,
            value.len()
        )),
        None => Some("asset_tag must be a string".into()),
    };
    match &verdict {
        Some(reason) => host::log(host::Level::Debug, reason),
        None => host::log(host::Level::Debug, "asset-tag-validator: accepted"),
    }
    match verdict {
        Some(reason) => format!("{{\"valid\":false,\"reason\":\"{reason}\"}}").into_bytes(),
        None => b"{\"valid\":true}".to_vec(),
    }
}

marula_pdk::plugin!(handle);
