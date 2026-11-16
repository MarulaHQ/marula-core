//! "Describe it" → a quote or contract draft, from words, notes or uploads.
//!
//! Shape: natural-language entry points per record kind, returning the SAME
//! fields the existing create dialogs already carry, so a described record is
//! never a privileged path around the schema. The endpoints persist NOTHING:
//! a person reviews the draft, edits it in the ordinary builder, and saves
//! through the existing create route, which re-validates.
//!
//! The model call rides the shared AI admission path
//! (`ai_policy::authorize_surface_call` + `chat_runner::resolve_transport`)
//! with the `quote_authoring` / `contract_authoring` native surfaces. Those
//! keys are migration-seeded and deny-by-default: no grant means the UI never
//! renders the door and the endpoint refuses with a typed reason.
//!
//! The check step is not optional and not a setting. `understanding` is part of
//! every successful answer: one sentence, the facts the server could match, the
//! questions it cannot decide, and what it assumed. The UI draws that card
//! before the draft is buildable.
//!
//! Provenance is mechanical, not a claim. A line is `notes` only when the model
//! returned the words it came from AND this module found those words in one of
//! the supplied sources; those offsets drive the numbered marks on the notes
//! tab. Everything else is `assumed`, and the server — never the model — sets
//! the assumed flags for a zero price or a catalogue price.
//!
//! Non-purpose: no pricing invention, no totals from the model, no clause text
//! from the model. A line the notes never priced comes back at zero with an
//! assumption that says so; a matched line takes its price from
//! `catalog_items`; a contract draft may only name a clause from the approved
//! library and fill one parameter. Totals are computed here and in the client
//! from quantities and prices, never asked of the model. Nothing is written
//! until the ordinary create route is called.

use axum::{extract::State, http::Extensions, Json};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::routes::ai_policy::{authorize_surface_call, org_bound_tx, AiAdmissionAsk, AiEdge};
use crate::routes::chat_runner::{self, LoopMessage, ModelCallRequest};
use crate::state::AppState;
use core_db::ai_policy::{record_usage, AiUsageNew};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};

/// The migration-seeded native surface keys. One per record kind, so an admin
/// can grant quotes without granting contracts.
pub const QUOTE_SURFACE: &str = "quote_authoring";
pub const CONTRACT_SURFACE: &str = "contract_authoring";

/// A pasted ledger of meeting notes is far larger than a sentence; the bound
/// keeps one source inside a model context and stops a paste accident from
/// becoming a bill.
const MAX_SOURCE_CHARS: usize = 12_000;
/// Combined bound across every source in one request.
const MAX_COMBINED_CHARS: usize = 24_000;
const MAX_SOURCES: usize = 6;
/// An uploaded document the extractor reads is text. The cap is on the decoded
/// bytes, before any parsing, so a zip bomb never reaches a parser.
const MAX_UPLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Questions the check step will put in front of a person. More than this is a
/// form, not a check.
const MAX_QUESTIONS: usize = 3;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

// --- wire types -----------------------------------------------------------

/// One thing the person gave the agent to read.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthorSource {
    /// Client-chosen stable id. The server assigns one when it is absent, and
    /// provenance spans point at it.
    #[serde(default)]
    pub id: Option<String>,
    /// `typed` | `document` | `transcript` | `recording`.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    pub text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthorMoneyRequest {
    /// The words to read. Kept as its own field for the original one-box call.
    #[serde(default)]
    pub transcript: String,
    /// Typed notes, uploaded documents and transcripts, in one list.
    #[serde(default)]
    pub sources: Vec<AuthorSource>,
    /// An optional steer, such as "ignore the part about printers".
    #[serde(default)]
    pub instruction: Option<String>,
    /// A client the person picked in the panel, so the draft never guesses.
    #[serde(default)]
    pub client_id: Option<Uuid>,
}

/// Where a draft line came from. Character offsets into the source text, so the
/// notes tab can highlight exactly those words.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SourceSpan {
    pub source_id: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuoteLineDraft {
    pub description: String,
    /// Quantity × 1000 (the product's milli-unit).
    pub quantity_milli: i64,
    /// Unit price in minor currency units (cents).
    pub unit_price_minor: i64,
    #[serde(default)]
    pub unit: Option<String>,
    /// A section heading, as the pack's item table groups its rows.
    #[serde(default)]
    pub section: Option<String>,
    /// An optional extra: priced and shown, not counted in the total.
    #[serde(default)]
    pub optional: bool,
    /// Present when the line's name matched a catalogue item.
    #[serde(default)]
    pub catalog_item_id: Option<Uuid>,
    /// `notes` | `assumed` | `catalogue`. `notes` always carries a span.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_span: Option<SourceSpan>,
    #[serde(default)]
    pub quantity_assumed: bool,
    #[serde(default)]
    pub price_assumed: bool,
}

/// One fact the draft matched, before the person checks it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MatchedFact {
    pub label: String,
    pub value: String,
}

/// A question the draft cannot decide. `options` are the answers, and `chosen`
/// is the likeliest one, already selected when the card opens.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DraftQuestion {
    pub id: String,
    pub text: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub chosen: Option<String>,
}

/// The check step. Always present on a successful answer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Understanding {
    pub statement: String,
    #[serde(default)]
    pub facts: Vec<MatchedFact>,
    #[serde(default)]
    pub questions: Vec<DraftQuestion>,
    #[serde(default)]
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuoteDraft {
    pub title: String,
    pub currency: String,
    #[serde(default)]
    pub valid_for_days: Option<i64>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub client_name: Option<String>,
    /// The budget the notes mentioned, when they mentioned one. The client
    /// compares it against the computed total; the server never adjusts a
    /// price to fit it.
    #[serde(default)]
    pub budget_minor: Option<i64>,
    #[serde(default)]
    pub lines: Vec<QuoteLineDraft>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub understanding: Option<Understanding>,
    /// The "things to check" summary the draft editor lists.
    #[serde(default)]
    pub checks: Vec<String>,
}

/// A clause selection on a contract draft. The text is the library's, never the
/// model's; only `parameter_value` can differ, and when it does the selection
/// is a variant and needs legal review.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClauseRef {
    pub clause_id: Uuid,
    pub key: String,
    pub title: String,
    pub body: String,
    pub version: i32,
    /// `standard` | `variant`.
    pub status: String,
    pub review_required: bool,
    #[serde(default)]
    pub parameter_label: Option<String>,
    #[serde(default)]
    pub parameter_value: Option<String>,
    #[serde(default)]
    pub standard_value: Option<String>,
}

