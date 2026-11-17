#![cfg(feature = "conformance")]
//! CI-only conformance timing test for a 5,000-page incremental rebuild.

use std::path::{Path, PathBuf};
use std::time::Instant;

use fast_static_site::{Config, MemFs, Site};

#[test]
fn incremental_rebuild_5000_pages_under_1000ms() {
    let mut mem = MemFs::new();
    mem.insert(
        "templates/page.html",
        "<!doctype html><html><body><h1>{{ page.title }}</h1>{{ page.content | safe }}</body></html>",
    );

    for sec in 0..10 {
        for idx in 0..500 {
            let path = format!("content/sec{sec}/page_{idx:03}.md");
            let src = format!(
                "---\ntitle: Sec {sec} Page {idx:03}\ndate: 2026-01-01\nlayout: page\n---\n# Heading {idx}\n\nParagraph text for page {idx}.\n"
            );
            mem.insert(path, src);
        }
    }

    let mut site = Site::new(Box::new(mem), Config::default());
    let full_report = site.build().unwrap();
    assert_eq!(full_report.rendered, 5_000);

    site.fs_mut()
        .write(
            Path::new("content/sec3/page_042.md"),
            b"---\ntitle: Sec 3 Page 042\ndate: 2026-01-01\nlayout: page\n---\n# Updated\n\nEdited body.\n",
        )
        .unwrap();

    let start = Instant::now();
    let rebuild_report = site
        .rebuild(&[PathBuf::from("content/sec3/page_042.md")])
        .unwrap();
    let elapsed = start.elapsed();

    println!(
        "5000-page single-page incremental rebuild took {:?} ({} ms)",
        elapsed,
        elapsed.as_millis()
    );
    assert_eq!(rebuild_report.rendered, 1);
    assert_eq!(rebuild_report.written, 1);
    assert!(
        elapsed.as_millis() < 1_000,
        "expected rebuild under 1000 ms in debug profile, took {elapsed:?}"
    );
}
