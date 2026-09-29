//! Deterministic label rendering (Wave 32 lane 5, `p32-labels-scanning`).
//!
//! ONE layout model feeds two print targets: the browser print dialog (the
//! web app draws [`ResolvedLabel`] at exact millimetre geometry) and raw ZPL
//! for networked Zebra printers ([`to_zpl`]). Determinism is the contract:
//! the same template + the same resolved token values always produce the
//! same element positions, the same truncated strings and the same ZPL
//! bytes — so a printed label and its on-screen preview cannot disagree,
//! and the unit tests can pin the exact output.
//!
//! Geometry rules (fixed, documented once):
//!   * padding is 1.5 mm on every edge;
//!   * the QR (when enabled) reserves a square at the TOP-RIGHT, clamped to
//!     6–14 mm, at most 45 % of the content width, and small enough that the
//!     barcode band below it still fits;
//!   * the Code128 band (when a barcode token is set) spans the FULL content
//!     width under both the text block and the QR — the two symbologies
//!     never overlap (an overlapped barcode is an unscannable barcode),
//!     clamped to 4–8 mm and 28 % of the label height;
//!   * text lines split the remaining height evenly, clamped to 2.4–4.6 mm,
//!     and overflow lines are DROPPED from the end (reported, never silent);
//!   * text FITS FIRST: a line that would not fit at its nominal size shrinks
//!     its font (down to 45 % of the line box) before anything is cut, so an
//!     identifier like a 16-character asset tag prints whole;
//!   * only when even the smallest size cannot fit does the text truncate —
//!     and the line carries `truncated = true` so the UI can say so. The
//!     character budget uses the average glyph advance 0.58 × FONT size
//!     (not the line box), which is what a rendered Inter line actually
//!     measures at these sizes.
//!
//! ZPL notes: `^CI28` (UTF-8), `^FH` hex escaping for every byte that ZPL
//! reserves, `^A0N` scalable font, `^BY2,2` Code128 (module width 2 dots),
//! `^BQN,2,<magnification>` QR (error correction L, automatic input mode).
//! QR magnification is sized from a 37-module worst case (a ~60-character
//! deep link) so the symbol fits the reserved square.

use crate::entities::LabelTemplate;

const PADDING_MM: f64 = 1.5;
const BARCODE_GAP_MM: f64 = 0.8;
const QR_MIN_MM: f64 = 6.0;
const QR_MAX_MM: f64 = 14.0;
const BARCODE_MIN_MM: f64 = 4.0;
const BARCODE_MAX_MM: f64 = 8.0;
const LINE_MIN_MM: f64 = 2.4;
const LINE_MAX_MM: f64 = 4.6;
/// Average glyph advance as a fraction of the FONT size (deterministic
/// approximation of rendered Inter at label sizes — the same number drives
/// preview, print and ZPL truncation).
const GLYPH_FACTOR: f64 = 0.58;
/// Nominal font size as a fraction of the line box.
const FONT_FACTOR: f64 = 0.72;
/// Floor a line's font may shrink to before truncation kicks in.
const FONT_FLOOR_FACTOR: f64 = 0.45;
/// Font shrink step (mm) — keeps the search deterministic and coarse enough
/// that the ZPL never jitters on rounding.
const FONT_STEP_MM: f64 = 0.1;

