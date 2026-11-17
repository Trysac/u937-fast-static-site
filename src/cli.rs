use std::io::Write;
use std::path::Path;

use crate::fs::DiskFs;
use crate::site::{Config, Site};

/// Runs the `fast-static-site` command-line interface.
///
/// Supported invocation:
/// `build <dir> [--live-reload]`
///
/// Exit codes:
/// - `0`: build succeeded
/// - `1`: domain validation failure (front matter, template error, input too large)
/// - `2`: CLI usage error or missing/unreadable site directory / I/O error
pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    if args.is_empty() {
        let _ = writeln!(
            stderr,
            "usage: fast-static-site build <dir> [--live-reload]"
        );
        return 2;
    }

    let cmd = args[0].as_str();
    if cmd != "build" {
        let _ = writeln!(
            stderr,
            "unknown command '{cmd}'; expected: build <dir> [--live-reload]"
        );
        return 2;
    }

    let mut dir: Option<&str> = None;
    let mut live_reload = false;

    for arg in &args[1..] {
        if arg == "--live-reload" {
            live_reload = true;
        } else if arg.starts_with('-') {
            let _ = writeln!(stderr, "unknown flag '{arg}'");
            return 2;
        } else if dir.is_none() {
            dir = Some(arg.as_str());
        } else {
            let _ = writeln!(stderr, "unexpected argument '{arg}'");
            return 2;
        }
    }

    let Some(dir_str) = dir else {
        let _ = writeln!(stderr, "missing <dir> argument for build");
        return 2;
    };

    let root = Path::new(dir_str);
    if !root.is_dir() {
        let _ = writeln!(stderr, "site directory does not exist: {dir_str}");
        return 2;
    }

    let config = Config {
        live_reload,
        ..Config::default()
    };
    let fs = Box::new(DiskFs::new(root));
    let mut site = Site::new(fs, config);

    match site.build() {
        Ok(report) => {
            let _ = writeln!(
                stdout,
                "rendered={} written={} copied={} deleted={}",
                report.rendered,
                report.written,
                report.copied,
                report.deleted.len()
            );
            0
        }
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            if err.code == "io" { 2 } else { 1 }
        }
    }
}