/// One line of a contract's pricing, from quantities and rates.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractPriceLine {
    pub label: String,
    pub quantity_milli: i64,
    pub unit_price_minor: i64,
    #[serde(default)]
    pub quantity_assumed: bool,
    #[serde(default)]
    pub price_assumed: bool,
    #[serde(default)]
    pub source_span: Option<SourceSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractDraft {
    pub name: String,
    pub contract_type: String,
    #[serde(default)]
    pub billing_frequency: Option<String>,
    #[serde(default)]
    pub value_minor: Option<i64>,
    pub currency: String,
    #[serde(default)]
    pub term_months: Option<i64>,
    #[serde(default)]
    pub effective_from: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Clause titles, kept for the original response shape. These are library
    /// titles, never model-written sentences.
    #[serde(default)]
    pub clauses: Vec<String>,
    #[serde(default)]
    pub clause_refs: Vec<ClauseRef>,
    #[serde(default)]
    pub pricing: Vec<ContractPriceLine>,
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub understanding: Option<Understanding>,
    #[serde(default)]
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MoneyAuthoringIssue {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MoneyAuthorUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MoneyAuthorModel {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorQuoteResponse {
    pub valid: bool,
    pub saved: bool,
    pub draft: Option<QuoteDraft>,
    pub assumptions: Vec<String>,
    pub issues: Vec<MoneyAuthoringIssue>,
    pub usage: MoneyAuthorUsage,
    pub model_info: MoneyAuthorModel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The failure reference a person can quote to support (Q14).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorContractResponse {
    pub valid: bool,
    pub saved: bool,
    pub draft: Option<ContractDraft>,
    pub assumptions: Vec<String>,
    pub issues: Vec<MoneyAuthoringIssue>,
    pub usage: MoneyAuthorUsage,
    pub model_info: MoneyAuthorModel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// What the panel can already see, before anything is committed. A chip is a
/// read of the words, not a promise about the draft.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExtractedField {
    /// `client` | `site` | `item` | `quantity` | `date` | `exclusion` | `term`
    /// | `budget`.
    pub key: String,
    pub value: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExtractRequest {
    #[serde(default)]
    pub transcript: String,
    #[serde(default)]
    pub sources: Vec<AuthorSource>,
    #[serde(default)]
    pub client_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExtractResponse {
    pub fields: Vec<ExtractedField>,
    pub characters: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SourceUploadRequest {
    pub name: String,
    /// The browser's `File.type`, or a best guess from the extension.
    #[serde(default)]
    pub content_type: String,
    /// The file, base64-encoded. Text only: a format the extractor cannot read
    /// is refused by name rather than guessed at.
    pub data_base64: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SourceUploadResponse {
    pub accepted: bool,
    /// A typed reason when refused: `too_large`, `unsupported_type`,
    /// `not_text`, `empty`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<AuthorSource>,
}

/// One proposed edit to a draft, in plain words plus a machine-applicable path.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangeOperation {
    /// `lines[0].unit_price_minor`, `notes`, `lines[+]`.
    pub path: String,
    pub value: Value,
    /// One sentence a person reads on the proposal card.
    pub summary: String,
    /// `edit` | `add`.
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct QuoteChangeRequest {
    pub draft: QuoteDraft,
    pub instruction: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ContractChangeRequest {
    pub draft: ContractDraft,
    pub instruction: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct QuoteChangeResponse {
    pub valid: bool,
    pub saved: bool,
    pub sentence: String,
    pub operations: Vec<ChangeOperation>,
    /// The draft as it WOULD be. Nothing is applied until the person clicks.
    pub draft: Option<QuoteDraft>,
    pub resulting_total_minor: i64,
    pub currency: String,
    pub issues: Vec<MoneyAuthoringIssue>,
    pub usage: MoneyAuthorUsage,
    pub model_info: MoneyAuthorModel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContractChangeResponse {
    pub valid: bool,
    pub saved: bool,
    pub sentence: String,
    pub operations: Vec<ChangeOperation>,
    pub draft: Option<ContractDraft>,
    pub resulting_total_minor: i64,
    pub currency: String,
    pub issues: Vec<MoneyAuthoringIssue>,
    pub usage: MoneyAuthorUsage,
    pub model_info: MoneyAuthorModel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// --- model answer shapes (private) ---------------------------------------

#[derive(Debug, Default, Deserialize)]
struct AssumedFlags {
    #[serde(default)]
    quantity: bool,
    #[serde(default)]
    price: bool,
}

#[derive(Debug, Deserialize)]
struct ModelLine {
    #[serde(default)]
    description: String,
    #[serde(default)]
    quantity_milli: i64,
    #[serde(default)]
    unit_price_minor: i64,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    section: Option<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    assumed: AssumedFlags,
    /// The exact words this line came from, copied from a source. Offsets are
    /// found here, never trusted from the model.
    #[serde(default)]
    span: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ModelUnderstanding {
    #[serde(default)]
    statement: String,
    #[serde(default)]
    facts: Vec<MatchedFact>,
    #[serde(default)]
    questions: Vec<DraftQuestion>,
    #[serde(default)]
    assumptions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelQuote {
    #[serde(default)]
    title: String,
    #[serde(default)]
    currency: String,
    #[serde(default)]
    valid_for_days: Option<i64>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    lines: Vec<ModelLine>,
    #[serde(default)]
    assumptions: Vec<String>,
    #[serde(default)]
    understanding: Option<ModelUnderstanding>,
}

#[derive(Debug, Deserialize)]
struct ModelContract {
    #[serde(default)]
    name: String,
    #[serde(default)]
    contract_type: String,
    #[serde(default)]
    billing_frequency: Option<String>,
    #[serde(default)]
    currency: String,
    #[serde(default)]
    term_months: Option<i64>,
    #[serde(default)]
    effective_from: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    client_name: Option<String>,
    /// Library keys, never clause prose.
    #[serde(default)]
    clause_keys: Vec<String>,
    /// The one value a clause may fill, keyed by clause key.
    #[serde(default)]
    clause_parameters: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pricing: Vec<ModelLine>,
    #[serde(default)]
    assumptions: Vec<String>,
    #[serde(default)]
    understanding: Option<ModelUnderstanding>,
}

#[derive(Debug, Deserialize)]
struct ModelChange {
    #[serde(default)]
    sentence: String,
    #[serde(default)]
    operations: Vec<ChangeOperation>,
}

// --- sources --------------------------------------------------------------

/// Everything the person gave us, trimmed, bounded and with stable ids.
fn read_sources(req_transcript: &str, sources: &[AuthorSource]) -> ApiResult<Vec<AuthorSource>> {
    let mut out: Vec<AuthorSource> = Vec::new();
    let trimmed = req_transcript.trim();
    if !trimmed.is_empty() {
        out.push(AuthorSource {
            id: Some("typed-1".into()),
            kind: "typed".into(),
            name: None,
            text: trimmed.to_string(),
        });
    }
    for (index, source) in sources.iter().enumerate() {
        let text = source.text.trim();
        if text.is_empty() {
            continue;
        }
        if text.chars().count() > MAX_SOURCE_CHARS {
            return Err(ApiError::Validation(format!(
                "a source must be at most {MAX_SOURCE_CHARS} characters"
            )));
        }
        let kind = match source.kind.as_str() {
            "typed" | "document" | "transcript" | "recording" => source.kind.clone(),
            _ => "document".into(),
        };
        out.push(AuthorSource {
            id: Some(
                source
                    .id
                    .clone()
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or_else(|| format!("source-{}", index + 1)),
            ),
            kind,
            name: source.name.clone().filter(|name| !name.trim().is_empty()),
            text: text.to_string(),
        });
    }
    if out.is_empty() {
        return Err(ApiError::Validation(
            "give me something to read: a sentence, notes, a document or a transcript".into(),
        ));
    }
    if out.len() > MAX_SOURCES {
        return Err(ApiError::Validation(format!(
            "at most {MAX_SOURCES} sources can be read in one go"
        )));
    }
    let combined: usize = out.iter().map(|s| s.text.chars().count()).sum();
    if combined > MAX_COMBINED_CHARS {
        return Err(ApiError::Validation(format!(
            "the sources together must be at most {MAX_COMBINED_CHARS} characters"
        )));
    }
    Ok(out)
}

/// Character offsets of `needle` in `hay`, case-insensitively. Character
/// offsets, not bytes, because the notes tab slices the original text by them.
fn find_char_span(hay: &str, needle: &str) -> Option<(usize, usize)> {
    let needle = needle.trim();
    if needle.chars().count() < 6 {
        return None;
    }
    let hay_folded = hay.to_lowercase();
    let needle_folded = needle.to_lowercase();
    let byte = hay_folded.find(&needle_folded)?;
    let start = hay_folded[..byte].chars().count();
    let len = needle_folded.chars().count();
    Some((start, start + len))
}

/// Resolve the words the model quoted back into a source and offsets. A line
/// whose words are not in any source has no provenance, which is what makes
/// "from your notes" a fact rather than a claim.
fn resolve_span(sources: &[AuthorSource], quote: Option<&str>) -> Option<SourceSpan> {
    let quote = quote?;
    for source in sources {
        if let Some((start, end)) = find_char_span(&source.text, quote) {
            return Some(SourceSpan {
                source_id: source.id.clone().unwrap_or_default(),
                start,
                end,
            });
        }
    }
    None
}

fn combined_text(sources: &[AuthorSource]) -> String {
    sources
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

// --- money ----------------------------------------------------------------

/// Quantity (milli) × price (minor), rounded half-to-even, the same rule the
/// `quote_lines` amount uses. Positive operands only.
fn line_amount_minor(quantity_milli: i64, unit_price_minor: i64) -> i64 {
    let numerator = quantity_milli.max(0) as i128 * unit_price_minor.max(0) as i128;
    let denominator = 1000i128;
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let doubled = remainder * 2;
    let rounded = if doubled > denominator || (doubled == denominator && quotient % 2 != 0) {
        quotient + 1
    } else {
        quotient
    };
    rounded as i64
}

fn quote_total(draft: &QuoteDraft) -> i64 {
    draft
        .lines
        .iter()
        .filter(|line| !line.optional)
        .map(|line| line_amount_minor(line.quantity_milli, line.unit_price_minor))
        .sum()
}

fn contract_total(draft: &ContractDraft) -> i64 {
    draft
        .pricing
        .iter()
        .map(|line| line_amount_minor(line.quantity_milli, line.unit_price_minor))
        .sum()
}

/// Drop thousands separators that sit between digits, so "$5,000" and "5000"
/// read as the same number.
fn without_grouping(text: &str) -> String {
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (index, c) in chars.iter().enumerate() {
        if *c == ',' {
            let before = index.checked_sub(1).and_then(|i| chars.get(i));
            let after = chars.get(index + 1);
            if before.is_some_and(char::is_ascii_digit) && after.is_some_and(char::is_ascii_digit) {
                continue;
            }
        }
        out.push(*c);
    }
    out
}

/// A number written on its own, not the tail of a longer one: "500" is not in
/// "1500" and not in "500.99".
fn contains_amount(haystack: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(found) = haystack[from..].find(needle) {
        let start = from + found;
        let end = start + needle.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        let bounded = |c: Option<char>| match c {
            Some(c) => !c.is_ascii_digit() && c != '.',
            None => true,
        };
        if bounded(before) && bounded(after) {
            return true;
        }
        from = end;
        if from >= haystack.len() {
            break;
        }
    }
    false
}

/// Whether the words actually state this amount. A copied span proves the
/// description exists, never its price, so a price the notes do not contain is
/// not kept and the draft asks for it instead.
fn source_states_amount(text: &str, amount_minor: i64) -> bool {
    if amount_minor <= 0 {
        return false;
    }
    let whole = amount_minor / 100;
    let cents = amount_minor % 100;
    let mut forms: Vec<String> = Vec::new();
    if cents == 0 {
        forms.push(whole.to_string());
    }
    forms.push(format!("{whole}.{cents:02}"));
    if whole >= 1000 && whole % 1000 == 0 {
        forms.push(format!("{}k", whole / 1000));
    }
    let haystack = without_grouping(text);
    forms
        .iter()
        .any(|form| contains_amount(&haystack, form.as_str()))
}

// --- catalogue matching ---------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
struct CatalogRow {
    id: Uuid,
    name: String,
    unit: String,
    unit_price_minor: i64,
}

fn normalise(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The digits a name states, so "24 port" and "48 port" can never be the same
/// specification. Word overlap alone would drop both numbers as short words.
fn numeric_tokens(text: &str) -> Vec<&str> {
    text.split(' ')
        .filter(|token| !token.is_empty() && token.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// Two names that both state numbers only describe the same thing when they
/// state the same numbers.
fn numbers_disagree(wanted: &[&str], name: &[&str]) -> bool {
    if wanted.is_empty() || name.is_empty() {
        return false;
    }
    let mut a = wanted.to_vec();
    let mut b = name.to_vec();
    a.sort_unstable();
    b.sort_unstable();
    a != b
}

/// Exact name first, then word overlap in either direction, longest match wins
/// so "PoE switch" prefers the catalogue's "24 port PoE switch" to "switch".
/// A candidate whose numbers contradict the line's is never a match: an
/// ambiguous catalogue price would be a price nobody set.
fn match_catalog<'a>(description: &str, items: &'a [CatalogRow]) -> Option<&'a CatalogRow> {
    let wanted = normalise(description);
    if wanted.is_empty() {
        return None;
    }
    if let Some(exact) = items.iter().find(|item| normalise(&item.name) == wanted) {
        return Some(exact);
    }
    let wanted_tokens: Vec<&str> = wanted.split(' ').filter(|t| t.len() > 2).collect();
    let wanted_numbers = numeric_tokens(&wanted);
    if wanted_tokens.is_empty() {
        return None;
    }
    let mut best: Option<(&CatalogRow, usize)> = None;
    for item in items {
        let name = normalise(&item.name);
        let name_tokens: Vec<&str> = name.split(' ').filter(|t| t.len() > 2).collect();
        if name_tokens.is_empty() {
            continue;
        }
        if numbers_disagree(&wanted_numbers, &numeric_tokens(&name)) {
            continue;
        }
        let hits = name_tokens
            .iter()
            .filter(|token| wanted_tokens.contains(token))
            .count();
        // The item's own words must mostly be present: half is the floor, and
        // a single shared generic word is never a match.
        if hits * 2 < name_tokens.len() || hits < 2 {
            continue;
        }
        let score = hits * 100 + name_tokens.len();
        if best.is_none_or(|(_, best_score)| score > best_score) {
            best = Some((item, score));
        }
    }
    best.map(|(item, _)| item)
}

// --- budget ---------------------------------------------------------------

/// Read a budget the notes stated. "around 5k", "$5,000", "budget of 5000".
/// The number is returned as minor units; nothing is adjusted to fit it.
fn detect_budget(text: &str) -> Option<i64> {
    let lowered = text.to_lowercase();
    let index = lowered.find("budget")?;
    let tail: String = lowered.get(index..)?.chars().take(120).collect();
    let mut digits = String::new();
    let mut started = false;
    for c in tail.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            started = true;
        } else if started {
            if c == ',' && !digits.is_empty() {
                continue;
            }
            if c == '.' {
                break;
            }
            if (c == 'k' || c == 'K') && !digits.is_empty() {
                let value: i64 = digits.parse().ok()?;
                return Some(value * 100_000);
            }
            break;
        }
    }
    let value: i64 = digits.parse().ok()?;
    if value <= 0 {
        return None;
    }
    // A budget written without a decimal point is major units.
    Some(value * 100)
}

// --- clause library -------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
struct ClauseRow {
    id: Uuid,
    org_id: Option<Uuid>,
    key: String,
    title: String,
    body: String,
    version: i32,
    status: String,
    review_required: bool,
    parameter_label: Option<String>,
    standard_value: Option<String>,
}

/// One row per key: an org's own higher version wins over the shipped one.
fn pick_library(rows: Vec<ClauseRow>) -> Vec<ClauseRow> {
    let mut out: Vec<ClauseRow> = Vec::new();
    for row in rows {
        match out.iter_mut().find(|kept| kept.key == row.key) {
            Some(kept) => {
                let row_is_org = row.org_id.is_some();
                let kept_is_org = kept.org_id.is_some();
                if (row_is_org && !kept_is_org)
                    || (row_is_org == kept_is_org && row.version > kept.version)
                {
                    *kept = row;
                }
            }
            None => out.push(row),
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

fn clause_ref(row: &ClauseRow, parameter_value: Option<String>) -> ClauseRef {
    // Only a clause that carries a parameter can take a value. A clause with
    // nothing to fill keeps the library wording whatever the model supplied.
    let takes_a_value = row.parameter_label.is_some() || row.standard_value.is_some();
    let parameter_value = parameter_value
        .filter(|value| !value.trim().is_empty())
        .filter(|_| takes_a_value);
    let variant = match (&row.standard_value, &parameter_value) {
        (Some(standard), Some(value)) => normalise(standard) != normalise(value),
        _ => false,
    };
    ClauseRef {
        clause_id: row.id,
        key: row.key.clone(),
        title: row.title.clone(),
        body: row.body.clone(),
        version: row.version,
        status: if variant {
            "variant".into()
        } else {
            row.status.clone()
        },
        review_required: variant || row.review_required,
        parameter_label: row.parameter_label.clone(),
        parameter_value,
        standard_value: row.standard_value.clone(),
    }
}

// --- extraction (the live chips) ----------------------------------------

const UNIT_WORDS: [&str; 20] = [
    "access points",
    "access point",
    "aps",
    "ap",
    "switches",
    "switch",
    "cable runs",
    "cable run",
    "runs",
    "users",
    "devices",
    "seats",
    "laptops",
    "hours",
    "days",
    "sites",
    "cameras",
    "printers",
    "servers",
    "phones",
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

fn is_month(word: &str) -> bool {
    let word = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    MONTHS.contains(&word.as_str())
        || (word.len() == 3 && MONTHS.iter().any(|m| m.starts_with(&word)))
}

fn push_unique(fields: &mut Vec<ExtractedField>, key: &str, value: String) {
    let value = value
        .trim()
        .trim_matches(|c: char| c == ',' || c == '.')
        .to_string();
    if value.is_empty() || fields.iter().any(|f| f.value.eq_ignore_ascii_case(&value)) {
        return;
    }
    fields.push(ExtractedField {
        key: key.into(),
        value,
    });
}

/// A deterministic read of the words: what a person can see, shown back before
/// anything is drafted. No model call, so it is safe to run on every keystroke.
fn extract_fields(text: &str, client_names: &[String]) -> Vec<ExtractedField> {
    let mut fields: Vec<ExtractedField> = Vec::new();
    let lowered = text.to_lowercase();

    for name in client_names {
        if name.len() <= 2 {
            continue;
        }
        let folded = name.to_lowercase();
        let first = folded
            .split_whitespace()
            .find(|word| word.len() >= 4)
            .unwrap_or("");
        // A client is named by its full name or by the word people call it:
        // "Acme kickoff" names Acme Corporation. A generic short word is not
        // enough, which is why the token floor is four characters.
        if lowered.contains(&folded) || (!first.is_empty() && lowered.contains(first)) {
            push_unique(&mut fields, "client", name.clone());
            break;
        }
    }

    // A site reads as the words after " at " that carry a capital, or a token
    // ending in HQ.
    if let Some(index) = lowered.find(" at ") {
        // `lowered` is a copy: its byte offsets are only safe to slice by for
        // ASCII, so a miss returns no site rather than panicking on a boundary.
        let tail: Vec<&str> = text
            .get(index + 4..)
            .unwrap_or("")
            .split([',', '.', ';', '\n'])
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        let mut site: Vec<&str> = Vec::new();
        for word in tail.iter().take(5) {
            if site.is_empty() && !word.chars().next().is_some_and(char::is_uppercase) {
                break;
            }
            site.push(word);
            if word.ends_with("HQ") {
                break;
            }
        }
        if !site.is_empty() {
            push_unique(&mut fields, "site", site.join(" "));
        }
    }
    for word in text.split_whitespace() {
        if word.ends_with("HQ") && word.len() > 2 {
            push_unique(&mut fields, "site", word.to_string());
        }
    }

    // Quantities: a number followed by a unit word.
    let words: Vec<&str> = text.split_whitespace().collect();
    for (index, word) in words.iter().enumerate() {
        let number: String = word.chars().filter(|c| c.is_ascii_digit()).collect();
        if number.is_empty() || number.len() > 6 {
            continue;
        }
        for span in 1..=2 {
            if index + span >= words.len() {
                break;
            }
            let candidate = words[index + 1..=index + span]
                .join(" ")
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            if UNIT_WORDS.contains(&candidate.as_str()) {
                push_unique(&mut fields, "quantity", format!("{number} {candidate}"));
                break;
            }
        }
    }

    // Dates: a month word with its number, or the month alone.
    for (index, word) in words.iter().enumerate() {
        if !is_month(word) {
            continue;
        }
        let month = word
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_string();
        let previous = if index > 0 {
            words[index - 1].trim_matches(|c: char| !c.is_alphanumeric())
        } else {
            ""
        };
        if !previous.is_empty() && previous.chars().any(|c| c.is_ascii_digit()) {
            push_unique(&mut fields, "date", format!("{previous} {month}"));
        } else {
            push_unique(&mut fields, "date", month);
        }
    }

    // Exclusions: a sentence that says what is out.
    for sentence in text.split(['.', '\n']) {
        let lowered_sentence = sentence.to_lowercase();
        let excluded = [
            " no ",
            "zero downtime",
            "not included",
            "stays as is",
            "ignore",
            "exclud",
        ]
        .iter()
        .any(|needle| lowered_sentence.contains(needle));
        if excluded {
            let trimmed: String = sentence.trim().chars().take(70).collect();
            if !trimmed.is_empty() {
                push_unique(&mut fields, "exclusion", trimmed);
            }
        }
    }

    // Terms: payment and validity, stated plainly.
    for (index, word) in words.iter().enumerate() {
        let lowered_word = word.to_lowercase();
        if lowered_word == "net" {
            if let Some(next) = words.get(index + 1) {
                push_unique(
                    &mut fields,
                    "term",
                    format!("Net {}", next.trim_matches(|c: char| !c.is_alphanumeric())),
                );
            }
        }
        if let Some(next) = words
            .get(index + 1)
            .filter(|_| lowered_word.starts_with("valid"))
        {
            push_unique(&mut fields, "term", format!("Valid {}", next));
        }
        if lowered_word == "notice" && index > 0 {
            let previous = words[index - 1];
            if previous.chars().any(|c| c.is_ascii_digit()) {
                push_unique(&mut fields, "term", format!("{previous} days notice"));
            }
        }
        if lowered_word.starts_with("renew") {
            push_unique(&mut fields, "term", "Renews automatically".into());
        }
    }

    if let Some(budget) = detect_budget(text) {
        push_unique(
            &mut fields,
            "budget",
            format!("Budget {:.2}", budget as f64 / 100.0),
        );
    }

    // The activities the pack's chips name, in the words people use.
    for (needle, label) in [
        ("survey", "A site survey"),
        ("install", "An install"),
        ("cabl", "Cabling"),
        ("handover", "Configuration and handover"),
        ("support plan", "A support plan"),
        ("weekend", "Weekend work"),
        ("after-hours", "After-hours rate"),
    ] {
        if lowered.contains(needle) {
            push_unique(&mut fields, "item", label.to_string());
        }
    }

    if let Some(budget) = detect_budget(text) {
        push_unique(
            &mut fields,
            "budget",
            format!("Budget {:.2}", budget as f64 / 100.0),
        );
    }

    fields.truncate(12);
    fields
}

// --- validation -----------------------------------------------------------

fn validate_understanding(mut understanding: ModelUnderstanding, statement: &str) -> Understanding {
    if understanding.statement.trim().is_empty() {
        understanding.statement = statement.to_string();
    }
    understanding.questions.truncate(MAX_QUESTIONS);
    for question in understanding.questions.iter_mut() {
        question.id = question.id.trim().to_string();
        if question.id.is_empty() {
            question.id = normalise(&question.text).replace(' ', "-");
        }
        question.options.retain(|option| !option.trim().is_empty());
        if question.options.len() < 2 {
            question.options = vec!["Yes".into(), "No".into()];
        }
        question.chosen = Some(match question.chosen.clone() {
            Some(chosen) if question.options.contains(&chosen) => chosen,
            _ => question.options[0].clone(),
        });
    }
    understanding
        .facts
        .retain(|fact| !fact.value.trim().is_empty());
    understanding
        .assumptions
        .retain(|line| !line.trim().is_empty());
    Understanding {
        statement: understanding.statement,
        facts: understanding.facts,
        questions: understanding.questions,
        assumptions: understanding.assumptions,
    }
}

fn validate_quote(
    draft: &mut QuoteDraft,
    sources: &[AuthorSource],
    catalog: &[CatalogRow],
    clients: &[(Uuid, String)],
) -> Vec<MoneyAuthoringIssue> {
    let mut issues = Vec::new();
    if draft.title.trim().is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "title".into(),
            message: "the quote needs a title".into(),
        });
    }
    if draft.currency.len() != 3 {
        draft.currency = "USD".into();
    }
    draft.currency = draft.currency.to_uppercase();
    draft
        .lines
        .retain(|line| !line.description.trim().is_empty() && line.quantity_milli > 0);
    if draft.lines.is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "lines".into(),
            message: "the notes did not name any billable work".into(),
        });
    }

    let mut extra_assumptions: Vec<String> = Vec::new();
    let text = combined_text(sources);
    for (index, line) in draft.lines.iter_mut().enumerate() {
        line.description = line.description.trim().to_string();
        if line.unit_price_minor < 0 {
            line.unit_price_minor = 0;
            extra_assumptions.push(format!(
                "Line {} had a negative price, so it is zero until you set it.",
                index + 1
            ));
        }
        // A span that does not point at a source we were given is dropped. The
        // offsets came from this module, so this only catches a caller
        // round-tripping a draft through another org's sources.
        line.source_span = line.source_span.take().filter(|span| {
            sources
                .iter()
                .any(|source| source.id.as_deref() == Some(span.source_id.as_str()))
        });

        // The catalogue prices a line it recognises. Its price is stated, not
        // assumed, and it replaces whatever the notes implied.
        if let Some(item) = match_catalog(&line.description, catalog) {
            line.catalog_item_id = Some(item.id);
            line.unit_price_minor = item.unit_price_minor;
            if line.unit.as_deref().unwrap_or("").trim().is_empty() {
                line.unit = Some(item.unit.clone());
            }
            line.price_assumed = item.unit_price_minor == 0;
            line.source = "catalogue".into();
        } else if line.unit_price_minor > 0 && !source_states_amount(&text, line.unit_price_minor) {
            // The span proves the words, never the amount. A price the notes do
            // not state is not kept, so the check list asks for it.
            line.unit_price_minor = 0;
            line.price_assumed = true;
            extra_assumptions.push(format!(
                "Line {} had a price the notes do not state, so it is zero until you set it.",
                index + 1
            ));
        } else if line.unit_price_minor == 0 {
            line.price_assumed = true;
            line.source = "assumed".into();
        }

        // Provenance is mechanical: no span, no "from your notes". A catalogue
        // line is its own case — its price is stated, its words may not be.
        if line.source_span.is_none() && line.source != "catalogue" {
            line.source = "assumed".into();
        }
    }

    // A client the person picked wins; otherwise trust a name the server can
    // find in the org's client list.
    if draft.client_id.is_none() {
        if let Some(name) = draft.client_name.clone() {
            let found = clients
                .iter()
                .find(|(_, client)| client.eq_ignore_ascii_case(name.trim()));
            if let Some((id, _)) = found {
                draft.client_id = Some(*id);
            }
        }
    }
    if let Some(id) = draft.client_id {
        if let Some((_, name)) = clients.iter().find(|(client_id, _)| *client_id == id) {
            draft.client_name = Some(name.clone());
        }
    }

    draft.budget_minor = draft.budget_minor.or_else(|| {
        let text = draft.notes.clone().unwrap_or_default();
        detect_budget(&text)
    });
    draft.assumptions.extend(extra_assumptions);
    draft.assumptions.retain(|line| !line.trim().is_empty());
    if draft.valid_for_days.unwrap_or(0) <= 0 {
        draft.valid_for_days = Some(30);
    }
    draft.checks = quote_checks(draft);
    issues
}

fn validate_contract(
    draft: &mut ContractDraft,
    sources: &[AuthorSource],
    clients: &[(Uuid, String)],
    library: &[ClauseRow],
    requested_keys: &[String],
    parameters: &std::collections::BTreeMap<String, String>,
) -> Vec<MoneyAuthoringIssue> {
    const TYPES: [&str; 6] = [
        "recurring",
        "time_and_materials",
        "retainer",
        "fixed_fee",
        "per_unit",
        "per_incident",
    ];
    const FREQUENCIES: [&str; 5] = ["weekly", "monthly", "quarterly", "semiannual", "annual"];
    let mut issues = Vec::new();
    if draft.name.trim().is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "name".into(),
            message: "the agreement needs a name".into(),
        });
    }
    if !TYPES.contains(&draft.contract_type.as_str()) {
        draft.contract_type = "recurring".into();
    }
    if let Some(frequency) = draft.billing_frequency.as_deref() {
        if !FREQUENCIES.contains(&frequency) {
            draft.billing_frequency = Some("monthly".into());
        }
    }
    if draft.currency.len() != 3 {
        draft.currency = "USD".into();
    }
    draft.currency = draft.currency.to_uppercase();

    let text = combined_text(sources);
    for line in draft.pricing.iter_mut() {
        line.label = line.label.trim().to_string();
        line.source_span = line.source_span.clone().filter(|span| {
            sources
                .iter()
                .any(|s| s.id.as_deref() == Some(span.source_id.as_str()))
        });
        if line.unit_price_minor < 0 {
            line.unit_price_minor = 0;
            draft.assumptions.push(format!(
                "The rate for {} was negative, so it needs setting.",
                line.label
            ));
        } else if line.unit_price_minor > 0 && !source_states_amount(&text, line.unit_price_minor) {
            // A span proves the words, never the rate. A rate the words do not
            // state is not kept, so the check list asks for it.
            line.unit_price_minor = 0;
            draft.assumptions.push(format!(
                "The rate for {} was not in your words, so it needs setting.",
                line.label
            ));
        }
        if line.unit_price_minor == 0 {
            line.price_assumed = true;
        }
    }
    draft
        .pricing
        .retain(|line| !line.label.is_empty() && line.quantity_milli > 0);
    // The value is the arithmetic over the rates. A total the model wrote is
    // never kept: with no rates there is no value to stand behind.
    let value = if draft.pricing.is_empty() {
        None
    } else {
        Some(contract_total(draft))
    };
    draft.value_minor = value;

    if draft.client_id.is_none() {
        if let Some(name) = draft.client_name.clone() {
            if let Some((id, _)) = clients
                .iter()
                .find(|(_, client)| client.eq_ignore_ascii_case(name.trim()))
            {
                draft.client_id = Some(*id);
            }
        }
    }
    if let Some(id) = draft.client_id {
        if let Some((_, name)) = clients.iter().find(|(client_id, _)| *client_id == id) {
            draft.client_name = Some(name.clone());
        }
    }

    // Clauses: only library rows, never model prose. A draft that names none
    // still gets the approved baseline, because a contract without clauses is
    // not a contract.
    let wanted: Vec<&ClauseRow> = if requested_keys.is_empty() {
        library.iter().collect()
    } else {
        library
            .iter()
            .filter(|row| requested_keys.iter().any(|key| key == &row.key))
            .collect()
    };
    draft.clause_refs = wanted
        .iter()
        .map(|row| clause_ref(row, parameters.get(&row.key).cloned()))
        .collect();
    draft.clauses = draft.clause_refs.iter().map(|r| r.title.clone()).collect();
    if requested_keys.is_empty() && !library.is_empty() {
        draft.assumptions.push(
            "The clauses come from your approved library; the notes did not choose any.".into(),
        );
    }
    for clause in draft.clause_refs.iter().filter(|c| c.review_required) {
        draft.assumptions.push(format!(
            "{} differs from the standard wording, so it is marked for legal review.",
            clause.title
        ));
    }
    draft.assumptions.retain(|line| !line.trim().is_empty());
    draft.checks = contract_checks(draft);
    issues
}

/// The "things to check" list. Derived from the flags and the totals, so a mark
/// on a row always has a sentence behind it.
fn quote_checks(draft: &QuoteDraft) -> Vec<String> {
    let mut checks: Vec<String> = Vec::new();
    let total = quote_total(draft);
    for line in &draft.lines {
        if line.price_assumed {
            checks.push(format!(
                "The price for {} was not in your notes. Set it before sending.",
                line.description
            ));
        } else if line.source == "catalogue" {
            checks.push(format!(
                "The price for {} comes from your catalogue.",
                line.description
            ));
        }
        if line.quantity_assumed {
            checks.push(format!(
                "The quantity for {} was assumed.",
                line.description
            ));
        }
        if line.source_span.is_none() && line.source == "assumed" && !line.price_assumed {
            checks.push(format!(
                "{} was added; it was not in your notes.",
                line.description
            ));
        }
    }
    if let Some(budget) = draft.budget_minor {
        if total > budget && budget > 0 {
            checks.push(format!(
                "The total is {:.2} over the budget in your notes.",
                (total - budget) as f64 / 100.0
            ));
        }
    }
    checks.truncate(9);
    checks
}

fn contract_checks(draft: &ContractDraft) -> Vec<String> {
    let mut checks: Vec<String> = Vec::new();
    for clause in draft.clause_refs.iter().filter(|c| c.review_required) {
        checks.push(format!(
            "{} differs from the standard wording. Legal review is advised.",
            clause.title
        ));
    }
    for line in draft.pricing.iter().filter(|line| line.price_assumed) {
        checks.push(format!("The rate for {} needs setting.", line.label));
    }
    for line in draft.pricing.iter().filter(|line| line.quantity_assumed) {
        checks.push(format!("The quantity for {} was assumed.", line.label));
    }
    if draft.pricing.is_empty() {
        checks.push("The agreement has no rate. Add a price before sending.".into());
    }
    if draft.client_id.is_none() {
        checks.push("The client was not matched. Choose it before sending.".into());
    }
    checks.truncate(9);
    checks
}

// --- prompts --------------------------------------------------------------

fn quote_system_prompt() -> String {
    r#"You turn a technician's own words about a job into a QUOTE DRAFT.

Return ONLY one JSON object with this shape:
{
  "title": "short title for the quote",
  "currency": "USD",
  "valid_for_days": 30,
  "notes": "anything the customer should read, or null",
  "client_name": "the client named in the words, or null",
  "understanding": {
    "statement": "one sentence: what this job is, for whom",
    "facts": [{"label": "Budget", "value": "About $5,000, from the notes"}],
    "questions": [{"id": "approver", "text": "Who approves this?", "options": ["Likeliest answer", "Other"], "chosen": "Likeliest answer"}],
    "assumptions": ["one plain sentence per assumption"]
  },
  "lines": [
    {"description": "what is being sold", "quantity_milli": 1000, "unit_price_minor": 10000,
     "unit": "each", "section": "Hardware", "optional": false,
     "assumed": {"quantity": false, "price": false},
     "span": "the exact words from the notes this line came from"}
  ],
  "assumptions": ["facts you had to guess"]
}

Rules:
- quantity_milli is quantity x 1000: one unit is 1000, two and a half hours is 2500.
- unit_price_minor is the unit price in cents: $100.00 is 10000.
- NEVER invent a price that was not in the notes. Use 0 and set
  assumed.price true. The catalogue supplies prices for the things it knows.
- "span" must be copied WORD FOR WORD from the notes, at most 120 characters,
  and must be the words this line came from. If you cannot quote them, leave
  span null.
- Set assumed.quantity or assumed.price when you guessed that field.
- Group lines under sections such as Hardware, Cabling, Labour, Optional.
  An optional extra is "optional": true and is not part of the total.
- Ask at most two questions, only for what you cannot decide, and pre-select
  the likeliest answer in "chosen" (it must be one of the options).
- If the notes name a currency, use it. Otherwise USD and say so in assumptions.
- If the notes are unusable, return {"lines": [], "assumptions": ["..."]}.
- Never write a total, a subtotal or a tax amount.
- No prose outside the JSON."#
        .to_string()
}

fn contract_system_prompt() -> String {
    r#"You turn a technician's own words about a client agreement into a CONTRACT DRAFT.

You never write legal wording. You choose clauses from the approved library by
key and may fill that clause's one parameter.

Return ONLY one JSON object with this shape:
{
  "name": "short name for the agreement",
  "contract_type": "recurring",
  "billing_frequency": "monthly",
  "currency": "USD",
  "term_months": 12,
  "effective_from": "2026-11-15",
  "notes": "what the agreement covers, or null",
  "client_name": "the client named in the words, or null",
  "clause_keys": ["scope", "response", "exclusions", "payment", "term", "renewal_notice", "data_handling"],
  "clause_parameters": {"renewal_notice": "60 days", "response": "2 hours", "payment": "Net 14"},
  "pricing": [
    {"description": "Users", "quantity_milli": 12000, "unit_price_minor": 3800,
     "assumed": {"quantity": false, "price": false}, "span": "12 users at $38 each per month"}
  ],
  "understanding": {
    "statement": "one sentence: what this agreement is, for whom",
    "facts": [{"label": "Template", "value": "Managed Silver, version 4"}],
    "questions": [{"id": "signer", "text": "Who signs?", "options": ["Likeliest", "Other"], "chosen": "Likeliest"}],
    "assumptions": ["one plain sentence per assumption"]
  },
  "assumptions": ["facts you had to guess"]
}

Rules:
- contract_type is one of: recurring, time_and_materials, retainer, fixed_fee,
  per_unit, per_incident.
- billing_frequency is one of: weekly, monthly, quarterly, semiannual, annual,
  or null for a one-off.
- clause_keys may only contain keys from that list. Never invent a key and never
  write clause text.
- clause_parameters: at most one value per clause, copied from the words.
  Omit a clause whose standard value stands.
- pricing is quantity x rate, monthly where the agreement bills monthly.
  quantity_milli is quantity x 1000; unit_price_minor is cents.
- NEVER invent a number that was not in the words. Leave it out and say so in
  assumptions.
- "span" must be copied WORD FOR WORD from the words, at most 120 characters.
- effective_from is in the past or today, never a guess at the future.
- Ask at most one question, with its likeliest answer in "chosen".
- Never write a total, a subtotal or a tax amount.
- No prose outside the JSON."#
        .to_string()
}

fn quote_change_prompt() -> String {
    r#"You change an existing QUOTE DRAFT from one sentence.

Return ONLY one JSON object:
{"sentence": "one sentence naming the change", "operations": [
  {"path": "lines[0].unit_price_minor", "value": 17955, "summary": "Take 5% off the access points", "kind": "edit"}
]}

Allowed paths:
- lines[i].description (text)
- lines[i].quantity_milli (quantity x 1000)
- lines[i].unit_price_minor (cents)
- lines[i].optional (true or false)
- lines[i].unit  (text)
- title, notes, valid_for_days
- lines[+] (kind "add") where value is a whole line object with description,
  quantity_milli and unit_price_minor

Rules:
- One operation per change, and one plain sentence per operation.
- Never state a total. Never add tax. Never change currency.
- If the sentence asks for something outside those paths, return no operations
  and say why in "sentence".
- Do not describe the whole draft back; only the operations.
- No prose outside the JSON."#
        .to_string()
}

fn contract_change_prompt() -> String {
    r#"You change an existing CONTRACT DRAFT from one sentence.

Return ONLY one JSON object:
{"sentence": "one sentence naming the change", "operations": [
  {"path": "pricing[0].unit_price_minor", "value": 3990, "summary": "Raise the user rate to $39.90", "kind": "edit"}
]}

Allowed paths:
- name, notes, term_months, effective_from, billing_frequency
- pricing[i].label, pricing[i].quantity_milli, pricing[i].unit_price_minor
- clause_parameters.<clause key> (a value the clause fills)

Rules:
- One operation per change, and one plain sentence per operation.
- Never write clause text and never invent a clause key.
- Never state a total.
- If the sentence asks for something outside those paths, return no operations
  and say why in "sentence".
- No prose outside the JSON."#
        .to_string()
}

// --- model call -----------------------------------------------------------

struct CallOutcome {
    content: String,
    prompt_tokens: u32,
    completion_tokens: u32,
    error: Option<String>,
}

async fn run_call(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    surface_key: &str,
    system: &str,
    transcript: &str,
) -> ApiResult<(CallOutcome, String, String)> {
    let mut tx = org_bound_tx(state, org_id, user_id).await?;
    let call = authorize_surface_call(
        state,
        &mut tx,
        AiAdmissionAsk {
            org_id,
            user_id,
            surface_key,
            provider_ask: None,
            model_ask: None,
            required_capabilities: &[AiEdge::Completion],
            internal_core_call: false,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    let transport = chat_runner::resolve_transport(org_id, &call);
    let request = ModelCallRequest {
        model: Some(call.model.clone()),
        messages: vec![
            LoopMessage {
                role: "system".into(),
                content: system.to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            LoopMessage {
                role: "user".into(),
                content: transcript.to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ],
        tools: Vec::new(),
        response_format: Some(json!({ "type": "json_object" })),
    };
    let (content, prompt_tokens, completion_tokens, error) = match transport.call(&request).await {
        Ok(turn) => (
            turn.content,
            turn.prompt_tokens,
            turn.completion_tokens,
            None,
        ),
        Err(err) => (String::new(), 0, 0, Some(err.to_string())),
    };

    // Usage is recorded whether or not the transport returned content: the
    // provider may have billed a failed turn, and the cap has to see it.
    let mut usage_tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return Ok((
                CallOutcome {
                    content,
                    prompt_tokens,
                    completion_tokens,
                    error,
                },
                call.provider.clone(),
                call.model.clone(),
            ))
        }
    };
    if core_db::set_org_and_user(&mut usage_tx, org_id, user_id)
        .await
        .is_ok()
    {
        let _ = record_usage(
            &mut usage_tx,
            &AiUsageNew {
                id: Uuid::now_v7(),
                org_id,
                user_id,
                surface: call.surface_key.clone(),
                provider: call.provider.clone(),
                model: call.model.clone(),
                prompt_tokens: prompt_tokens as i32,
                completion_tokens: completion_tokens as i32,
                cost_weight: call.cost_weight,
                request_id: Some(format!("money_author:{}", Uuid::now_v7())),
            },
        )
        .await;
        let _ = core_db::audit::insert(
            &mut *usage_tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Agent(format!("money_authoring/user:{user_id}")),
                action: "ai.money_author".into(),
                entity_type: Some(surface_key.to_string()),
                entity_id: None,
                detail: json!({
                    "surface": call.surface_key,
                    "provider": call.provider,
                    "model": call.model,
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": completion_tokens,
                    "error": error,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await;
        let _ = usage_tx.commit().await;
    }

    Ok((
        CallOutcome {
            content,
            prompt_tokens,
            completion_tokens,
            error,
        },
        call.provider.clone(),
        call.model.clone(),
    ))
}

/// Pull a JSON object out of a model answer. Providers vary: some wrap it in a
/// fence, some prepend a sentence. The boundary is the outer braces and the
/// parse still has to succeed.
fn extract_json(content: &str) -> Option<Value> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&content[start..=end]).ok()
}

// --- reads (org scoped) ---------------------------------------------------

async fn read_clients(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
) -> ApiResult<Vec<(Uuid, String)>> {
    let mut tx = org_bound_tx(state, org_id, user_id).await?;
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, name FROM clients WHERE org_id = $1 ORDER BY name, id LIMIT 500",
    )
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

async fn read_catalog(state: &AppState, org_id: Uuid, user_id: Uuid) -> ApiResult<Vec<CatalogRow>> {
    let mut tx = org_bound_tx(state, org_id, user_id).await?;
    let rows = sqlx::query_as::<_, CatalogRow>(
        "SELECT id, name, unit, unit_price_minor FROM catalog_items
          WHERE org_id = $1 AND active
          ORDER BY name, id
          LIMIT 500",
    )
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

async fn read_library(state: &AppState, org_id: Uuid, user_id: Uuid) -> ApiResult<Vec<ClauseRow>> {
    let mut tx = org_bound_tx(state, org_id, user_id).await?;
    let rows = sqlx::query_as::<_, ClauseRow>(
        "SELECT id, org_id, key, title, body, version, status, review_required,
                parameter_label, standard_value
           FROM contract_clauses
          WHERE (org_id = $1 OR org_id IS NULL) AND active",
    )
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(pick_library(rows))
}

/// The user message the model sees: the person's words, labelled by source, and
/// the optional steer. Labels matter — a transcript and an instruction are read
/// differently, and an instruction is not a source.
fn model_user_message(sources: &[AuthorSource], instruction: Option<&str>) -> String {
    let mut out = String::from("The person gave you these sources:\n");
    for (index, source) in sources.iter().enumerate() {
        let name = source
            .name
            .clone()
            .unwrap_or_else(|| format!("source {}", index + 1));
        out.push_str(&format!(
            "\n--- source {} [{}] {} ---\n{}\n",
            index + 1,
            source.kind,
            name,
            source.text
        ));
    }
    if let Some(instruction) = instruction.map(str::trim).filter(|i| !i.is_empty()) {
        out.push_str(&format!("\nTheir instruction: {instruction}\n"));
    }
    out
}

// --- handlers: author -----------------------------------------------------

/// POST /api/quotes/author — draft a quote from words, notes or uploads. No write.
#[utoipa::path(operation_id = "quote_author", post, path = "/api/quotes/author",
    tag = "quotes", security(("bearer" = [])), request_body = AuthorMoneyRequest,
    responses((status = 200, body = AuthorQuoteResponse),
              (status = 403, description = "AI admission refused (typed reason in the envelope)")))]
pub async fn author_quote(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AuthorMoneyRequest>,
) -> ApiResult<Json<AuthorQuoteResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let sources = read_sources(&req.transcript, &req.sources)?;
    let instruction = req.instruction.as_deref();

    let (outcome, provider, model) = run_call(
        &state,
        org_id,
        user_id,
        QUOTE_SURFACE,
        &quote_system_prompt(),
        &model_user_message(&sources, instruction),
    )
    .await?;

    let mut issues = Vec::new();
    let mut draft = extract_json(&outcome.content).and_then(|value| {
        match serde_json::from_value::<ModelQuote>(value) {
            Ok(model_draft) => Some(model_draft),
            Err(error) => {
                issues.push(MoneyAuthoringIssue {
                    field: "draft".into(),
                    message: format!("the model's answer was not a quote draft: {error}"),
                });
                None
            }
        }
    });

    let draft = match draft.take() {
        None => None,
        Some(model_draft) => {
            let clients = read_clients(&state, org_id, user_id).await?;
            let catalog = read_catalog(&state, org_id, user_id).await?;
            let text = combined_text(&sources);
            let mut draft = QuoteDraft {
                title: model_draft.title,
                currency: model_draft.currency,
                valid_for_days: model_draft.valid_for_days,
                notes: model_draft.notes,
                client_id: req.client_id,
                client_name: model_draft.client_name,
                budget_minor: detect_budget(&text),
                lines: model_draft
                    .lines
                    .iter()
                    .map(|line| QuoteLineDraft {
                        description: line.description.clone(),
                        quantity_milli: line.quantity_milli,
                        unit_price_minor: line.unit_price_minor,
                        unit: line.unit.clone(),
                        section: line.section.clone(),
                        optional: line.optional,
                        catalog_item_id: None,
                        source: if resolve_span(&sources, line.span.as_deref()).is_some() {
                            "notes".into()
                        } else {
                            "assumed".into()
                        },
                        source_span: resolve_span(&sources, line.span.as_deref()),
                        quantity_assumed: line.assumed.quantity,
                        price_assumed: line.assumed.price || line.unit_price_minor == 0,
                    })
                    .collect(),
                assumptions: model_draft.assumptions.clone(),
                understanding: None,
                checks: Vec::new(),
            };
            issues.extend(validate_quote(&mut draft, &sources, &catalog, &clients));

            let understanding = model_draft.understanding.unwrap_or_default();
            let mut understanding = validate_understanding(
                understanding,
                &format!(
                    "A quote for {}.",
                    draft
                        .client_name
                        .clone()
                        .unwrap_or_else(|| "the client in your notes".into())
                ),
            );
            if let Some(name) = draft.client_name.clone() {
                understanding.facts.insert(
                    0,
                    MatchedFact {
                        label: "Client".into(),
                        value: if draft.client_id.is_some() {
                            format!("{name}, matched")
                        } else {
                            format!("{name}, needs choosing")
                        },
                    },
                );
            }
            understanding.facts.push(MatchedFact {
                label: "Items".into(),
                value: format!(
                    "{} found, {} optional",
                    draft.lines.iter().filter(|l| !l.optional).count(),
                    draft.lines.iter().filter(|l| l.optional).count()
                ),
            });
            if let Some(budget) = draft.budget_minor {
                understanding.facts.push(MatchedFact {
                    label: "Budget".into(),
                    value: format!("{:.2}, from your notes", budget as f64 / 100.0),
                });
            }
            understanding
                .assumptions
                .extend(draft.assumptions.iter().cloned());
            understanding.assumptions.truncate(6);
            draft.understanding = Some(understanding);
            Some(draft)
        }
    };

    if draft.is_none() && issues.is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "draft".into(),
            message: "the model returned no draft".into(),
        });
    }
    let assumptions = draft
        .as_ref()
        .map(|d| d.assumptions.clone())
        .unwrap_or_default();
    let valid = draft.is_some() && issues.is_empty();
    let reference = (draft.is_none() && outcome.error.is_some())
        .then(|| format!("dsc_{}", Uuid::now_v7().simple()))
        .map(|value| value.chars().take(14).collect::<String>());
    Ok(Json(AuthorQuoteResponse {
        valid,
        saved: false,
        draft,
        assumptions,
        issues,
        usage: MoneyAuthorUsage {
            prompt_tokens: outcome.prompt_tokens,
            completion_tokens: outcome.completion_tokens,
        },
        model_info: MoneyAuthorModel { provider, model },
        error: outcome.error,
        reference,
    }))
}

/// POST /api/contracts/author — draft a contract from words, notes or uploads.
/// No write, no legal wording: clauses come from the library by key.
#[utoipa::path(operation_id = "contract_author", post, path = "/api/contracts/author",
    tag = "contracts", security(("bearer" = [])), request_body = AuthorMoneyRequest,
    responses((status = 200, body = AuthorContractResponse),
              (status = 403, description = "AI admission refused (typed reason in the envelope)")))]
pub async fn author_contract(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AuthorMoneyRequest>,
) -> ApiResult<Json<AuthorContractResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let sources = read_sources(&req.transcript, &req.sources)?;
    let instruction = req.instruction.as_deref();

    let (outcome, provider, model) = run_call(
        &state,
        org_id,
        user_id,
        CONTRACT_SURFACE,
        &contract_system_prompt(),
        &model_user_message(&sources, instruction),
    )
    .await?;

    let mut issues = Vec::new();
    let mut parsed =
        extract_json(&outcome.content).and_then(|value| {
            match serde_json::from_value::<ModelContract>(value) {
                Ok(model_draft) => Some(model_draft),
                Err(error) => {
                    issues.push(MoneyAuthoringIssue {
                        field: "draft".into(),
                        message: format!("the model's answer was not a contract draft: {error}"),
                    });
                    None
                }
            }
        });

    let draft = match parsed.take() {
        None => None,
        Some(mut model_draft) => {
            let clients = read_clients(&state, org_id, user_id).await?;
            let library = read_library(&state, org_id, user_id).await?;
            let model_understanding = model_draft.understanding.take();
            let template_name = model_draft_name(&model_draft);
            let clause_keys = model_draft.clause_keys.clone();
            let clause_parameters = model_draft.clause_parameters.clone();
            let mut draft = ContractDraft {
                name: model_draft.name,
                contract_type: model_draft.contract_type,
                billing_frequency: model_draft.billing_frequency,
                value_minor: None,
                currency: model_draft.currency,
                term_months: model_draft.term_months,
                effective_from: model_draft.effective_from,
                notes: model_draft.notes,
                clauses: Vec::new(),
                clause_refs: Vec::new(),
                pricing: model_draft
                    .pricing
                    .iter()
                    .map(|line| ContractPriceLine {
                        label: line.description.clone(),
                        quantity_milli: line.quantity_milli,
                        unit_price_minor: line.unit_price_minor,
                        quantity_assumed: line.assumed.quantity,
                        price_assumed: line.assumed.price || line.unit_price_minor == 0,
                        source_span: resolve_span(&sources, line.span.as_deref()),
                    })
                    .collect(),
                client_id: req.client_id,
                client_name: model_draft.client_name,
                assumptions: model_draft.assumptions.clone(),
                understanding: None,
                checks: Vec::new(),
            };
            issues.extend(validate_contract(
                &mut draft,
                &sources,
                &clients,
                &library,
                &clause_keys,
                &clause_parameters,
            ));

            let mut understanding = validate_understanding(
                model_understanding.unwrap_or_default(),
                &format!(
                    "A contract for {}.",
                    draft
                        .client_name
                        .clone()
                        .unwrap_or_else(|| "the client in your words".into())
                ),
            );
            if let Some(name) = draft.client_name.clone() {
                understanding.facts.insert(
                    0,
                    MatchedFact {
                        label: "Client".into(),
                        value: if draft.client_id.is_some() {
                            format!("{name}, matched")
                        } else {
                            format!("{name}, needs choosing")
                        },
                    },
                );
            }
            understanding.facts.push(MatchedFact {
                label: "Wording".into(),
                value: format!(
                    "{} clauses from your library, {} changed",
                    draft.clause_refs.len(),
                    draft
                        .clause_refs
                        .iter()
                        .filter(|c| c.review_required)
                        .count()
                ),
            });
            if let Some(name) = template_name {
                understanding.facts.push(MatchedFact {
                    label: "Template".into(),
                    value: name,
                });
            }
            understanding
                .assumptions
                .extend(draft.assumptions.iter().cloned());
            understanding.assumptions.push(
                "I wrote no new legal wording. Every clause is from your approved library.".into(),
            );
            understanding.assumptions.truncate(7);
            draft.understanding = Some(understanding);
            Some(draft)
        }
    };

    if draft.is_none() && issues.is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "draft".into(),
            message: "the model returned no draft".into(),
        });
    }
    let assumptions = draft
        .as_ref()
        .map(|d| d.assumptions.clone())
        .unwrap_or_default();
    let valid = draft.is_some() && issues.is_empty();
    let reference = (draft.is_none() && outcome.error.is_some())
        .then(|| format!("dsc_{}", Uuid::now_v7().simple()))
        .map(|value| value.chars().take(14).collect::<String>());
    Ok(Json(AuthorContractResponse {
        valid,
        saved: false,
        draft,
        assumptions,
        issues,
        usage: MoneyAuthorUsage {
            prompt_tokens: outcome.prompt_tokens,
            completion_tokens: outcome.completion_tokens,
        },
        model_info: MoneyAuthorModel { provider, model },
        error: outcome.error,
        reference,
    }))
}

/// The template name a contract's words implied, when the model named one.
fn model_draft_name(model: &ModelContract) -> Option<String> {
    model
        .understanding
        .as_ref()
        .and_then(|understanding| {
            understanding
                .facts
                .iter()
                .find(|fact| fact.label.eq_ignore_ascii_case("template"))
        })
        .map(|fact| fact.value.clone())
}

// --- handlers: extract ----------------------------------------------------

/// POST /api/quotes/extract — the live chips. No model call, no write: it reads
/// the words the panel already has, so it is safe to run on every keystroke.
#[utoipa::path(operation_id = "quote_extract", post, path = "/api/quotes/extract",
    tag = "quotes", security(("bearer" = [])), request_body = ExtractRequest,
    responses((status = 200, body = ExtractResponse)))]
