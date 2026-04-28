//! Library surface for `datasynth-cli`.
//!
//! The crate is primarily the `datasynth-data` binary (see `main.rs`), but a
//! few modules are exposed via a thin `[lib]` target so integration tests
//! can exercise internal routing helpers without spinning up a full
//! `EnhancedOrchestrator` end-to-end.
//!
//! Currently exposed:
//! - [`output_writer`]: comprehensive output writer for all generated data.
//!   Used by the group-audit shard runner (v5.0+) via
//!   [`output_writer::write_all_output_with_root`] to route each entity's
//!   archive under `{root_dir}/entities/{code}/`.

pub mod output_writer;
