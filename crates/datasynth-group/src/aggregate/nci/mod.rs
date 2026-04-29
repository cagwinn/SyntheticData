//! Non-controlling interest (NCI) consolidation — Chunk 7.
//!
//! After the post-elimination consolidated trial balance is assembled
//! (Chunk 5) and per-entity translation has bridged each subsidiary
//! into the group presentation currency (Chunk 6), the aggregate phase
//! must measure and roll forward the share of equity attributable to
//! non-controlling shareholders for every fully-consolidated subsidiary
//! that is not wholly owned by the parent.
//!
//! # Standards reference
//!
//! - **IFRS 10** *Consolidated Financial Statements* §§ B94–B96 and § 22:
//!   the parent must allocate profit or loss and each component of
//!   other comprehensive income (OCI) between the controlling and
//!   non-controlling interests in proportion to their ownership
//!   interest.
//! - **IFRS 12** *Disclosure of Interests in Other Entities* §§ 10–17:
//!   the rollforward of NCI for each subsidiary that has material NCI
//!   must be disclosed in the consolidated notes.
//! - **US GAAP — ASC 810** *Consolidation*: the equivalent NCI
//!   measurement framework.  Both frameworks require the same opening
//!   + share-of-profit + share-of-OCI − dividends rollforward identity.
//!
//! # v5.0 scope
//!
//! - **Full consolidation only.**  NCI is measured only for entities
//!   with [`crate::config::ConsolidationMethod::Full`] and an
//!   `ownership_percent < 1.0`.  `Parent` entities are wholly owned and
//!   produce no NCI; `EquityMethod` / `Proportional` / `FairValue`
//!   entities are *not* fully consolidated and instead route through
//!   [`super::equity_method`] (Task 7.3) or stay deferred for later
//!   chunks.
//! - **Single-period rollforward.**  The opening NCI is supplied by the
//!   caller (Task 7.2 layers in the prior-period auto-load).
//! - **No goodwill / fair-value step-up at acquisition.**  The
//!   acquisition-date NCI measurement (full-goodwill vs partial-
//!   goodwill, fair-value vs proportionate-share) is deferred to a
//!   later chunk; v5.0 just rolls forward an already-computed opening
//!   balance through the period's profit, OCI, and dividends.
//!
//! # Module layout
//!
//! - [`rollforward`] (Task 7.1): the core
//!   `compute_nci_rollforward(NciInputs) -> NciRollforward` function
//!   that derives the closing NCI from the IFRS 10 / ASC 810
//!   identity:
//!
//!   ```text
//!   closing_nci = opening_nci
//!               + (1 - ownership_percent) * net_income
//!               + (1 - ownership_percent) * oci
//!               - (1 - ownership_percent) * dividends_paid
//!   ```
//!
//! - [`opening`] (Task 7.2): prior-period ingestion and on-disk writer.
//!
//! See `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`
//! §"Aggregate phase — Chunk 7" for the full Chunk-7 module layout.

pub mod opening;
pub mod rollforward;

pub use opening::{ingest_opening_nci_balances, write_nci_rollforward, NCI_ROLLFORWARD_FILENAME};
pub use rollforward::{compute_nci_rollforward, NciInputs, NciRollforward};