pub async fn extract_quote(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ExtractRequest>,
) -> ApiResult<Json<ExtractResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    extract(state, authed, req).await
}

/// POST /api/contracts/extract — the live chips for a contract description.
#[utoipa::path(operation_id = "contract_extract", post, path = "/api/contracts/extract",
    tag = "contracts", security(("bearer" = [])), request_body = ExtractRequest,
    responses((status = 200, body = ExtractResponse)))]
pub async fn extract_contract(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ExtractRequest>,
) -> ApiResult<Json<ExtractResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    extract(state, authed, req).await
}

async fn extract(
    state: AppState,
    authed: crate::auth::AuthedUser,
    req: ExtractRequest,
) -> ApiResult<Json<ExtractResponse>> {
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let sources = read_sources(&req.transcript, &req.sources)?;
    let text = combined_text(&sources);
    let clients = read_clients(&state, org_id, user_id).await?;
    let names: Vec<String> = clients.into_iter().map(|(_, name)| name).collect();
    Ok(Json(ExtractResponse {
        fields: extract_fields(&text, &names),
        characters: text.chars().count(),
    }))
}

// --- handlers: clauses ----------------------------------------------------

/// GET /api/contracts/clauses — the approved library the contract builder
/// chooses from. Read-only, org-scoped, and the only source of clause text.
#[utoipa::path(operation_id = "contract_clauses_list", get, path = "/api/contracts/clauses",
    tag = "contracts", security(("bearer" = [])),
    responses((status = 200, body = Vec<ClauseRef>)))]
