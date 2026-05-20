//! SP6 — corpus text taxonomy: PII-safe placeholder grammar + conditional
//! template pools keyed on (source, account-class).
//!
//! Replaces SP4.4's verbatim source-keyed `TextTemplatePrior`. Generated text
//! is synthetic-by-construction: tokenized real templates whose PII spans are
//! fillable placeholders. Line text is conditioned on (source, account-class);
//! header text is source-keyed; CoA descriptions are per-account templates
//! filled once per run.

use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// A PII-placeholder kind the generator must resolve to a concrete value.
/// Structural placeholders (`{year}`, `{quarter}`, `{month}`, `{date}`,
/// `{digits}`) are NOT in this enum — `PlaceholderGrammar::fill` handles those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PiiPlaceholderKind {
    Patient,
    Person,
    Company,
    Street,
}

impl PiiPlaceholderKind {
    /// The placeholder token as it appears in a template string.
    pub fn token(self) -> &'static str {
        match self {
            PiiPlaceholderKind::Patient => "{patient}",
            PiiPlaceholderKind::Person => "{person}",
            PiiPlaceholderKind::Company => "{company}",
            PiiPlaceholderKind::Street => "{street}",
        }
    }

    /// Parse a placeholder token to its kind. `None` for structural or unknown.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "{patient}" => Some(PiiPlaceholderKind::Patient),
            "{person}" => Some(PiiPlaceholderKind::Person),
            "{company}" => Some(PiiPlaceholderKind::Company),
            "{street}" => Some(PiiPlaceholderKind::Street),
            _ => None,
        }
    }
}

/// Resolves a PII-placeholder kind to a concrete value. Implemented by the
/// generator (wired to master data) and by `SyntheticExampleResolver` (used at
/// extraction time, where master data does not exist).
pub trait PlaceholderResolver {
    /// Resolve a PII-placeholder kind to a concrete value.
    fn resolve(&mut self, kind: PiiPlaceholderKind, rng: &mut dyn rand::Rng) -> String;
}

/// A built-in resolver emitting obviously-synthetic tokens. Used to produce
/// `TemplateEntry::synthetic_example` at extraction time and in tests.
pub struct SyntheticExampleResolver;

impl PlaceholderResolver for SyntheticExampleResolver {
    fn resolve(&mut self, kind: PiiPlaceholderKind, _rng: &mut dyn rand::Rng) -> String {
        match kind {
            PiiPlaceholderKind::Patient => "Example Patient".to_string(),
            PiiPlaceholderKind::Person => "Example Person".to_string(),
            PiiPlaceholderKind::Company => "Example GmbH".to_string(),
            PiiPlaceholderKind::Street => "Example Street 1".to_string(),
        }
    }
}

/// One residual-PII scan hit.
#[derive(Debug, Clone, PartialEq)]
pub struct PiiHit {
    /// Static label of the pattern that matched (e.g. `"patient_record"`).
    pub pattern: &'static str,
    /// The substring that matched.
    pub matched: String,
}

/// A single PII-safe text template.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplateEntry {
    /// Tokenized, PII-safe template string.
    pub template: String,
    /// Probability mass within the owning pool (renormalised after filtering).
    pub probability: f64,
    /// The template run through `fill` once at extraction time with a
    /// fixed-seed RNG and `SyntheticExampleResolver` — a debug/audit example
    /// carrying ZERO corpus content. Replaces SP4.4's verbatim `example` field.
    pub synthetic_example: String,
}

/// A weighted pool of templates for one `(source, class)` or `source` key.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplatePool {
    pub templates: Vec<TemplateEntry>,
    /// Total observations underpinning the pool (pre-truncation).
    pub n: usize,
}

/// Extraction metadata for a `TextTaxonomyPrior`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TaxonomyMeta {
    pub min_occurrences: usize,
    pub max_templates_per_pool: usize,
    /// Class-granularity tier used for `line_pools` keys (e.g. `"iso21378_l2"`).
    pub class_tier: String,
    pub n_client_inputs: usize,
}

/// SP6 — corpus text taxonomy prior. Replaces `TextTemplatePrior`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextTaxonomyPrior {
    /// Line text keyed on the flattened string `"SOURCE|CLASS"`. `CLASS` is the
    /// ISO 21378 Level-2 account class; lines whose account has no resolvable
    /// class are grouped under `"SOURCE|_unknown_"`.
    pub line_pools: BTreeMap<String, TemplatePool>,
    /// Header text keyed on source only (a JE header has no single account).
    pub header_pools: BTreeMap<String, TemplatePool>,
    /// CoA description templates keyed on account number — one per account.
    pub coa_pools: BTreeMap<String, TemplateEntry>,
    /// Extraction metadata.
    pub meta: TaxonomyMeta,
}

