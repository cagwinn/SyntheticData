//! IAS 21 — *The Effects of Changes in Foreign Exchange Rates*.
//!
//! After the post-elimination consolidated trial balance is assembled
//! (Chunk 5), the aggregate phase must translate every component
//! entity's TB from its functional currency to the group presentation
//! currency. This module hosts the per-entity translation pipeline:
//!
//! - [`classify`] — IAS 21 monetary / non-monetary / equity / P&L
//!   classification of GL accounts (drives the rate-basis decision).
//!
//! Subsequent tasks in Chunk 6 add per-entity translation
//! (Task 6.2), CTA computation and rollforward (Task 6.3), the
//! translation worksheet emission (Task 6.4), and an end-to-end
//! Mini-Nestlé round-trip (Task 6.5).
//!
//! # IAS 21 contract recap
//!
//! - Monetary balance-sheet items (cash, AR, AP, debt) translate at
//!   the **closing rate** (IAS 21.23(a)).
//! - Non-monetary balance-sheet items measured at historical cost
//!   (inventory, fixed assets, intangibles, goodwill, prepaid
//!   expenses) translate at the **historical rate** (IAS 21.23(b)).
//! - P&L items translate at the rate at the date of the transaction —
//!   the **average rate** is permitted as a practical approximation
//!   when rates are stable (IAS 21.40).
//! - Equity (common stock, retained earnings, reserves) translates at
//!   the **historical rate** at the date of acquisition or initial
//!   recognition.
//! - The residual translation difference (CTA) accumulates in OCI per
//!   IAS 21.39 and is recycled to P&L on disposal.
//!
//! See `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`
//! §"Aggregate phase — Chunk 6" for the full module layout.

pub mod classify;

pub use classify::{classify_account, TranslationAccountType};