pub async fn list_clauses(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<ClauseRef>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let library = read_library(&state, org_id, user_id).await?;
    Ok(Json(
        library.iter().map(|row| clause_ref(row, None)).collect(),
    ))
}

// --- handlers: change -----------------------------------------------------

/// POST /api/quotes/author/change — propose edits from one sentence. Nothing is
/// applied, here or by the client, until a person clicks Apply.
#[utoipa::path(operation_id = "quote_author_change", post, path = "/api/quotes/author/change",
    tag = "quotes", security(("bearer" = [])), request_body = QuoteChangeRequest,
    responses((status = 200, body = QuoteChangeResponse)))]
pub async fn change_quote(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<QuoteChangeRequest>,
) -> ApiResult<Json<QuoteChangeResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let instruction = req.instruction.trim().to_string();
    if instruction.is_empty() {
        return Err(ApiError::Validation("say what should change".into()));
    }
    let user_message = format!(
        "The instruction: {instruction}\n\nThe current draft:\n{}",
        serde_json::to_string(&req.draft).unwrap_or_default()
    );
    let (outcome, provider, model) = run_call(
        &state,
        org_id,
        user_id,
        QUOTE_SURFACE,
        &quote_change_prompt(),
        &user_message,
    )
    .await?;

    let mut issues = Vec::new();
    let parsed = extract_json(&outcome.content).and_then(|value| {
        match serde_json::from_value::<ModelChange>(value) {
            Ok(change) => Some(change),
            Err(error) => {
                issues.push(MoneyAuthoringIssue {
                    field: "change".into(),
                    message: format!("the answer was not a change proposal: {error}"),
                });
                None
            }
        }
    });
    let mut draft = req.draft;
    let mut operations: Vec<ChangeOperation> = Vec::new();
    let mut sentence = "Nothing changes until you apply.".to_string();
    if let Some(change) = parsed {
        if !change.sentence.trim().is_empty() {
            sentence = change.sentence.trim().to_string();
        }
        let (applied, rejected) = apply_quote_operations(&mut draft, change.operations);
        operations = applied;
        issues.extend(rejected);
    }
    let valid = !operations.is_empty() && issues.is_empty();
    if operations.is_empty() && issues.is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "change".into(),
            message: "I could not read a change in that sentence.".into(),
        });
    }
    Ok(Json(QuoteChangeResponse {
        valid,
        saved: false,
        sentence,
        operations,
        draft: valid.then_some(draft.clone()),
        resulting_total_minor: quote_total(&draft),
        currency: draft.currency,
        issues,
        usage: MoneyAuthorUsage {
            prompt_tokens: outcome.prompt_tokens,
            completion_tokens: outcome.completion_tokens,
        },
        model_info: MoneyAuthorModel { provider, model },
        error: outcome.error,
    }))
}