impl TextTaxonomyPrior {
    /// Sentinel class component used when a line's account has no resolvable
    /// ISO 21378 class.
    pub const UNKNOWN_CLASS: &'static str = "_unknown_";

    /// Build the flattened `"SOURCE|CLASS"` key used by `line_pools`.
    pub fn line_key(source: &str, account_class: &str) -> String {
        format!("{source}|{account_class}")
    }
}

/// Stateless tokenize / fill / scan engine. No dependency on the generator or
/// fingerprint crates — locale and master-data wiring arrive via a
/// `PlaceholderResolver` at fill time.
pub struct PlaceholderGrammar;

// --- residual-PII scan + tokenize Phase-A statics ---
//
// The `.unwrap()` on each regex is sound: the literals are compile-time
// constants whose well-formedness is pinned by the test suite. Grouped into
// a private submodule so the crate-level `#![deny(clippy::unwrap_used)]`
// is bypassed with a single module-level `#![allow]` rather than ten
// per-static attributes.

mod scan_patterns {
    #![allow(clippy::unwrap_used)]

    use regex::Regex;
    use std::sync::LazyLock;

    /// Patient record marker: `G:dd.dd.dd`. Presence implies an un-stripped name.
    pub(super) static RE_PATIENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"G:\s*\d{2}\.\d{2}\.\d{2}").unwrap());
    /// `*Lastname,Firstname` star record, anchored at start.
    pub(super) static RE_PERSON_STAR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\*[A-ZÄÖÜ][\w\u{00C0}-\u{017F}.'\-]+\s*,\s*[A-ZÄÖÜ]").unwrap()
    });
    /// Honorific / title followed by a name.
    pub(super) static RE_TITLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(Prof|Dr|Dipl|Pfr|Pfarrer|Herr|Frau|Hr|Fr|med|iur|lic)\.\s").unwrap()
    });
    /// `Initial. Surname` (e.g. `U. Frey`).
    pub(super) static RE_INITIAL_SURNAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b[A-ZÄÖÜ]\.\s*[A-ZÄÖÜ][a-zäöüß]{2,}\b").unwrap());
    /// `Surname Initial.` (e.g. `Frey U.`, `Mueller H.`). The trailing
    /// `[A-Z].` must be followed by whitespace or end-of-string, so that
    /// legal-entity abbreviations like `Europe B.V.` / `Suisse S.A.` /
    /// `Nespresso S.A.` do NOT match (the period there is followed by
    /// another capital letter — part of the abbreviation, not a name initial).
    /// Pre-T16 corpus scan found ~90% of `surname_initial` raw hits were
    /// legal-entity suffixes, not person names.
    pub(super) static RE_SURNAME_INITIAL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b[A-ZÄÖÜ][a-zäöüß]{2,}\s+[A-ZÄÖÜ]\.(?:\s|$)").unwrap());

    // --- tokenize Phase-A statics ---

    /// `dd.mm.yy` date triplet inside a patient `G:`/`E:`/`A:` record.
    pub(super) static RE_GEA_DATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([GEA]):\s*\d{2}\.\d{2}\.\d{2}").unwrap());
    /// Street address: a capitalised word ending in a street-type suffix, then
    /// a number. Case-insensitive on the suffix.
    pub(super) static RE_STREET: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b[A-ZÄÖÜ][\w\u{00C0}-\u{017F}.\-]*(?:str\.|strasse|gasse|weg|platz)\s*\d+[A-Za-z]?\b").unwrap()
    });
    /// 4-digit year 19xx / 20xx not embedded in a longer digit run.
    pub(super) static RE_YEAR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(?:19|20)\d{2}\b").unwrap());
    /// Quarter marker Q1–Q4 (case-insensitive), not followed by another digit.
    pub(super) static RE_QUARTER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\bQ[1-4]\b").unwrap());
    /// Run of >=4 digits.
    pub(super) static RE_DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{4,}").unwrap());

    /// A run of >=2 capitalised, whitespace-separated tokens (Unicode-aware).
    /// Candidate person-name span; confirmed only if a token is a known given
    /// name (see `given_names`). German capitalises all nouns, so case alone
    /// can't separate a surname from a common noun — the given-name gazetteer
    /// is the anchor, and we redact the whole run (safe over-redaction) rather
    /// than risk leaking the adjacent surname.
    pub(super) static RE_NAME_RUN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\p{Lu}[\p{L}.'\-]*(?:\s+\p{Lu}[\p{L}.'\-]*)+").unwrap());
}

