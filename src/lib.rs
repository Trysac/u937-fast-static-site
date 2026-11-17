// The README is the crate documentation, so its usage example runs as a doc test.
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

/// Command-line interface runner for `fast-static-site`.
pub mod cli;
/// Error type and machine-readable codes.
pub mod error;
/// Virtual and disk file system abstractions.
pub mod fs;
/// Hand-written CommonMark 0.31.2 subset parser and HTML renderer.
pub mod markdown;
/// Incremental static site builder, dependency graph, and live-reload event formatter.
pub mod site;
/// Strict HTML template parser and evaluator.
pub mod template;

pub use error::{ErrorCode, SiteError};
pub use fs::{DiskFs, MemFs, SiteFs};
pub use markdown::{escape_html, markdown_to_html};
pub use site::{
    BuildReport, Config, DEFAULT_MAX_SOURCE_BYTES, LIVE_RELOAD_SNIPPET, Site, fnv1a64, reload_event,
};
pub use template::{DEFAULT_MAX_INCLUDE_DEPTH, Template, Value};