/// POST /api/contracts/author/change — propose edits to a contract draft.
#[utoipa::path(operation_id = "contract_author_change", post, path = "/api/contracts/author/change",
    tag = "contracts", security(("bearer" = [])), request_body = ContractChangeRequest,
    responses((status = 200, body = ContractChangeResponse)))]
pub async fn change_contract(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ContractChangeRequest>,
) -> ApiResult<Json<ContractChangeResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let instruction = req.instruction.trim().to_string();
    if instruction.is_empty() {
        return Err(ApiError::Validation("say what should change".into()));
    }
    let user_message = format!(
        "The instruction: {instruction}\n\nThe current draft:\n{}",
        serde_json::to_string(&req.draft).unwrap_or_default()
    );
    let (outcome, provider, model) = run_call(
        &state,
        org_id,
        user_id,
        CONTRACT_SURFACE,
        &contract_change_prompt(),
        &user_message,
    )
    .await?;

    let mut issues = Vec::new();
    let parsed = extract_json(&outcome.content).and_then(|value| {
        match serde_json::from_value::<ModelChange>(value) {
            Ok(change) => Some(change),
            Err(error) => {
                issues.push(MoneyAuthoringIssue {
                    field: "change".into(),
                    message: format!("the answer was not a change proposal: {error}"),
                });
                None
            }
        }
    });
    let mut draft = req.draft;
    let mut operations: Vec<ChangeOperation> = Vec::new();
    let mut sentence = "Nothing changes until you apply.".to_string();
    if let Some(change) = parsed {
        if !change.sentence.trim().is_empty() {
            sentence = change.sentence.trim().to_string();
        }
        let library = read_library(&state, org_id, user_id).await?;
        let (applied, rejected) =
            apply_contract_operations(&mut draft, change.operations, &library);
        operations = applied;
        issues.extend(rejected);
    }
    let valid = !operations.is_empty() && issues.is_empty();
    if operations.is_empty() && issues.is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "change".into(),
            message: "I could not read a change in that sentence.".into(),
        });
    }
    Ok(Json(ContractChangeResponse {
        valid,
        saved: false,
        sentence,
        operations,
        draft: valid.then_some(draft.clone()),
        resulting_total_minor: contract_total(&draft),
        currency: draft.currency,
        issues,
        usage: MoneyAuthorUsage {
            prompt_tokens: outcome.prompt_tokens,
            completion_tokens: outcome.completion_tokens,
        },
        model_info: MoneyAuthorModel { provider, model },
        error: outcome.error,
    }))
}