/// Given-name gazetteer for Phase-A name tokenization. Generic given names
/// (country-pack union + Swiss/DE/FR/IT supplement) — NOT PII, a name
/// dictionary like city names. Used to anchor `Firstname Lastname` detection
/// that the initial/title/patient regexes miss.
mod given_names {
    use std::collections::HashSet;
    use std::sync::LazyLock;

    /// Normalize a token for gazetteer lookup: lowercase + drop umlaut/accent
    /// characters entirely. The corpus text has umlauts STRIPPED (e.g.
    /// `Jürg`→`Jrg`, `Rückstellung`→`Rckstellung`), so a gazetteer entry with
    /// proper umlauts would never match the corpus form — normalizing both
    /// sides the same way bridges that.
    pub(super) fn normalize(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                // Umlauts: the corpus drops them entirely (Jürg→Jrg, Löhne→Lhne).
                'ä' | 'ö' | 'ü' | 'Ä' | 'Ö' | 'Ü' => {}
                // Accented latin: map to the base letter (Régis→Regis).
                'é' | 'è' | 'ê' | 'ë' => out.push('e'),
                'à' | 'â' | 'á' => out.push('a'),
                'î' | 'ï' | 'í' => out.push('i'),
                'ô' | 'ó' => out.push('o'),
                'û' | 'ú' => out.push('u'),
                'ç' => out.push('c'),
                'ñ' => out.push('n'),
                'ß' => out.push_str("ss"),
                _ => out.extend(c.to_lowercase()),
            }
        }
        out
    }

    pub(super) static GIVEN_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
        include_str!("../../resources/given_names.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(normalize)
            .filter(|n| !n.is_empty())
            .collect()
    });

    /// True if any sub-token of `run` is a known given name. Splits on
    /// whitespace AND intra-token separators (`-`, `/`, `.`, `,`) so
    /// compound/prefixed forms like `Hans-Rudolf` or `ESD-Roger` are matched
    /// part-by-part. Each part is normalized (umlaut-stripped + lowercased)
    /// before lookup.
    pub(super) fn run_has_given_name(run: &str) -> bool {
        run.split(|c: char| c.is_whitespace() || matches!(c, '-' | '/' | '.' | ',' | '_'))
            .any(|part| {
                let cleaned = part.trim_matches(|c: char| !c.is_alphabetic());
                !cleaned.is_empty() && GIVEN_NAMES.contains(&normalize(cleaned))
            })
    }
}

use scan_patterns::{
    RE_DIGITS, RE_GEA_DATE, RE_INITIAL_SURNAME, RE_NAME_RUN, RE_PATIENT, RE_PERSON_STAR,
    RE_QUARTER, RE_STREET, RE_SURNAME_INITIAL, RE_TITLE, RE_YEAR,
};

/// Month names (English + German, full + common abbreviations), longest-first.
const MONTH_NAMES: &[&str] = &[
    "September",
    "Februar",
    "Dezember",
    "November",
    "February",
    "December",
    "January",
    "October",
    "Januar",
    "Oktober",
    "August",
    "März",
    "Maerz",
    "April",
    "March",
    "Juni",
    "Juli",
    "June",
    "July",
    "Mai",
    "May",
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
];