/// Everything the renderer needs about the entity being labelled.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelInput {
    /// Resolved `(token, text)` pairs, in template order. Tokens absent from
    /// this list render as an empty line (honest blank, never a placeholder).
    pub values: Vec<(String, String)>,
    /// The QR payload — the entity's canonical deep link.
    pub qr_data: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LabelLine {
    pub token: String,
    pub text: String,
    pub y_mm: f64,
    /// The line box height (spacing).
    pub height_mm: f64,
    /// The effective font size for this line: `0.72 × height_mm` when the
    /// text fits, shrunk (floor `0.45 × height_mm`) until it does. The ZPL
    /// `^A0N` height and the browser preview both use THIS number, so the
    /// print and the preview can never disagree on how big the text is.
    pub font_mm: f64,
    /// True when even the smallest font could not fit the text and the tail
    /// was cut — surfaced in the UI, never silent.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LabelBarcode {
    pub token: String,
    pub data: String,
    pub y_mm: f64,
    pub height_mm: f64,
    /// Band width: the FULL content width (the QR sits above it, never over it).
    pub width_mm: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LabelQr {
    pub data: String,
    pub y_mm: f64,
    pub size_mm: f64,
}

/// The deterministic rendered label: absolute geometry in millimetres.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResolvedLabel {
    pub purpose: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi: i32,
    pub lines: Vec<LabelLine>,
    pub barcode: Option<LabelBarcode>,
    pub qr: Option<LabelQr>,
    /// Lines dropped because they could not fit at the minimum line height.
    pub dropped_lines: usize,
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// Truncates to `max_chars` characters (deterministic, no ellipsis so the
/// preview and the ZPL field carry the identical bytes).
fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// How many characters fit on a line of width `text_w` at font size `font_mm`.
fn char_budget(text_w: f64, font_mm: f64) -> usize {
    if font_mm <= 0.0 {
        return 4;
    }
    ((text_w / (GLYPH_FACTOR * font_mm)).floor() as usize).max(4)
}

/// Fits `text` into `text_w`: keeps the nominal font when it fits, shrinks
/// the font by [`FONT_STEP_MM`] down to the floor when it does not, and only
/// then truncates (reporting it). Returns `(effective_font_mm, text, truncated)`.
fn fit_line(text: &str, text_w: f64, line_h_mm: f64) -> (f64, String, bool) {
    let nominal = round1(line_h_mm * FONT_FACTOR);
    let floor = round1(line_h_mm * FONT_FLOOR_FACTOR);
    let chars = text.chars().count();
    let mut font = nominal;
    while font > floor && chars > char_budget(text_w, font) {
        font = round1((font - FONT_STEP_MM).max(floor));
    }
    let budget = char_budget(text_w, font);
    if chars > budget {
        (font, truncate_chars(text, budget), true)
    } else {
        (font, text.to_string(), false)
    }
}

/// Renders the template against resolved entity values.
pub fn render_label(template: &LabelTemplate, input: &LabelInput) -> ResolvedLabel {
    let (w10, h10) = crate::entities::size_tenths_mm(template).unwrap_or((508, 254));
    let w = w10 as f64 / 10.0;
    let h = h10 as f64 / 10.0;
    let content_w = w - 2.0 * PADDING_MM;
    let content_h = h - 2.0 * PADDING_MM;

    // QR reservation (right edge, bottom-aligned).
    let qr_size = if template.include_qr {
        round1(clamp(content_h.min(content_w * 0.45), QR_MIN_MM, QR_MAX_MM))
    } else {
        0.0
    };
    let text_w = content_w - if qr_size > 0.0 { qr_size + 1.0 } else { 0.0 };

    // Code128 band (under the text).
    let barcode_data = template
        .barcode_token
        .as_ref()
        .and_then(|token| {
            input
                .values
                .iter()
                .find(|(t, _)| t == token)
                .map(|(_, text)| text.clone())
        })
        .filter(|text| !text.trim().is_empty());
    let barcode_h = if barcode_data.is_some() {
        round1(clamp(h * 0.28, BARCODE_MIN_MM, BARCODE_MAX_MM))
    } else {
        0.0
    };
    let lines_area = content_h - barcode_h - if barcode_h > 0.0 { BARCODE_GAP_MM } else { 0.0 };

    // Text lines: drop from the end when the label cannot fit them all.
    let mut tokens: Vec<&String> = template.fields.iter().collect();
    let mut dropped_lines = 0usize;
    if !tokens.is_empty() && lines_area > 0.0 {
        let fits = (lines_area / LINE_MIN_MM).floor() as usize;
        if tokens.len() > fits {
            dropped_lines = tokens.len() - fits;
            tokens.truncate(fits);
        }
    } else if !tokens.is_empty() {
        dropped_lines = tokens.len();
        tokens.clear();
    }
    let n = tokens.len();
    let line_h = if n == 0 {
        0.0
    } else {
        round1(clamp(lines_area / n as f64, LINE_MIN_MM, LINE_MAX_MM))
    };
    let lines: Vec<LabelLine> = tokens
        .iter()
        .enumerate()
        .map(|(i, token)| {
            let raw = input
                .values
                .iter()
                .find(|(t, _)| t == *token)
                .map(|(_, text)| text.as_str())
                .unwrap_or("");
            let (font_mm, text, truncated) = if line_h > 0.0 {
                fit_line(raw, text_w, line_h)
            } else {
                (0.0, String::new(), false)
            };
            LabelLine {
                token: (*token).clone(),
                text,
                y_mm: round1(PADDING_MM + i as f64 * line_h),
                height_mm: line_h,
                font_mm,
                truncated,
            }
        })
        .collect();

    let content_top = PADDING_MM + (n as f64 * line_h).max(qr_size);
    let barcode = barcode_data.map(|data| LabelBarcode {
        token: template.barcode_token.clone().unwrap_or_default(),
        data,
        y_mm: round1(
            (content_top
                + if n > 0 || qr_size > 0.0 {
                    BARCODE_GAP_MM
                } else {
                    0.0
                })
            .min((h - PADDING_MM - barcode_h).max(PADDING_MM)),
        ),
        height_mm: barcode_h,
        width_mm: round1(content_w),
    });

    let qr = if qr_size > 0.0 {
        Some(LabelQr {
            data: input.qr_data.clone(),
            y_mm: round1(PADDING_MM),
            size_mm: qr_size,
        })
    } else {
        None
    };

    ResolvedLabel {
        purpose: template.purpose.clone(),
        width_mm: round1(w),
        height_mm: round1(h),
        dpi: template.dpi,
        lines,
        barcode,
        qr,
        dropped_lines,
    }
}

// ---------------------------------------------------------------------------
// ZPL
// ---------------------------------------------------------------------------

fn mm_to_dots(mm: f64, dpi: i32) -> i32 {
    ((mm / 25.4) * dpi as f64).round() as i32
}

/// Escapes field data for `^FH` mode: every byte ZPL reserves (`^`, `~`,
/// `_`) and every control byte becomes `_XX` hex; UTF-8 bytes ≥ 0x80 pass
/// through unchanged under `^CI28`.
pub fn zpl_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b == b'^' || b == b'~' || b == b'_' || b < 0x20 {
            out.push_str(&format!("_{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

fn font_dots(font_mm: f64, dpi: i32) -> (i32, i32) {
    let h = ((font_mm / 25.4) * dpi as f64).round() as i32;
    let h = h.max(8);
    (h, (h as f64 * 0.6).round().max(4.0) as i32)
}

/// Renders ZPL for a resolved label. Byte-deterministic.
pub fn to_zpl(label: &ResolvedLabel) -> String {
    let dpi = label.dpi;
    let mut out = String::new();
    out.push_str("^XA\n^CI28\n");
    out.push_str(&format!("^PW{}\n", mm_to_dots(label.width_mm, dpi)));
    out.push_str(&format!("^LL{}\n", mm_to_dots(label.height_mm, dpi)));
    for line in &label.lines {
        let (fh, fw) = font_dots(line.font_mm, dpi);
        out.push_str(&format!(
            "^FO{},{}\n^A0N,{},{}\n^FH^FD{}^FS\n",
            mm_to_dots(PADDING_MM, dpi),
            mm_to_dots(line.y_mm, dpi),
            fh,
            fw,
            zpl_escape(&line.text)
        ));
    }
    if let Some(barcode) = &label.barcode {
        let h_dots = mm_to_dots(barcode.height_mm, dpi).max(20);
        // Module width from the band: Code128 is 11 modules per symbol plus
        // the 13-module stop (11n + 37 for n data symbols), so the symbol
        // fits the band width instead of running past it. Clamped 1–6.
        let modules = 11 * barcode.data.len() as i32 + 37;
        let available = mm_to_dots(barcode.width_mm, dpi).max(40);
        let module = (available / modules.max(1)).clamp(1, 6);
        out.push_str(&format!(
            "^FO{},{}\n^BY{},2,{}\n^BCN,{},N,N,N\n^FH^FD{}^FS\n",
            mm_to_dots(PADDING_MM, dpi),
            mm_to_dots(barcode.y_mm, dpi),
            module,
            h_dots,
            h_dots,
            zpl_escape(&barcode.data)
        ));
    }
    if let Some(qr) = &label.qr {
        let size_dots = mm_to_dots(qr.size_mm, dpi);
        // Worst-case module count for a ~60-char deep link at ECC L.
        let magnification = clamp((size_dots / 37) as f64, 2.0, 10.0) as i32;
        let x = mm_to_dots(label.width_mm - PADDING_MM - qr.size_mm, dpi).max(0);
        out.push_str(&format!(
            "^FO{},{}^BQN,2,{}\n^FH^FDLA,{}^FS\n",
            x,
            mm_to_dots(qr.y_mm, dpi),
            magnification,
            zpl_escape(&qr.data)
        ));
    }
    out.push_str("^XZ\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::LabelTemplate;
    use uuid::Uuid;

    fn template() -> LabelTemplate {
        LabelTemplate {
            id: Uuid::nil(),
            org_id: Uuid::nil(),
            name: "Unit".into(),
            purpose: "serialized_unit".into(),
            size_preset: "zebra_2x1".into(),
            width_tenths_mm: None,
            height_tenths_mm: None,
            dpi: 203,
            barcode_token: Some("serial".into()),
            include_qr: true,
            fields: vec!["name".into(), "serial".into(), "location".into()],
            is_default: true,
            active: true,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn input() -> LabelInput {
        LabelInput {
            values: vec![
                ("name".into(), "Torque wrench 1/2in".into()),
                ("serial".into(), "TW-000123".into()),
                ("location".into(), "Van 2".into()),
            ],
            qr_data: "marula://stock-serial/00000000-0000-0000-0000-000000000000".into(),
        }
    }

    #[test]
    fn render_is_deterministic_and_geometry_stays_on_the_label() {
        let t = template();
        let a = render_label(&t, &input());
        let b = render_label(&t, &input());
        assert_eq!(a, b, "same inputs render the same label");
        assert_eq!((a.width_mm, a.height_mm), (50.8, 25.4));
        for line in &a.lines {
            assert!(line.y_mm + line.height_mm <= a.height_mm + 0.01, "{line:?}");
        }
        let qr = a.qr.expect("qr on");
        assert!(qr.size_mm >= QR_MIN_MM && qr.size_mm <= QR_MAX_MM);
        assert!(qr.y_mm + qr.size_mm <= a.height_mm + 0.01);
        let bar = a.barcode.expect("barcode on");
        assert_eq!(bar.data, "TW-000123");
        assert!(bar.y_mm + bar.height_mm <= a.height_mm + 0.01);
        // Symbologies never overlap: an overlapped barcode does not scan.
        let qr_left = a.width_mm - PADDING_MM - qr.size_mm;
        let qr_bottom = qr.y_mm + qr.size_mm;
        assert!(qr_bottom <= bar.y_mm + 0.01, "qr {qr:?} clears bar {bar:?}");
        assert!(
            bar.width_mm <= a.width_mm - 2.0 * PADDING_MM + 0.01,
            "band stays inside the label: {bar:?}"
        );
        assert!(qr_left > PADDING_MM);
    }

    #[test]
    fn an_identifier_fits_whole_instead_of_truncating() {
        // D1 regression (vision review): a 16-character asset tag on a
        // DYMO multi-purpose label must print WHOLE — the font shrinks
        // rather than cutting the value a technician reads.
        let mut t = template();
        t.purpose = "device_asset".into();
        t.fields = vec!["name".into(), "tag".into()];
        t.barcode_token = Some("tag".into());
        t.size_preset = "dymo_multipurpose".into();
        t.width_tenths_mm = None;
        t.height_tenths_mm = None;
        let mut i = input();
        i.values = vec![
            ("name".into(), "Review loan device".into()),
            ("tag".into(), "RVW-ASSET-832448".into()),
        ];
        let a = render_label(&t, &i);
        let tag = a.lines.iter().find(|l| l.token == "tag").expect("tag line");
        assert!(!tag.truncated, "the tag must not be cut: {tag:?}");
        assert_eq!(tag.text, "RVW-ASSET-832448");
        assert!(
            tag.font_mm < tag.height_mm * 0.72,
            "font shrank to fit: {tag:?}"
        );
        assert!(
            tag.font_mm >= tag.height_mm * 0.45,
            "never below the floor: {tag:?}"
        );
    }

    #[test]
    fn absurdly_long_text_truncates_at_the_floor_and_flags_it() {
        let mut t = template();
        t.include_qr = false;
        t.fields = vec!["name".into()];
        let mut i = input();
        i.values = vec![("name".into(), "X".repeat(300))];
        let a = render_label(&t, &i);
        let line = &a.lines[0];
        assert!(line.truncated, "flagged, never silent: {line:?}");
        assert!(line.font_mm <= line.height_mm * 0.72 + 0.001);
        let text_w = 50.8 - 2.0 * PADDING_MM;
        let budget = ((text_w / (GLYPH_FACTOR * line.font_mm)).floor() as usize).max(4);
        assert_eq!(line.text.chars().count(), budget);
    }

    #[test]
    fn overflow_lines_are_dropped_and_reported() {
        let mut t = template();
        t.size_preset = "custom".into();
        t.width_tenths_mm = Some(200);
        t.height_tenths_mm = Some(120); // 12mm tall — cannot fit 3 lines + barcode + qr
        t.fields = vec!["name".into(), "serial".into(), "location".into()];
        let a = render_label(&t, &input());
        assert!(a.dropped_lines > 0, "short label drops lines honestly");
        assert_eq!(a.lines.len() + a.dropped_lines, 3);
    }

    #[test]
    fn zpl_is_well_formed_and_escaped() {
        let mut t = template();
        t.fields = vec!["name".into(), "serial".into()];
        let mut i = input();
        i.values[0] = ("name".into(), "C^_under_".into());
        let label = render_label(&t, &i);
        let zpl = to_zpl(&label);
        assert!(zpl.starts_with("^XA\n^CI28\n"));
        assert!(zpl.ends_with("^XZ\n"));
        assert!(
            zpl.contains("C_5E_5Funder_5F"),
            "reserved bytes hex-escaped: {zpl}"
        );
        assert!(zpl.contains("^BQN,2,"), "qr present");
        assert!(zpl.contains("^BCN,"), "code128 present");
        assert_eq!(zpl.matches("^FD").count(), zpl.matches("^FS").count());
        // Deterministic bytes.
        assert_eq!(zpl, to_zpl(&render_label(&t, &i)));
    }

    #[test]
    fn no_symbologies_when_disabled() {
        let mut t = template();
        t.include_qr = false;
        t.barcode_token = None;
        t.fields = vec!["name".into()];
        let label = render_label(&t, &input());
        assert!(label.qr.is_none());
        assert!(label.barcode.is_none());
        let zpl = to_zpl(&label);
        assert!(!zpl.contains("^BQ"));
        assert!(!zpl.contains("^BC"));
    }

    #[test]
    fn empty_values_render_blank_not_placeholder() {
        let mut t = template();
        t.fields = vec!["sku".into()];
        let i = LabelInput {
            values: vec![],
            qr_data: "marula://stock-item/1".into(),
        };
        let label = render_label(&t, &i);
        assert_eq!(label.lines[0].text, "");
    }
}