/// Apply the operations whose paths are allowed, in order, and report the ones
/// that were not. Returning both is the point: a person sees the rejected
/// sentence instead of silently losing half a request.
fn apply_quote_operations(
    draft: &mut QuoteDraft,
    operations: Vec<ChangeOperation>,
) -> (Vec<ChangeOperation>, Vec<MoneyAuthoringIssue>) {
    let mut applied = Vec::new();
    let mut issues = Vec::new();
    for mut operation in operations {
        operation.summary = operation.summary.trim().to_string();
        if operation.summary.is_empty() {
            operation.summary = operation.path.clone();
        }
        let path = operation.path.trim().to_string();
        let result = apply_quote_operation(draft, &path, &operation);
        match result {
            Ok(()) => {
                operation.path = path;
                applied.push(operation);
            }
            Err(message) => issues.push(MoneyAuthoringIssue {
                field: path,
                message,
            }),
        }
    }
    // The checks are facts about the draft as it now stands, so they are
    // rebuilt from it: a price that was set stops asking to be set.
    draft.checks = quote_checks(draft);
    (applied, issues)
}

fn as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number.round() as i64))
}

fn as_text(value: &Value) -> Option<String> {
    value.as_str().map(|text| text.to_string())
}

fn apply_quote_operation(
    draft: &mut QuoteDraft,
    path: &str,
    operation: &ChangeOperation,
) -> Result<(), String> {
    if path == "lines[+]" || operation.kind == "add" && path.is_empty() {
        let line: QuoteLineDraft = serde_json::from_value(operation.value.clone())
            .map_err(|_| "a new line needs a description, a quantity and a price".to_string())?;
        if line.description.trim().is_empty() || line.quantity_milli <= 0 {
            return Err("a new line needs a description and a quantity".into());
        }
        if line.unit_price_minor < 0 {
            return Err("a price cannot be negative".into());
        }
        draft.lines.push(QuoteLineDraft {
            source: "assumed".into(),
            source_span: None,
            catalog_item_id: None,
            price_assumed: line.price_assumed || line.unit_price_minor == 0,
            ..line
        });
        return Ok(());
    }
    match path {
        "title" => draft.title = as_text(&operation.value).ok_or("the title must be text")?,
        "notes" => draft.notes = as_text(&operation.value),
        "valid_for_days" => {
            draft.valid_for_days =
                Some(as_i64(&operation.value).ok_or("validity must be a number")?)
        }
        _ => {
            let (index, field) = parse_line_path(path)?;
            let line = draft
                .lines
                .get_mut(index)
                .ok_or_else(|| format!("there is no line {}", index + 1))?;
            match field {
                "description" => {
                    line.description =
                        as_text(&operation.value).ok_or("the description must be text")?
                }
                "quantity_milli" => {
                    let quantity =
                        as_i64(&operation.value).ok_or("the quantity must be a number")?;
                    if quantity <= 0 {
                        return Err("a quantity must be more than zero".into());
                    }
                    line.quantity_milli = quantity;
                    line.quantity_assumed = false;
                }
                "unit_price_minor" => {
                    let price = as_i64(&operation.value).ok_or("the price must be a number")?;
                    if price < 0 {
                        return Err("a price cannot be negative".into());
                    }
                    line.unit_price_minor = price;
                    line.price_assumed = false;
                }
                "optional" => {
                    line.optional = operation
                        .value
                        .as_bool()
                        .ok_or("optional must be true or false")?
                }
                "unit" => line.unit = as_text(&operation.value),
                other => return Err(format!("{other} is not a field I can change")),
            }
        }
    }
    Ok(())
}

fn parse_line_path(path: &str) -> Result<(usize, &str), String> {
    let rest = path
        .strip_prefix("lines[")
        .ok_or_else(|| format!("{path} is not a field I can change"))?;
    let (index, field) = rest
        .split_once("].")
        .ok_or_else(|| format!("{path} is not a field I can change"))?;
    let index: usize = index
        .parse()
        .map_err(|_| format!("{path} is not a field I can change"))?;
    Ok((index, field))
}

fn apply_contract_operations(
    draft: &mut ContractDraft,
    operations: Vec<ChangeOperation>,
    library: &[ClauseRow],
) -> (Vec<ChangeOperation>, Vec<MoneyAuthoringIssue>) {
    let mut applied = Vec::new();
    let mut issues = Vec::new();
    for mut operation in operations {
        operation.summary = operation.summary.trim().to_string();
        if operation.summary.is_empty() {
            operation.summary = operation.path.clone();
        }
        let path = operation.path.trim().to_string();
        match apply_contract_operation(draft, &path, &operation, library) {
            Ok(()) => {
                operation.path = path;
                applied.push(operation);
            }
            Err(message) => issues.push(MoneyAuthoringIssue {
                field: path,
                message,
            }),
        }
    }
    // Rebuilt from the draft as it now stands, so a cleared rate or a new
    // clause variant carries the check it earns.
    draft.checks = contract_checks(draft);
    (applied, issues)
}

fn apply_contract_operation(
    draft: &mut ContractDraft,
    path: &str,
    operation: &ChangeOperation,
    library: &[ClauseRow],
) -> Result<(), String> {
    if let Some(key) = path.strip_prefix("clause_parameters.") {
        let row = library
            .iter()
            .find(|row| row.key == key)
            .ok_or_else(|| format!("{key} is not a clause in your library"))?;
        if row.parameter_label.is_none() && row.standard_value.is_none() {
            return Err(format!("{key} has no value to fill"));
        }
        let value = as_text(&operation.value).ok_or("a clause value must be text")?;
        let mut updated = clause_ref(row, Some(value));
        updated.review_required = updated.status == "variant" || row.review_required;
        draft.clause_refs.retain(|clause| clause.key != row.key);
        draft.clause_refs.push(updated);
        draft.clause_refs.sort_by(|a, b| a.key.cmp(&b.key));
        draft.clauses = draft.clause_refs.iter().map(|c| c.title.clone()).collect();
        return Ok(());
    }
    match path {
        "name" => draft.name = as_text(&operation.value).ok_or("the name must be text")?,
        "notes" => draft.notes = as_text(&operation.value),
        "term_months" => {
            draft.term_months = Some(as_i64(&operation.value).ok_or("the term must be a number")?)
        }
        "effective_from" => {
            draft.effective_from = as_text(&operation.value).filter(|text| text.len() == 10)
        }
        "billing_frequency" => {
            draft.billing_frequency = as_text(&operation.value);
        }
        _ => {
            let rest = path
                .strip_prefix("pricing[")
                .ok_or_else(|| format!("{path} is not a field I can change"))?;
            let (index, field) = rest
                .split_once("].")
                .ok_or_else(|| format!("{path} is not a field I can change"))?;
            let index: usize = index
                .parse()
                .map_err(|_| format!("{path} is not a field I can change"))?;
            let line = draft
                .pricing
                .get_mut(index)
                .ok_or_else(|| format!("there is no pricing line {}", index + 1))?;
            match field {
                "label" => {
                    line.label = as_text(&operation.value).ok_or("the label must be text")?
                }
                "quantity_milli" => {
                    let quantity =
                        as_i64(&operation.value).ok_or("the quantity must be a number")?;
                    if quantity <= 0 {
                        return Err("a quantity must be more than zero".into());
                    }
                    line.quantity_milli = quantity;
                    line.quantity_assumed = false;
                }
                "unit_price_minor" => {
                    let price = as_i64(&operation.value).ok_or("the rate must be a number")?;
                    if price < 0 {
                        return Err("a rate cannot be negative".into());
                    }
                    line.unit_price_minor = price;
                    line.price_assumed = false;
                }
                other => return Err(format!("{other} is not a field I can change")),
            }
            draft.value_minor = Some(contract_total(draft));
        }
    }
    Ok(())
}

// --- handlers: upload -----------------------------------------------------

/// Content types the extractor reads as text. A PDF or a Word file is refused
/// by name rather than decoded into noise: saying "I cannot read that" is
/// better than drafting from binary rubbish.
fn is_text_type(content_type: &str, name: &str) -> bool {
    let lowered_type = content_type.to_lowercase();
    if lowered_type.starts_with("text/") {
        return true;
    }
    if matches!(
        lowered_type.as_str(),
        "application/json" | "application/xml" | "application/csv"
    ) {
        return true;
    }
    let lowered_name = name.to_lowercase();
    [".txt", ".md", ".csv", ".vtt", ".srt", ".json", ".log"]
        .iter()
        .any(|suffix| lowered_name.ends_with(suffix))
}

/// POST /api/money-authoring/sources — read an uploaded file as text.
///
/// It writes nothing: the text goes back to the panel, which hands it to the
/// author route like any other source. No second file store exists, and the
/// text is not kept after the response.
#[utoipa::path(operation_id = "money_authoring_upload_source", post, path = "/api/money-authoring/sources",
    tag = "quotes", security(("bearer" = [])), request_body = SourceUploadRequest,
    responses((status = 200, body = SourceUploadResponse)))]
pub async fn upload_source(
    State(_state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<SourceUploadRequest>,
) -> ApiResult<Json<SourceUploadResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;

    let refused = |reason: &str, message: &str| {
        Json(SourceUploadResponse {
            accepted: false,
            reason: Some(reason.into()),
            message: Some(message.into()),
            length: None,
            source: None,
        })
    };

    if !is_text_type(&req.content_type, &req.name) {
        return Ok(refused(
            "unsupported_type",
            "I can read text, Markdown, CSV and VTT or SRT transcripts. A PDF or Word file needs exporting to text first.",
        ));
    }
    let payload = req.data_base64.trim();
    if payload.is_empty() {
        return Ok(refused("empty", "That file was empty."));
    }
    let bytes = match base64::engine::general_purpose::STANDARD.decode(payload) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(refused("not_text", "That file did not decode as a file.")),
    };
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Ok(refused(
            "too_large",
            "Files must be 2 MB or smaller. Paste the relevant part instead.",
        ));
    }
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            return Ok(refused(
                "not_text",
                "That file is not text. Export it to text or paste the part that matters.",
            ))
        }
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(refused("empty", "That file had no readable text."));
    }
    if trimmed.chars().count() > MAX_SOURCE_CHARS {
        return Ok(refused(
            "too_large",
            &format!("A source can hold {MAX_SOURCE_CHARS} characters. Split the file or paste the part that matters."),
        ));
    }
    Ok(Json(SourceUploadResponse {
        accepted: true,
        reason: None,
        message: None,
        length: Some(trimmed.chars().count()),
        source: Some(AuthorSource {
            id: None,
            kind: "document".into(),
            name: Some(req.name),
            text: trimmed.to_string(),
        }),
    }))
}

