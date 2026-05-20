//! SP6 — Curated PII denylist (Phase B of tokenization).
//!
//! The denylist is PII-derived (its left-hand side IS real proper nouns) and
//! NEVER enters the public repo. It lives at a private path alongside the real
//! corpus; the extractor takes `--pii-denylist <path>`. When absent, only
//! Phase A (automated structural) tokenization runs.
//!
//! File format — one rule per line, tab-separated:
//!   <literal-or-/regex/>\t<patient|person|company|street>
//! Lines that are blank or start with `#` are ignored.
//!
//! **Regex engine:** uses `regex_lite` (the same engine the rest of this crate
//! uses for performance / build-size reasons). `regex_lite` does NOT support
//! Unicode-aware `\b`/`\B` boundaries, `\p{…}` Unicode classes, or lookaround.
//! Denylist regex authors should use ASCII patterns or literal proper-noun
//! matches — for non-ASCII names (e.g. names with `ß`, umlauts), prefer the
//! `<literal>` form over a `/regex/` with `\b`.

use std::collections::BTreeMap;
use std::path::Path;

use aho_corasick::{AhoCorasick, MatchKind};
use datasynth_core::distributions::text_taxonomy::PiiPlaceholderKind;
use regex_lite::Regex;

use crate::FingerprintError;

/// Curated mapping from real proper-noun spans to placeholder kinds.
///
/// **SP6.1 performance note.** Literal exact-match rules are compiled into
/// an Aho–Corasick automaton with `MatchKind::LeftmostLongest` semantics so
/// `apply` runs in a single linear pass over the input regardless of how
/// many literal rules the denylist carries. T16 surfaced the prior
/// sequential `.replace()` loop as O(N×|text|) — a 27k-entry denylist took
/// ~9 hours to process one 3M-row JE file. The AC pass collapses that to a
/// single linear scan + per-match replacement.
#[derive(Default)]
pub struct PiiDenylist {
    /// Aho–Corasick automaton built from all literal-form entries. `None`
    /// when the denylist carries zero literal rules (regex-only denylists
    /// are still supported).
    exact_ac: Option<AhoCorasick>,
    /// Parallel to the AC pattern index: `replacement[i]` is the
    /// `PiiPlaceholderKind::token()` for pattern `i`. `&'static str` because
    /// `kind.token()` always returns a static placeholder literal.
    exact_replacements: Vec<&'static str>,
    /// Regex family rules — applied AFTER the AC pass, in file order.
    patterns: Vec<(Regex, PiiPlaceholderKind)>,
    /// Count of literal entries loaded (audit / debug only).
    n_literals: usize,
}

impl std::fmt::Debug for PiiDenylist {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiiDenylist")
            .field("n_literals", &self.n_literals)
            .field("n_patterns", &self.patterns.len())
            .finish_non_exhaustive()
    }
}

fn parse_kind(s: &str) -> Option<PiiPlaceholderKind> {
    match s.trim() {
        "patient" => Some(PiiPlaceholderKind::Patient),
        "person" => Some(PiiPlaceholderKind::Person),
        "company" => Some(PiiPlaceholderKind::Company),
        "street" => Some(PiiPlaceholderKind::Street),
        _ => None,
    }
}