impl PlaceholderGrammar {
    /// Fill a template to a concrete string. Structural placeholders
    /// (`{year}`, `{quarter}`, `{month}`, `{date}`, `{digits}`) are filled
    /// internally from `rng`; PII placeholders are delegated to `resolver`.
    /// Unknown `{…}` tokens are emitted verbatim.
    pub fn fill<R: rand::Rng>(
        template: &str,
        resolver: &mut dyn PlaceholderResolver,
        rng: &mut R,
    ) -> String {
        use rand::RngExt;
        if template.is_empty() {
            return String::new();
        }
        let mut out = String::with_capacity(template.len() + 16);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            rest = &rest[open..];
            let Some(close) = rest.find('}') else {
                // unbalanced — emit the remainder verbatim
                out.push_str(rest);
                return out;
            };
            let token = &rest[..=close];
            rest = &rest[close + 1..];
            if let Some(kind) = PiiPlaceholderKind::from_token(token) {
                let resolved = resolver.resolve(kind, rng);
                out.push_str(&resolved);
                continue;
            }
            match token {
                "{year}" => {
                    let y: u32 = rng.random_range(2018..=2024);
                    out.push_str(&y.to_string());
                }
                "{quarter}" => {
                    let q: u32 = rng.random_range(1..=4);
                    out.push('Q');
                    out.push_str(&q.to_string());
                }
                "{month}" => {
                    const MONTHS: &[&str] = &[
                        "January",
                        "February",
                        "March",
                        "April",
                        "May",
                        "June",
                        "July",
                        "August",
                        "September",
                        "October",
                        "November",
                        "December",
                    ];
                    out.push_str(MONTHS[rng.random_range(0..MONTHS.len())]);
                }
                "{date}" => {
                    let d: u32 = rng.random_range(1..=28);
                    let m: u32 = rng.random_range(1..=12);
                    let y: u32 = rng.random_range(2018..=2024);
                    // Use ISO-style yyyy-mm-dd so the output never matches
                    // RE_PATIENT's `G:\s*\d{2}\.\d{2}\.\d{2}` (dot-separated).
                    out.push_str(&format!("{y}-{m:02}-{d:02}"));
                }
                "{digits}" => {
                    let n = rng.random_range(4..=8);
                    for _ in 0..n {
                        out.push(char::from(b'0' + rng.random_range(0u8..10)));
                    }
                }
                _ => out.push_str(token), // unknown — verbatim
            }
        }
        out.push_str(rest);
        out
    }

    /// Scan a string for residual PII patterns. Returns one hit per pattern
    /// that matches; an empty result means the string is clean. Patterns that
    /// detect PII-bearing *shapes* — used as a hard gate at extraction and in
    /// CI. Templates whose PII spans are already placeholders (`{person}`,
    /// `{patient}`, …) do not match these patterns.
    pub fn residual_pii_scan(s: &str) -> Vec<PiiHit> {
        let mut hits = Vec::new();
        let checks: &[(&'static str, &Regex)] = &[
            ("patient_record", &RE_PATIENT),
            ("person_star", &RE_PERSON_STAR),
            ("title", &RE_TITLE),
            ("initial_surname", &RE_INITIAL_SURNAME),
            ("surname_initial", &RE_SURNAME_INITIAL),
        ];
        for (label, re) in checks {
            if let Some(m) = re.find(s) {
                hits.push(PiiHit {
                    pattern: label,
                    matched: m.as_str().to_string(),
                });
            }
        }
        // `Firstname Lastname` / `Lastname Firstname` — a capitalised run
        // anchored by a known given name. Catches the plain two-word names the
        // initial/title/patient regexes structurally miss (the SP6 leak class).
        for m in RE_NAME_RUN.find_iter(s) {
            if given_names::run_has_given_name(m.as_str()) {
                hits.push(PiiHit {
                    pattern: "given_name",
                    matched: m.as_str().to_string(),
                });
                break;
            }
        }
        hits
    }

    /// Phase A — automated structural placeholder-ization. Raw corpus string to
    /// a PII-safe template using only deterministic structural rules. Phase B
    /// (curated denylist for fuzzy proper nouns) is applied by the extractor.
    ///
    /// Rules, in order:
    /// 1. Patient `G:`-record: strip everything up to the first `G:dd.dd.dd`
    ///    marker (the name region, regardless of any `G` in the name), prepend
    ///    `*{patient} `, then replace each `G:/E:/A:dd.dd.dd` with the
    ///    letter-preserving form `G:{date}` / `E:{date}` / `A:{date}`.
    /// 2. `*Lastname,Firstname` star record: replace the matched span with
    ///    `*{person}`.
    /// 3. Street address: replace with `{street}`.
    /// 4. 4-digit years -> `{year}`; `Q1`-`Q4` -> `{quarter}`; month names ->
    ///    `{month}`; runs of >=4 digits -> `{digits}`.
    pub fn tokenize(s: &str) -> String {
        let t = s.trim();
        if t.is_empty() {
            return String::new();
        }

        // Rules 1 & 2 produce a *staged* prefix-handled string, then fall
        // through to the common tail (street + name-run + structural) so a
        // person name in the SUFFIX of a patient/star record is still caught
        // — they previously `return`ed early, leaking e.g. a trailing
        // `Firstname Lastname` after a `G:` patient marker.
        let staged: String = if let Some(m) = RE_PATIENT.find(t) {
            // Rule 1 — patient record. Slice off the name region up to the
            // date marker; date-stamp the G:/E:/A: triplets.
            let from_marker = &t[m.start()..];
            let dated = RE_GEA_DATE.replace_all(from_marker, "$1:{date}");
            format!("*{{patient}} {dated}").trim().to_string()
        } else if let Some(m) = RE_PERSON_STAR.find(t) {
            // Rule 2 — star person record. Replace the matched span with
            // *{person}, keep any suffix, drop the dangling firstname fragment.
            let mut out = String::with_capacity(t.len());
            out.push_str("*{person}");
            out.push_str(&t[m.end()..]);
            trim_leading_name_fragment(&out)
        } else {
            t.to_string()
        };

        // Rule 3 — street address.
        let staged = RE_STREET.replace_all(&staged, "{street}").into_owned();

        // Rule 3.5 — `Firstname Lastname` / `Lastname Firstname` person names.
        // A capitalised run anchored by a known given name collapses to
        // `{person}`. German capitalises all nouns, so we cannot tell a
        // surname from a description noun by case — redact the whole run
        // (safe over-redaction) rather than leak the adjacent surname.
        let staged = RE_NAME_RUN
            .replace_all(&staged, |caps: &regex::Captures| {
                let run = &caps[0];
                if given_names::run_has_given_name(run) {
                    "{person}".to_string()
                } else {
                    run.to_string()
                }
            })
            .into_owned();

        // Rule 4 — structural / temporal.
        let staged = RE_YEAR.replace_all(&staged, "{year}").into_owned();
        let staged = RE_QUARTER.replace_all(&staged, "{quarter}").into_owned();
        let staged = replace_months(&staged);
        RE_DIGITS.replace_all(&staged, "{digits}").into_owned()
    }
}