// --- routes ---------------------------------------------------------------

/// The authoring legs share the quotes and contracts tags, so they mount from
/// one merge line beside those modules.
pub fn routes(gen: crate::routes::ApiGeneration) -> axum::Router<AppState> {
    let base = gen.base();
    axum::Router::new()
        .route(
            &format!("{base}/quotes/author"),
            axum::routing::post(author_quote),
        )
        .route(
            &format!("{base}/quotes/author/change"),
            axum::routing::post(change_quote),
        )
        .route(
            &format!("{base}/quotes/extract"),
            axum::routing::post(extract_quote),
        )
        .route(
            &format!("{base}/contracts/author"),
            axum::routing::post(author_contract),
        )
        .route(
            &format!("{base}/contracts/author/change"),
            axum::routing::post(change_contract),
        )
        .route(
            &format!("{base}/contracts/extract"),
            axum::routing::post(extract_contract),
        )
        .route(
            &format!("{base}/contracts/clauses"),
            axum::routing::get(list_clauses),
        )
        .route(
            &format!("{base}/money-authoring/sources"),
            axum::routing::post(upload_source),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(description: &str, quantity_milli: i64, price: i64) -> QuoteLineDraft {
        QuoteLineDraft {
            description: description.into(),
            quantity_milli,
            unit_price_minor: price,
            unit: Some("each".into()),
            section: None,
            optional: false,
            catalog_item_id: None,
            source: "notes".into(),
            source_span: None,
            quantity_assumed: false,
            price_assumed: false,
        }
    }

    fn source(id: &str, text: &str) -> AuthorSource {
        AuthorSource {
            id: Some(id.into()),
            kind: "typed".into(),
            name: None,
            text: text.into(),
        }
    }

    fn empty_draft() -> QuoteDraft {
        QuoteDraft {
            title: "Wi-Fi refresh".into(),
            currency: "USD".into(),
            valid_for_days: Some(30),
            notes: None,
            client_id: None,
            client_name: None,
            budget_minor: None,
            lines: Vec::new(),
            assumptions: Vec::new(),
            understanding: None,
            checks: Vec::new(),
        }
    }

    fn clause_row(key: &str, standard: &str) -> ClauseRow {
        ClauseRow {
            id: Uuid::now_v7(),
            org_id: None,
            key: key.into(),
            title: format!("{key} clause"),
            body: format!("{key} body"),
            version: 1,
            status: "standard".into(),
            review_required: false,
            parameter_label: Some("Value".into()),
            standard_value: Some(standard.into()),
        }
    }

    fn contract_draft() -> ContractDraft {
        ContractDraft {
            name: "Managed Silver".into(),
            contract_type: "recurring".into(),
            billing_frequency: Some("monthly".into()),
            value_minor: None,
            currency: "USD".into(),
            term_months: Some(12),
            effective_from: None,
            notes: None,
            clauses: Vec::new(),
            clause_refs: Vec::new(),
            pricing: Vec::new(),
            client_id: None,
            client_name: None,
            assumptions: Vec::new(),
            understanding: None,
            checks: Vec::new(),
        }
    }

    #[test]
    fn a_fenced_answer_still_parses() {
        let value = extract_json("```json\n{\"title\":\"x\"}\n```").expect("parse");
        assert_eq!(value["title"], "x");
    }

    #[test]
    fn an_unpriced_line_is_kept_at_zero_with_an_assumption() {
        let mut draft = empty_draft();
        draft.lines.push(line("  Replace switch  ", 1000, -5));
        let issues = validate_quote(&mut draft, &[], &[], &[]);
        assert!(issues.is_empty());
        assert_eq!(draft.currency, "USD");
        assert_eq!(draft.lines[0].description, "Replace switch");
        assert_eq!(draft.lines[0].unit_price_minor, 0);
        assert!(draft.lines[0].price_assumed);
        assert_eq!(draft.lines[0].source, "assumed");
        assert_eq!(draft.assumptions.len(), 1);
    }

    #[test]
    fn a_quote_with_no_work_is_an_issue_not_a_draft() {
        let mut draft = empty_draft();
        let issues = validate_quote(&mut draft, &[], &[], &[]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].field, "lines");
    }

    #[test]
    fn a_line_without_found_span_is_assumed_not_from_notes() {
        let mut draft = empty_draft();
        let mut row = line("Install the switch", 1000, 5000);
        row.source = "notes".into();
        row.source_span = Some(SourceSpan {
            source_id: "typed-1".into(),
            start: 0,
            end: 10,
        });
        draft.lines.push(row);
        // The span names a source that is not in this request.
        validate_quote(&mut draft, &[], &[], &[]);
        assert!(draft.lines[0].source_span.is_none());
        assert_eq!(draft.lines[0].source, "assumed");
    }

    #[test]
    fn a_quoted_span_is_located_by_character_not_trusted() {
        let sources = vec![source(
            "typed-1",
            "Replace the 6 access points w/ Ubiquiti.",
        )];
        let span = resolve_span(&sources, Some("6 access points")).expect("span");
        assert_eq!(span.start, 12);
        assert_eq!(span.end, 27);
        assert_eq!(
            sources[0]
                .text
                .chars()
                .skip(span.start)
                .take(span.end - span.start)
                .collect::<String>(),
            "6 access points"
        );
    }

    #[test]
    fn a_span_shorter_than_six_characters_is_refused() {
        let sources = vec![source("typed-1", "Six APs, then a switch.")];
        assert!(resolve_span(&sources, Some("APs")).is_none());
    }

    #[test]
    fn half_even_rounding_matches_the_quote_line_rule() {
        // 1.5 x $10.00 = $15.00 exactly.
        assert_eq!(line_amount_minor(1500, 1000), 1500);
        // 0.5 cents rounds to the even neighbour: 0.
        assert_eq!(line_amount_minor(1000, 1), 1);
        assert_eq!(line_amount_minor(500, 1), 0);
        assert_eq!(line_amount_minor(1500, 1), 2);
    }

    #[test]
    fn an_optional_line_is_priced_but_not_in_the_total() {
        let mut draft = empty_draft();
        draft.lines.push(line("Switch", 1000, 50_000));
        let mut optional = line("Support plan", 1000, 20_000);
        optional.optional = true;
        draft.lines.push(optional);
        assert_eq!(quote_total(&draft), 50_000);
    }

    #[test]
    fn the_catalogue_sets_the_price_and_the_source() {
        let catalog = vec![CatalogRow {
            id: Uuid::now_v7(),
            name: "24 port PoE switch".into(),
            unit: "each".into(),
            unit_price_minor: 54_000,
        }];
        let mut draft = empty_draft();
        draft.lines.push(line("24 port PoE switch", 1000, 0));
        validate_quote(&mut draft, &[], &catalog, &[]);
        assert_eq!(draft.lines[0].unit_price_minor, 54_000);
        assert_eq!(draft.lines[0].source, "catalogue");
        assert!(!draft.lines[0].price_assumed);
        assert_eq!(draft.lines[0].catalog_item_id, Some(catalog[0].id));
        // The price came from the catalogue, and the check list says so.
        let checks = quote_checks(&draft);
        assert!(checks
            .iter()
            .any(|c| c.contains("comes from your catalogue")));
    }

    #[test]
    fn a_single_generic_word_does_not_match_the_catalogue() {
        let catalog = vec![CatalogRow {
            id: Uuid::now_v7(),
            name: "Support plan".into(),
            unit: "each".into(),
            unit_price_minor: 19_500,
        }];
        assert!(match_catalog("plan the office move", &catalog).is_none());
        assert!(match_catalog("Support plan, access points", &catalog).is_some());
    }

    #[test]
    fn the_budget_is_read_from_the_notes_and_never_applied() {
        assert_eq!(
            detect_budget("Budget around 5k. Half up front."),
            Some(500_000)
        );
        assert_eq!(detect_budget("budget of $5,000"), Some(500_000));
        assert_eq!(detect_budget("no numbers here"), None);
        let mut draft = empty_draft();
        draft.lines.push(line("Switch", 1000, 525_400));
        draft.budget_minor = Some(500_000);
        let checks = quote_checks(&draft);
        assert!(checks.iter().any(|c| c.contains("over the budget")));
        // The total is untouched by the budget.
        assert_eq!(quote_total(&draft), 525_400);
    }

    #[test]
    fn the_live_chips_read_the_words_the_pack_shows() {
        let text =
            "Acme kickoff w/ John + Zanele, 30 Sep. 4th floor wifi at Cape Town HQ is rubbish. \
                    Replace the 6 access points, new 24 port PoE switch, re-cable 2 runs. \
                    Site survey first, half a day. Install over a weekend, 2nd half of Nov. \
                    John wants zero downtime in work hours. Stellenbosch stays as is. \
                    They'd like an optional 3 yr support plan. Budget around 5k. Valid 30 days.";
        let fields = extract_fields(text, &["Acme Corporation".into()]);
        let values: Vec<&str> = fields.iter().map(|f| f.value.as_str()).collect();
        assert!(values.contains(&"Acme Corporation"));
        assert!(values.contains(&"6 access points"));
        assert!(values.contains(&"2 runs"));
        assert!(values.contains(&"A site survey"));
        assert!(values.contains(&"30 days"));
        assert!(fields.iter().any(|f| f.key == "exclusion"));
        assert!(fields.iter().any(|f| f.key == "budget"));
    }

    #[test]
    fn extraction_never_repeats_a_chip() {
        let fields = extract_fields("6 access points, 6 access points again", &[]);
        let count = fields
            .iter()
            .filter(|f| f.value == "6 access points")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn a_change_operation_outside_the_allowlist_is_reported() {
        let mut draft = empty_draft();
        draft.lines.push(line("Switch", 1000, 50_000));
        let (applied, issues) = apply_quote_operations(
            &mut draft,
            vec![
                ChangeOperation {
                    path: "currency".into(),
                    value: json!("EUR"),
                    summary: "Change currency".into(),
                    kind: "edit".into(),
                },
                ChangeOperation {
                    path: "lines[0].unit_price_minor".into(),
                    value: json!(47_500),
                    summary: "Take 5% off".into(),
                    kind: "edit".into(),
                },
            ],
        );
        assert_eq!(applied.len(), 1);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].field, "currency");
        assert_eq!(draft.lines[0].unit_price_minor, 47_500);
        assert_eq!(quote_total(&draft), 47_500);
    }

    #[test]
    fn a_change_can_add_a_line_and_the_total_follows() {
        let mut draft = empty_draft();
        draft.lines.push(line("Switch", 1000, 50_000));
        let (applied, issues) = apply_quote_operations(
            &mut draft,
            vec![ChangeOperation {
                path: "lines[+]".into(),
                value: json!({"description": "Cabling", "quantity_milli": 2000, "unit_price_minor": 16000}),
                summary: "Add cabling".into(),
                kind: "add".into(),
            }],
        );
        assert!(issues.is_empty());
        assert_eq!(applied.len(), 1);
        assert_eq!(draft.lines.len(), 2);
        assert_eq!(draft.lines[1].source, "assumed");
        assert_eq!(quote_total(&draft), 82_000);
    }

    #[test]
    fn a_negative_price_change_is_refused_not_applied() {
        let mut draft = empty_draft();
        draft.lines.push(line("Switch", 1000, 50_000));
        let (applied, issues) = apply_quote_operations(
            &mut draft,
            vec![ChangeOperation {
                path: "lines[0].unit_price_minor".into(),
                value: json!(-100),
                summary: "Make it free".into(),
                kind: "edit".into(),
            }],
        );
        assert!(applied.is_empty());
        assert_eq!(issues.len(), 1);
        assert_eq!(draft.lines[0].unit_price_minor, 50_000);
    }

    #[test]
    fn a_price_the_notes_do_not_state_is_not_kept() {
        let sources = vec![source("typed-1", "Install the switch over the weekend.")];
        let mut draft = empty_draft();
        let mut row = line("Install the switch", 1000, 50_000);
        row.source_span = Some(SourceSpan {
            source_id: "typed-1".into(),
            start: 0,
            end: 18,
        });
        draft.lines.push(row);
        validate_quote(&mut draft, &sources, &[], &[]);
        assert_eq!(draft.lines[0].unit_price_minor, 0);
        assert!(draft.lines[0].price_assumed);
        assert!(draft
            .assumptions
            .iter()
            .any(|line| line.contains("do not state")));
    }

    #[test]
    fn a_price_the_notes_state_is_kept() {
        let sources = vec![source(
            "typed-1",
            "Replace the switch, $500 each. Budget around 5k.",
        )];
        let mut draft = empty_draft();
        draft.lines.push(line("Replace the switch", 1000, 50_000));
        validate_quote(&mut draft, &sources, &[], &[]);
        assert_eq!(draft.lines[0].unit_price_minor, 50_000);
        assert!(!draft.lines[0].price_assumed);
    }

    #[test]
    fn an_amount_is_read_as_a_number_not_as_a_fragment() {
        assert!(source_states_amount("$5,000 for the work", 500_000));
        assert!(source_states_amount("5k for the work", 500_000));
        assert!(source_states_amount("2 x $250", 25_000));
        // The tail of a longer number is not that number.
        assert!(!source_states_amount("1,500 units", 50_000));
        assert!(!source_states_amount("$500.99", 50_000));
    }

    #[test]
    fn a_catalogue_number_that_conflicts_is_not_a_match() {
        let catalog = vec![CatalogRow {
            id: Uuid::now_v7(),
            name: "48 port PoE switch".into(),
            unit: "each".into(),
            unit_price_minor: 88_000,
        }];
        assert!(match_catalog("24 port PoE switch", &catalog).is_none());
        assert!(match_catalog("48 port PoE switch", &catalog).is_some());
        let mut draft = empty_draft();
        draft.lines.push(line("24 port PoE switch", 1000, 0));
        validate_quote(&mut draft, &[], &catalog, &[]);
        // No authoritative price is assigned to an ambiguous line.
        assert_eq!(draft.lines[0].catalog_item_id, None);
        assert!(draft.lines[0].price_assumed);
    }

    #[test]
    fn a_clause_without_a_parameter_takes_no_value() {
        let row = ClauseRow {
            id: Uuid::now_v7(),
            org_id: None,
            key: "scope".into(),
            title: "Services and scope".into(),
            body: "The supplier provides the services.".into(),
            version: 1,
            status: "standard".into(),
            review_required: false,
            parameter_label: None,
            standard_value: None,
        };
        let rendered = clause_ref(&row, Some("anything at all".into()));
        assert_eq!(rendered.parameter_value, None);
        assert_eq!(rendered.status, "standard");
        assert!(!rendered.review_required);
    }

    #[test]
    fn a_contract_total_the_model_wrote_is_discarded() {
        let sources = vec![source("typed-1", "12 users at $38 each, billed monthly.")];
        let mut draft = contract_draft();
        draft.value_minor = Some(500_000);
        draft.pricing.push(ContractPriceLine {
            label: "Users".into(),
            quantity_milli: 12_000,
            unit_price_minor: 3_800,
            quantity_assumed: false,
            price_assumed: false,
            source_span: Some(SourceSpan {
                source_id: "typed-1".into(),
                start: 0,
                end: 29,
            }),
        });
        validate_contract(
            &mut draft,
            &sources,
            &[],
            &[],
            &[],
            &std::collections::BTreeMap::new(),
        );
        assert_eq!(draft.value_minor, Some(45_600));

        // No rates means no value to stand behind, whatever the model said.
        draft.pricing.clear();
        draft.value_minor = Some(500_000);
        validate_contract(
            &mut draft,
            &sources,
            &[],
            &[],
            &[],
            &std::collections::BTreeMap::new(),
        );
        assert_eq!(draft.value_minor, None);
        assert!(draft.checks.iter().any(|check| check.contains("no rate")));
    }

    #[test]
    fn a_contract_rate_the_words_do_not_state_is_assumed() {
        let mut draft = contract_draft();
        draft.pricing.push(ContractPriceLine {
            label: "Users".into(),
            quantity_milli: 12_000,
            unit_price_minor: 3_800,
            quantity_assumed: false,
            price_assumed: false,
            source_span: None,
        });
        // No source states $38, so the rate is not kept as a stated amount.
        validate_contract(
            &mut draft,
            &[source("typed-1", "Managed service for the office.")],
            &[],
            &[],
            &[],
            &std::collections::BTreeMap::new(),
        );
        assert_eq!(draft.pricing[0].unit_price_minor, 0);
        assert!(draft.pricing[0].price_assumed);
        assert!(draft
            .checks
            .iter()
            .any(|check| check.contains("needs setting")));
    }

    #[test]
    fn a_negative_price_on_an_added_line_is_refused() {
        let mut draft = empty_draft();
        let (applied, issues) = apply_quote_operations(
            &mut draft,
            vec![ChangeOperation {
                path: "lines[+]".into(),
                value: json!({
                    "description": "Cabling",
                    "quantity_milli": 1000,
                    "unit_price_minor": -100
                }),
                summary: "Add cabling".into(),
                kind: "add".into(),
            }],
        );
        assert!(applied.is_empty());
        assert_eq!(issues.len(), 1);
        assert!(draft.lines.is_empty());
    }

    #[test]
    fn a_price_change_rebuilds_the_check_list() {
        let mut draft = empty_draft();
        let mut assumed = line("Switch", 1000, 0);
        assumed.price_assumed = true;
        draft.lines.push(assumed);
        draft.checks = quote_checks(&draft);
        assert!(draft
            .checks
            .iter()
            .any(|check| check.contains("was not in your notes")));

        let (applied, issues) = apply_quote_operations(
            &mut draft,
            vec![ChangeOperation {
                path: "lines[0].unit_price_minor".into(),
                value: json!(50_000),
                summary: "Set the switch price".into(),
                kind: "edit".into(),
            }],
        );
        assert!(issues.is_empty());
        assert_eq!(applied.len(), 1);
        assert!(!draft.lines[0].price_assumed);
        assert!(draft
            .checks
            .iter()
            .all(|check| !check.contains("was not in your notes")));
    }

    #[test]
    fn an_unknown_contract_type_falls_back_and_says_nothing_it_does_not_know() {
        let mut draft = ContractDraft {
            name: "Managed".into(),
            contract_type: "gold".into(),
            billing_frequency: Some("fortnightly".into()),
            value_minor: None,
            currency: "usd".into(),
            term_months: Some(12),
            effective_from: None,
            notes: None,
            clauses: Vec::new(),
            clause_refs: Vec::new(),
            pricing: Vec::new(),
            client_id: None,
            client_name: None,
            assumptions: Vec::new(),
            understanding: None,
            checks: Vec::new(),
        };
        let issues = validate_contract(
            &mut draft,
            &[],
            &[],
            &[],
            &[],
            &std::collections::BTreeMap::new(),
        );
        assert!(issues.is_empty());
        assert_eq!(draft.contract_type, "recurring");
        assert_eq!(draft.billing_frequency.as_deref(), Some("monthly"));
        assert_eq!(draft.currency, "USD");
        assert!(draft.value_minor.is_none());
    }

    #[test]
    fn a_contract_gets_the_library_clauses_and_a_changed_one_needs_review() {
        let library = vec![
            clause_row("term", "12 months"),
            clause_row("renewal_notice", "90 days"),
        ];
        let mut draft = ContractDraft {
            name: "Managed Silver".into(),
            contract_type: "recurring".into(),
            billing_frequency: Some("monthly".into()),
            value_minor: None,
            currency: "USD".into(),
            term_months: Some(12),
            effective_from: Some("2026-11-01".into()),
            notes: None,
            clauses: Vec::new(),
            clause_refs: Vec::new(),
            pricing: Vec::new(),
            client_id: None,
            client_name: None,
            assumptions: Vec::new(),
            understanding: None,
            checks: Vec::new(),
        };
        let mut parameters = std::collections::BTreeMap::new();
        parameters.insert("renewal_notice".to_string(), "60 days".to_string());
        validate_contract(&mut draft, &[], &[], &library, &[], &parameters);
        // No keys named: the approved baseline, not an empty contract.
        assert_eq!(draft.clause_refs.len(), 2);
        let notice = draft
            .clause_refs
            .iter()
            .find(|clause| clause.key == "renewal_notice")
            .expect("notice clause");
        assert_eq!(notice.status, "variant");
        assert!(notice.review_required);
        assert!(draft
            .checks
            .iter()
            .any(|check| check.contains("Legal review is advised")));
        // The text is the library's, never a model sentence.
        assert_eq!(notice.body, "renewal_notice body");
    }

    #[test]
    fn a_contract_total_is_built_from_quantities_and_rates() {
        // The rates are the notes' own, so they survive verification and the
        // total is the arithmetic over them.
        let sources = vec![source(
            "typed-1",
            "12 users at $38 each, 14 devices at $12 each.",
        )];
        let mut draft = ContractDraft {
            name: "Managed Silver".into(),
            contract_type: "recurring".into(),
            billing_frequency: Some("monthly".into()),
            value_minor: None,
            currency: "USD".into(),
            term_months: Some(12),
            effective_from: None,
            notes: None,
            clauses: Vec::new(),
            clause_refs: Vec::new(),
            pricing: vec![
                ContractPriceLine {
                    label: "Users".into(),
                    quantity_milli: 12_000,
                    unit_price_minor: 3_800,
                    quantity_assumed: false,
                    price_assumed: false,
                    source_span: None,
                },
                ContractPriceLine {
                    label: "Devices".into(),
                    quantity_milli: 14_000,
                    unit_price_minor: 1_200,
                    quantity_assumed: false,
                    price_assumed: false,
                    source_span: None,
                },
            ],
            client_id: None,
            client_name: None,
            assumptions: Vec::new(),
            understanding: None,
            checks: Vec::new(),
        };
        validate_contract(
            &mut draft,
            &sources,
            &[],
            &[],
            &[],
            &std::collections::BTreeMap::new(),
        );
        assert_eq!(draft.value_minor, Some(62_400));
    }

    #[test]
    fn a_contract_change_to_a_known_clause_marks_it_for_review() {
        let library = vec![clause_row("renewal_notice", "90 days")];
        let mut draft = ContractDraft {
            name: "Managed".into(),
            contract_type: "recurring".into(),
            billing_frequency: Some("monthly".into()),
            value_minor: Some(62_400),
            currency: "USD".into(),
            term_months: Some(12),
            effective_from: None,
            notes: None,
            clauses: Vec::new(),
            clause_refs: vec![clause_ref(&library[0], Some("90 days".into()))],
            pricing: Vec::new(),
            client_id: None,
            client_name: None,
            assumptions: Vec::new(),
            understanding: None,
            checks: Vec::new(),
        };
        let (applied, issues) = apply_contract_operations(
            &mut draft,
            vec![ChangeOperation {
                path: "clause_parameters.renewal_notice".into(),
                value: json!("60 days"),
                summary: "Move the notice period to 60 days".into(),
                kind: "edit".into(),
            }],
            &library,
        );
        assert!(issues.is_empty());
        assert_eq!(applied.len(), 1);
        assert_eq!(
            draft.clause_refs[0].parameter_value.as_deref(),
            Some("60 days")
        );
        assert!(draft.clause_refs[0].review_required);
    }

    #[test]
    fn a_clause_key_not_in_the_library_is_refused() {
        let mut draft = ContractDraft {
            name: "Managed".into(),
            contract_type: "recurring".into(),
            billing_frequency: None,
            value_minor: None,
            currency: "USD".into(),
            term_months: None,
            effective_from: None,
            notes: None,
            clauses: Vec::new(),
            clause_refs: Vec::new(),
            pricing: Vec::new(),
            client_id: None,
            client_name: None,
            assumptions: Vec::new(),
            understanding: None,
            checks: Vec::new(),
        };
        let (applied, issues) = apply_contract_operations(
            &mut draft,
            vec![ChangeOperation {
                path: "clause_parameters.invented_liability_cap".into(),
                value: json!("unlimited"),
                summary: "Add a liability cap".into(),
                kind: "edit".into(),
            }],
            &[clause_row("term", "12 months")],
        );
        assert!(applied.is_empty());
        assert_eq!(issues.len(), 1);
        assert!(draft.clause_refs.is_empty());
    }

    #[test]
    fn an_org_clause_row_wins_over_the_shipped_one() {
        let mut shipped = clause_row("term", "12 months");
        shipped.version = 3;
        let mut own = clause_row("term", "24 months");
        own.org_id = Some(Uuid::now_v7());
        own.version = 2;
        let picked = pick_library(vec![shipped, own]);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].standard_value.as_deref(), Some("24 months"));
    }

    #[test]
    fn a_question_keeps_a_chosen_answer_only_when_it_is_an_option() {
        let understanding = validate_understanding(
            ModelUnderstanding {
                statement: String::new(),
                facts: Vec::new(),
                questions: vec![
                    DraftQuestion {
                        id: String::new(),
                        text: "Who approves this?".into(),
                        options: vec!["Grace".into(), "John".into()],
                        chosen: Some("Nobody".into()),
                    },
                    DraftQuestion {
                        id: "empty".into(),
                        text: "Which weekend?".into(),
                        options: vec!["".into()],
                        chosen: None,
                    },
                ],
                assumptions: vec!["  ".into()],
            },
            "A quote for Acme.",
        );
        assert_eq!(understanding.statement, "A quote for Acme.");
        assert_eq!(understanding.questions[0].chosen.as_deref(), Some("Grace"));
        assert_eq!(understanding.questions[0].id, "who-approves-this");
        assert_eq!(understanding.questions[1].options, vec!["Yes", "No"]);
        assert!(understanding.assumptions.is_empty());
    }

    #[test]
    fn a_backwards_compatible_transcript_only_request_still_reads() {
        let sources = read_sources("Two switches and a day of labour", &[]).expect("sources");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, "typed");
        assert_eq!(sources[0].id.as_deref(), Some("typed-1"));
    }

    #[test]
    fn an_empty_request_is_a_typed_validation_error() {
        let error = read_sources("   ", &[]).expect_err("must refuse");
        assert!(matches!(error, ApiError::Validation(_)));
    }

    #[test]
    fn too_many_sources_are_refused() {
        let many: Vec<AuthorSource> = (0..7).map(|i| source(&format!("s{i}"), "note")).collect();
        let error = read_sources("", &many).expect_err("must refuse");
        assert!(matches!(error, ApiError::Validation(_)));
    }

    #[test]
    fn only_text_uploads_are_read() {
        assert!(is_text_type("text/plain", "notes.txt"));
        assert!(is_text_type("", "call.srt"));
        assert!(!is_text_type("application/pdf", "brief.pdf"));
        assert!(!is_text_type(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "notes.docx"
        ));
    }
}