impl PiiDenylist {
    /// Load from a tab-separated file at a private path.
    pub fn load(path: &Path) -> Result<Self, FingerprintError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| FingerprintError::PiiDenylist(format!("read {}: {e}", path.display())))?;
        // De-duplicate literal entries (a literal repeated across per-industry
        // slices is common; the AC accepts duplicates but the parallel
        // replacement vector would carry redundant entries).
        let mut exact_map: BTreeMap<String, PiiPlaceholderKind> = BTreeMap::new();
        let mut patterns = Vec::new();
        for (lineno, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(2, '\t');
            let lhs = parts.next().unwrap_or("").trim();
            let rhs = parts.next().unwrap_or("").trim();
            let kind = parse_kind(rhs).ok_or_else(|| {
                FingerprintError::PiiDenylist(format!(
                    "{}:{}: unknown placeholder kind {rhs:?}",
                    path.display(),
                    lineno + 1
                ))
            })?;
            if let Some(inner) = lhs.strip_prefix('/').and_then(|s| s.strip_suffix('/')) {
                let re = Regex::new(inner).map_err(|e| {
                    FingerprintError::PiiDenylist(format!(
                        "{}:{}: bad regex {inner:?}: {e}",
                        path.display(),
                        lineno + 1
                    ))
                })?;
                patterns.push((re, kind));
            } else if !lhs.is_empty() {
                exact_map.insert(lhs.to_string(), kind);
            }
        }
        let n_literals = exact_map.len();
        let (exact_ac, exact_replacements) = if exact_map.is_empty() {
            (None, Vec::new())
        } else {
            // Build parallel vectors. `MatchKind::LeftmostLongest` means
            // overlapping prefixes resolve to the longest match (so
            // "Berner Kantonalbank" wins over "Berner" when both exist).
            let (literals, kinds): (Vec<String>, Vec<PiiPlaceholderKind>) =
                exact_map.into_iter().unzip();
            let ac = AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .build(&literals)
                .map_err(|e| FingerprintError::PiiDenylist(format!("aho-corasick build: {e}")))?;
            let replacements: Vec<&'static str> = kinds.iter().map(|k| k.token()).collect();
            (Some(ac), replacements)
        };
        Ok(Self {
            exact_ac,
            exact_replacements,
            patterns,
            n_literals,
        })
    }

    /// Apply Phase B: replace every denylisted span with its placeholder
    /// token.
    ///
    /// Single linear pass for the literal rules (Aho–Corasick), then one
    /// regex sweep per regex rule. Throughput is independent of literal
    /// count after AC construction.
    pub fn apply(&self, s: &str) -> String {
        let mut out: String = match &self.exact_ac {
            Some(ac) => ac.replace_all(s, &self.exact_replacements),
            None => s.to_string(),
        };
        for (re, kind) in &self.patterns {
            out = re.replace_all(&out, kind.token()).into_owned();
        }
        out
    }

    /// `true` when the denylist carries no rules (e.g. an empty file).
    pub fn is_empty(&self) -> bool {
        self.n_literals == 0 && self.patterns.is_empty()
    }

    /// Count of literal exact-match rules loaded (audit / debug only).
    pub fn literal_count(&self) -> usize {
        self.n_literals
    }

    /// Count of regex rules loaded (audit / debug only).
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Write `contents` to a fresh per-test tempfile and return the handle.
    /// The caller MUST keep the handle alive until they are done with the
    /// path — dropping it deletes the file. Using `NamedTempFile` (not a
    /// hand-rolled `process::id()`-keyed path) is what keeps parallel-run
    /// tests from racing on the same path; the pre-CI version of these
    /// tests collided in `cargo test` default parallelism and surfaced as
    /// `literal_count == 0` on the larger fixture.
    fn write_tmp(contents: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(contents.as_bytes()).expect("write tempfile");
        f.flush().expect("flush tempfile");
        f
    }

    #[test]
    fn load_and_apply_exact_and_regex() {
        let tf = write_tmp(
            "# comment\n\
             Clarunis\tcompany\n\
             Inselspital\tcompany\n\
             /\\bKantonalbank\\b/\tcompany\n",
        );
        let dl = PiiDenylist::load(tf.path()).expect("load");

        assert_eq!(dl.apply("Kontokorrent Clarunis"), "Kontokorrent {company}");
        assert_eq!(
            dl.apply("Darlehen Inselspital Bern"),
            "Darlehen {company} Bern"
        );
        assert_eq!(dl.apply("Basler Kantonalbank EUR"), "Basler {company} EUR");
        assert_eq!(dl.apply("nothing to do here"), "nothing to do here");
    }

    #[test]
    fn malformed_line_is_an_error() {
        let tf = write_tmp("Clarunis\tnot_a_kind\n");
        let res = PiiDenylist::load(tf.path());
        assert!(res.is_err());
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(PiiDenylist::load(Path::new("/nonexistent/denylist.tsv")).is_err());
    }

    #[test]
    fn longest_match_wins_when_prefixes_overlap() {
        // "Berner Kantonalbank" must beat "Berner" alone. MatchKind::LeftmostLongest
        // is the property — pin it here so a future engine swap can't regress it.
        let tf = write_tmp(
            "Berner\tcompany\n\
             Berner Kantonalbank\tcompany\n",
        );
        let dl = PiiDenylist::load(tf.path()).expect("load");
        // The full phrase replaces with a single {company}; "Berner" alone is not
        // applied to the same span (no double-substitution).
        assert_eq!(
            dl.apply("Konto Berner Kantonalbank Zürich"),
            "Konto {company} Zürich"
        );
    }

    #[test]
    fn pure_regex_denylist_works_without_literals() {
        let tf = write_tmp("/\\bKantonalbank\\b/\tcompany\n");
        let dl = PiiDenylist::load(tf.path()).expect("load");
        assert_eq!(dl.literal_count(), 0);
        assert_eq!(dl.pattern_count(), 1);
        assert_eq!(dl.apply("Basler Kantonalbank EUR"), "Basler {company} EUR");
    }

    /// SP6.1 perf invariant. The prior sequential `.replace()` loop took
    /// minutes on a 10k-entry denylist with a 100 KB input. With the AC
    /// rewrite, the same workload runs in well under one second. The
    /// 5-second budget is generous enough to avoid CI flake while still
    /// catching a catastrophic regression (e.g. an accidental return to
    /// the O(N×|text|) loop).
    #[test]
    fn apply_scales_with_aho_corasick_not_with_denylist_size() {
        use std::time::Instant;

        // Build a 10k-entry literal denylist.
        let mut tsv = String::new();
        for i in 0..10_000 {
            tsv.push_str(&format!("LiteralEntry{i}\tcompany\n"));
        }
        let tf = write_tmp(&tsv);
        let dl = PiiDenylist::load(tf.path()).expect("load 10k-entry denylist");
        assert_eq!(dl.literal_count(), 10_000);

        // 100 KB synthetic input: alternates filler text + a denylist hit
        // (so the AC actually does replacements, not just scans).
        let mut input = String::new();
        while input.len() < 100_000 {
            input.push_str("Some neutral filler text without any pii in it. ");
            input.push_str("Match LiteralEntry42 here. ");
        }

        let t0 = Instant::now();
        let out = dl.apply(&input);
        let elapsed = t0.elapsed();
        assert!(
            out.contains("{company}"),
            "expected {{company}} substitutions"
        );
        assert!(
            elapsed.as_secs() < 5,
            "PiiDenylist::apply took {elapsed:?} on 10k literals × 100KB — likely fell back to O(N) loop"
        );
    }
}