/// Replace month names with `{month}` at word boundaries (longest-first so
/// "September" wins over a hypothetical "Sep" prefix).
fn replace_months(s: &str) -> String {
    let mut result = s.to_string();
    for name in MONTH_NAMES {
        // Word-boundary replace, case-sensitive (month names are capitalised
        // in this corpus). Build a fresh string to avoid re-matching `{month}`.
        let mut out = String::with_capacity(result.len());
        let nlen = name.len();
        let mut i = 0;
        while i < result.len() {
            if result[i..].starts_with(name) {
                let prev_alpha = i > 0
                    && result[..i]
                        .chars()
                        .next_back()
                        .map(|c| c.is_alphabetic())
                        .unwrap_or(false);
                let next_alpha = result[i + nlen..]
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic())
                    .unwrap_or(false);
                if !prev_alpha && !next_alpha {
                    out.push_str("{month}");
                    i += nlen;
                    continue;
                }
            }
            // push one char
            let ch_len = result[i..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            out.push_str(&result[i..i + ch_len]);
            i += ch_len;
        }
        result = out;
    }
    result
}

/// Drop a leading lowercase/name-char fragment immediately after `*{person}`.
/// Handles `RE_PERSON_STAR` matching only `*Lastname,F` and leaving `irstname`.
/// Consumes ONE contiguous alphabetic run (the firstname-completion) plus AT
/// MOST one trailing separator (space or comma) — does NOT consume further
/// alphabetic words which would eat description content. When non-empty
/// content remains, reinsert a single space separator for readability.
fn trim_leading_name_fragment(s: &str) -> String {
    const PREFIX: &str = "*{person}";
    if let Some(rest) = s.strip_prefix(PREFIX) {
        let mut end = 0usize;
        // Consume one contiguous alphabetic run (the firstname-completion).
        for (i, c) in rest.char_indices() {
            if c.is_alphabetic() {
                end = i + c.len_utf8();
            } else {
                break;
            }
        }
        // Consume up to one trailing comma or space — but NOT further alphabetics.
        if let Some(c) = rest[end..].chars().next() {
            if c == ',' || c == ' ' {
                end += c.len_utf8();
            }
        }
        let trimmed = &rest[end..];
        if trimmed.is_empty() {
            PREFIX.to_string()
        } else {
            format!("{PREFIX} {trimmed}")
        }
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn residual_scan_flags_patient_record() {
        let hits = PlaceholderGrammar::residual_pii_scan("*Gambon,Laurin G:01.02.03 E:04.05.06");
        assert!(
            hits.iter().any(|h| h.pattern == "patient_record"),
            "expected patient_record hit, got {hits:?}"
        );
    }

    #[test]
    fn residual_scan_flags_person_shapes() {
        // star record
        assert!(PlaceholderGrammar::residual_pii_scan("*Mueller,Hans")
            .iter()
            .any(|h| h.pattern == "person_star"));
        // initial + surname
        assert!(PlaceholderGrammar::residual_pii_scan("Forschung U. Frey")
            .iter()
            .any(|h| h.pattern == "initial_surname"));
        // title
        assert!(
            PlaceholderGrammar::residual_pii_scan("Kontokorrent Prof. Dr. M. Buess")
                .iter()
                .any(|h| h.pattern == "title")
        );
        // surname + initial (e.g. "Mueller H." in a description)
        assert!(
            PlaceholderGrammar::residual_pii_scan("Konsultation Mueller H.")
                .iter()
                .any(|h| h.pattern == "surname_initial")
        );
    }

    #[test]
    fn residual_scan_passes_clean_templates() {
        for clean in [
            "Rechnung {company}",
            "Mieten {month}.{year}",
            "ARIBA_ASN",
            "Darlehen {person}",
            "*{patient} G:{date} E:{date} A:{date}",
            "Umbuchung Anlage",
        ] {
            assert!(
                PlaceholderGrammar::residual_pii_scan(clean).is_empty(),
                "false positive on clean template: {clean:?}"
            );
        }
    }

    #[test]
    fn residual_scan_excludes_legal_entity_suffixes() {
        // Legal-entity abbreviations like B.V. / S.A. / S.r.l. have a
        // capital letter immediately following the period. The
        // `surname_initial` regex must NOT match these — they're
        // corporate-entity markers, not person initials. (Pre-T16 corpus
        // scan found ~90% of raw `surname_initial` hits were exactly
        // this shape, e.g. `Acme S.A.`, `Globex Europe B.V.` — only the
        // shape matters for this test, names are fictional.)
        for legal in [
            "Acme Europe B.V.",
            "Globex Suisse S.A.",
            "Initech S.A. Lugano",
            "Switzerland S.A.",
        ] {
            assert!(
                PlaceholderGrammar::residual_pii_scan(legal)
                    .iter()
                    .all(|h| h.pattern != "surname_initial"),
                "must not flag legal-entity suffix in: {legal:?}"
            );
        }
        // But a legitimate surname+initial at end-of-string still matches.
        assert!(
            PlaceholderGrammar::residual_pii_scan("Patient consult Mueller H.")
                .iter()
                .any(|h| h.pattern == "surname_initial"),
            "legitimate surname-initial at end-of-string must still match"
        );
    }

    /// SP6 leak class: plain `Firstname Lastname` / `Lastname Firstname` —
    /// the form the initial/title/patient regexes structurally miss and that
    /// shipped 285 real names in the first bundle cut. Fictional surnames
    /// (Mustermann/Beispiel) + real generic given names exercise the shape.
    #[test]
    fn residual_scan_flags_given_name_runs() {
        for s in [
            "Beratung Marc Mustermann",        // desc + given + surname
            "Erbschaft Anna Beispiel",         // desc + given + surname
            "Mustermann Thomas Guthaben",      // surname-first + given
            "Florian Beispiel, Verzugszinsen", // given + surname, comma
        ] {
            let hits = PlaceholderGrammar::residual_pii_scan(s);
            assert!(
                hits.iter().any(|h| h.pattern == "given_name"),
                "must flag given-name run in: {s:?} (got {hits:?})"
            );
        }
    }

    #[test]
    fn tokenize_collapses_person_name_runs() {
        // German capitalises all nouns, so a contiguous capitalised run that
        // contains a given name collapses whole — we cannot tell the leading
        // description noun ("Beratung") from the surname by case, so we
        // over-redact rather than risk leaking the surname.
        assert_eq!(
            PlaceholderGrammar::tokenize("Beratung Marc Mustermann"),
            "{person}"
        );
        // A lowercase word / punctuation terminates the run, preserving the
        // surrounding description.
        assert_eq!(
            PlaceholderGrammar::tokenize("Florian Beispiel, Verzugszinsen"),
            "{person}, Verzugszinsen"
        );
        assert_eq!(
            PlaceholderGrammar::tokenize("Kurt Beispiel/Miete Lager"),
            "{person}/Miete Lager"
        );
        // No name leaks through after tokenization.
        for s in ["Beratung Marc Mustermann", "Mustermann Thomas Guthaben"] {
            assert!(
                PlaceholderGrammar::residual_pii_scan(&PlaceholderGrammar::tokenize(s)).is_empty(),
                "tokenized form of {s:?} must be PII-clean"
            );
        }
    }

    /// Regression: compound/prefixed given-name tokens and umlaut-stripped
    /// corpus forms must still be recognized. The corpus drops umlauts
    /// (`Jürg`→`Jrg`) and joins tokens with `-`/`/` (`Hans-Rudolf`,
    /// `ESD-Roger`), which defeated a plain whitespace+lowercase lookup and
    /// leaked names through the first regen passes.
    #[test]
    fn tokenize_handles_compound_and_umlaut_stripped_names() {
        // Hyphenated compound given name (Hans + Rudolf both known).
        assert_eq!(
            PlaceholderGrammar::tokenize("Hans-Rudolf Beispiel"),
            "{person}"
        );
        // Prefix-joined given name (ESD-Roger → Roger is known).
        assert_eq!(
            PlaceholderGrammar::tokenize("ESD-Roger Mustermann"),
            "{person}"
        );
        // Umlaut-stripped corpus form: gazetteer has `Jürg`; corpus stores `Jrg`.
        assert_eq!(PlaceholderGrammar::tokenize("Jrg Mustermann"), "{person}");
        for s in [
            "Hans-Rudolf Beispiel",
            "ESD-Roger Mustermann",
            "Jrg Mustermann",
        ] {
            assert!(
                PlaceholderGrammar::residual_pii_scan(&PlaceholderGrammar::tokenize(s)).is_empty(),
                "compound/umlaut name leaked: {s:?}"
            );
        }
    }

    /// Regression: a person name in the SUFFIX of a patient/star record must
    /// also be collapsed. The patient/star rules used to `return` early,
    /// leaking a trailing `Firstname Lastname` (the JE_44 `Robert Hoe` regen
    /// failure). Now they fall through to the name-run + structural tail.
    #[test]
    fn tokenize_name_in_patient_or_star_suffix_is_clean() {
        for s in [
            "*Muster,A G:01.02.03 E:04.05.06 Thomas Beispiel",
            "*Muster,Anna Beratung Marc Mustermann",
        ] {
            let tok = PlaceholderGrammar::tokenize(s);
            assert!(
                PlaceholderGrammar::residual_pii_scan(&tok).is_empty(),
                "suffix name leaked: {s:?} -> {tok:?}"
            );
        }
    }

    #[test]
    fn name_detection_no_false_positives() {
        // Real bank names (kept by design), generic accounting terms, and
        // already-placeholdered text must NOT be flagged or rewritten.
        for clean in [
            "Deutsche Bank",
            "Kontokorrent {company} AG",
            "Material Werkzeuge Werkstoffe",
            "Goldman Sachs",
            "Standard Chartered",
        ] {
            assert!(
                PlaceholderGrammar::residual_pii_scan(clean)
                    .iter()
                    .all(|h| h.pattern != "given_name"),
                "false-positive given_name on: {clean:?}"
            );
            assert_eq!(
                PlaceholderGrammar::tokenize(clean),
                clean,
                "tokenize must not rewrite clean text: {clean:?}"
            );
        }
    }

    #[test]
    fn pii_placeholder_kind_token_roundtrip() {
        for kind in [
            PiiPlaceholderKind::Patient,
            PiiPlaceholderKind::Person,
            PiiPlaceholderKind::Company,
            PiiPlaceholderKind::Street,
        ] {
            assert_eq!(PiiPlaceholderKind::from_token(kind.token()), Some(kind));
        }
        assert_eq!(PiiPlaceholderKind::from_token("{year}"), None);
        assert_eq!(PiiPlaceholderKind::from_token("{unknown}"), None);
    }

    #[test]
    fn line_key_format() {
        assert_eq!(TextTaxonomyPrior::line_key("KR", "A.B"), "KR|A.B");
        assert_eq!(
            TextTaxonomyPrior::line_key("RE", TextTaxonomyPrior::UNKNOWN_CLASS),
            "RE|_unknown_"
        );
    }

    #[test]
    fn synthetic_example_resolver_emits_obvious_fakes() {
        let mut r = SyntheticExampleResolver;
        let mut rng = rand::rng();
        for kind in [
            PiiPlaceholderKind::Patient,
            PiiPlaceholderKind::Person,
            PiiPlaceholderKind::Company,
            PiiPlaceholderKind::Street,
        ] {
            let v = r.resolve(kind, &mut rng);
            assert!(v.starts_with("Example"), "expected obvious fake, got {v}");
        }
    }

    #[test]
    fn tokenize_patient_record_strips_name_even_with_g_in_it() {
        // The name "Gambon" contains a G — the strip must consume it. A naive
        // [^G]*? class cannot, and would leak the name. This is the bug the
        // first-pass cleaning sweep found.
        assert_eq!(
            PlaceholderGrammar::tokenize("*Gambon,Laurin G:01.02.03 E:04.05.06 A:07.08.09"),
            "*{patient} G:{date} E:{date} A:{date}"
        );
        assert_eq!(
            PlaceholderGrammar::tokenize("*Rykart,Frank G G:11.12.13"),
            "*{patient} G:{date}"
        );
    }

    #[test]
    fn tokenize_person_star_record() {
        assert_eq!(PlaceholderGrammar::tokenize("*Mueller,Hans"), "*{person}");
        // Pin behavior on `*Lastname,Firstname<rest>` shapes: after the
        // firstname fragment is trimmed, a single space separates {person}
        // from the remainder. Short digit runs (< 4) stay verbatim.
        assert_eq!(
            PlaceholderGrammar::tokenize("*Mueller,Hans Ref-123"),
            "*{person} Ref-123"
        );
    }

    #[test]
    fn tokenize_street_address() {
        assert_eq!(
            PlaceholderGrammar::tokenize("LUKB Mietzinskaution Roentgenpraxis, Spitalstrasse 5"),
            "LUKB Mietzinskaution Roentgenpraxis, {street}"
        );
    }

    #[test]
    fn tokenize_structural_temporal() {
        assert_eq!(
            PlaceholderGrammar::tokenize("Mieten 04.2021"),
            "Mieten 04.{year}"
        );
        assert_eq!(
            PlaceholderGrammar::tokenize("Sales Accrual Q1"),
            "Sales Accrual {quarter}"
        );
        assert_eq!(
            PlaceholderGrammar::tokenize("January accrual"),
            "{month} accrual"
        );
        assert_eq!(PlaceholderGrammar::tokenize("INV 1234567"), "INV {digits}");
        assert_eq!(PlaceholderGrammar::tokenize("GL 470"), "GL 470"); // short run kept
    }

    #[test]
    fn tokenize_fixed_vocab_unchanged() {
        assert_eq!(PlaceholderGrammar::tokenize("ARIBA_ASN"), "ARIBA_ASN");
        assert_eq!(
            PlaceholderGrammar::tokenize("CH Post: KUREPO Intercomp"),
            "CH Post: KUREPO Intercomp"
        );
    }

    #[test]
    fn tokenize_then_scan_is_clean() {
        // Every Phase-A-tokenized string with structural PII must scan clean.
        for raw in [
            "*Gambon,Laurin G:01.02.03 E:04.05.06 A:07.08.09",
            "*Mueller,Hans",
            "LUKB Spitalstrasse 5",
        ] {
            let tok = PlaceholderGrammar::tokenize(raw);
            assert!(
                PlaceholderGrammar::residual_pii_scan(&tok).is_empty(),
                "tokenize left residual PII: {raw:?} -> {tok:?}"
            );
        }
    }

    #[test]
    fn fill_structural_placeholders() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut resolver = SyntheticExampleResolver;
        let out = PlaceholderGrammar::fill(
            "Mieten {month}.{year} ref {digits} {quarter}",
            &mut resolver,
            &mut rng,
        );
        assert!(
            !out.contains('{'),
            "structural placeholders left unfilled: {out}"
        );
        assert!(out.starts_with("Mieten "));
    }

    #[test]
    fn fill_pii_placeholders_via_resolver() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut resolver = SyntheticExampleResolver;
        let out =
            PlaceholderGrammar::fill("Rechnung {company} / {person}", &mut resolver, &mut rng);
        assert_eq!(out, "Rechnung Example GmbH / Example Person");
    }

    #[test]
    fn fill_unknown_placeholder_kept_literal() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut resolver = SyntheticExampleResolver;
        let out = PlaceholderGrammar::fill("foo {bogus} bar", &mut resolver, &mut rng);
        assert_eq!(out, "foo {bogus} bar");
    }

    #[test]
    fn fill_then_scan_clean() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut resolver = SyntheticExampleResolver;
        for tmpl in ["Darlehen {person}", "*{patient} G:{date}", "{company} AG"] {
            let out = PlaceholderGrammar::fill(tmpl, &mut resolver, &mut rng);
            assert!(
                PlaceholderGrammar::residual_pii_scan(&out).is_empty(),
                "fill produced residual-PII shape: {tmpl:?} -> {out:?}"
            );
        }
    }

    #[test]
    fn prior_serde_roundtrip() {
        let mut prior = TextTaxonomyPrior::default();
        prior.line_pools.insert(
            TextTaxonomyPrior::line_key("KR", "A.B"),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Rechnung {company}".to_string(),
                    probability: 1.0,
                    synthetic_example: "Rechnung Example GmbH".to_string(),
                }],
                n: 42,
            },
        );
        prior.meta.class_tier = "iso21378_l2".to_string();
        let yaml = serde_yaml::to_string(&prior).expect("serialize");
        let back: TextTaxonomyPrior = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(prior, back);
    }
}
