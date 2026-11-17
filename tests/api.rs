//! Integration and property tests for `fast_static_site`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use proptest::prelude::*;

use fast_static_site::{
    BuildReport, Config, DiskFs, MemFs, Site, SiteFs, Template, Value, cli, markdown_to_html,
    reload_event,
};

#[test]
fn acceptance_markdown_headings_inline_code_and_links() {
    let out = markdown_to_html("# Hi *there*\n\nA `x<y` [link](https://example.test \"T\")");
    assert_eq!(
        out,
        "<h1>Hi <em>there</em></h1>\n<p>A <code>x&lt;y</code> <a href=\"https://example.test\" title=\"T\">link</a></p>\n"
    );
}

#[test]
fn acceptance_markdown_fenced_lists_and_raw_html_escaped() {
    assert_eq!(
        markdown_to_html("```rust\nfn main() {}\n```"),
        "<pre><code class=\"language-rust\">fn main() {}\n</code></pre>\n"
    );
    assert_eq!(
        markdown_to_html("- a\n- b"),
        "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n"
    );
    assert_eq!(
        markdown_to_html("<script>x</script>"),
        "<p>&lt;script&gt;x&lt;/script&gt;</p>\n"
    );
}

#[test]
fn markdown_additional_constructs() {
    // Thematic breaks, blockquotes up to depth 32, ordered lists, strong, autolinks, hard breaks, intraword underscore
    let md = "***\n> quoted\n\n1. first\n2. second\n\n**bold** and _em_ and foo_bar_baz and <https://example.test> and <user@example.test>\nline1  \nline2";
    let html = markdown_to_html(md);
    assert!(html.contains("<hr />\n"));
    assert!(html.contains("<blockquote>\n<p>quoted</p>\n</blockquote>\n"));
    assert!(html.contains("<ol>\n<li>first</li>\n<li>second</li>\n</ol>\n"));
    assert!(html.contains("<strong>bold</strong>"));
    assert!(html.contains("<em>em</em>"));
    assert!(html.contains("foo_bar_baz"));
    assert!(html.contains("<a href=\"https://example.test\">https://example.test</a>"));
    assert!(html.contains("<a href=\"mailto:user@example.test\">user@example.test</a>"));
    assert!(html.contains("line1<br />\nline2"));

    // ~~~ fenced block without info string, closing ATX hashes, backslash escapes
    let md2 = "## Title ##\n~~~\nplain\n~~~\n\\*not em\\* `` `code` ``";
    let html2 = markdown_to_html(md2);
    assert!(html2.contains("<h2>Title</h2>\n"));
    assert!(html2.contains("<pre><code>plain\n</code></pre>\n"));
    assert!(html2.contains("*not em*"));
    assert!(html2.contains("<code>`code`</code>"));
}

#[test]
fn acceptance_templates_escaping_loops_and_errors() {
    // 1. Escaping {{ page.title }} with "<b>" -> "&lt;b&gt;" and safe filter
    let tpl = Template::parse("t.html", "{{ page.title }}|{{ page.title | safe }}").unwrap();
    let ctx = Value::map([("page", Value::map([("title", Value::from("<b>"))]))]);
    let rendered = tpl.render(&ctx, &|_| None).unwrap();
    assert_eq!(rendered, "&lt;b&gt;|<b>");

    // 2. Loop over pages sorted by date descending, then title in Site
    let mut mem = MemFs::new();
    mem.insert(
        "templates/list.html",
        "{% for p in pages %}{{ p.title }},{% endfor %}",
    );
    mem.insert(
        "content/posts/a.md",
        "---\ntitle: A\ndate: 2026-01-01\nlayout: list\n---\nBody A",
    );
    mem.insert(
        "content/posts/c.md",
        "---\ntitle: C\ndate: 2026-03-01\nlayout: list\n---\nBody C",
    );
    mem.insert(
        "content/posts/b.md",
        "---\ntitle: B\ndate: 2026-02-01\nlayout: list\n---\nBody B",
    );
    let mut site = Site::new(Box::new(mem), Config::default());
    site.build().unwrap();
    let out = site
        .fs()
        .read(Path::new("public/posts/a/index.html"))
        .unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "C,B,A,");

    // 3. Unclosed {% if x %} gives template_syntax with line 1
    let err = Template::parse("bad.html", "{% if x %}hello").unwrap_err();
    assert_eq!(err.code, "template_syntax");
    assert_eq!(err.line, Some(1));

    // 4. Include cycle gives include_cycle
    let a_tpl = Template::parse("a.html", "{% include \"b.html\" %}").unwrap();
    let cycle_err = a_tpl
        .render(&Value::Map(BTreeMap::new()), &|name| match name {
            "a.html" => Some("{% include \"b.html\" %}".to_owned()),
            "b.html" => Some("{% include \"a.html\" %}".to_owned()),
            _ => None,
        })
        .unwrap_err();
    assert_eq!(cycle_err.code, "include_cycle");

    // 5. Unknown variable gives unknown_variable
    let u_tpl = Template::parse("u.html", "{{ page.missing }}").unwrap();
    let u_err = u_tpl.render(&ctx, &|_| None).unwrap_err();
    assert_eq!(u_err.code, "unknown_variable");
}

#[test]
fn template_conditionals_and_missing_include() {
    let tpl = Template::parse(
        "cond.html",
        "{% if show %}YES{% else %}NO{% endif %}-{% include \"part.html\" %}",
    )
    .unwrap();
    let ctx_yes = Value::map([("show", Value::Bool(true))]);
    let out = tpl
        .render(&ctx_yes, &|name| {
            (name == "part.html").then(|| "PART".to_owned())
        })
        .unwrap();
    assert_eq!(out, "YES-PART");

    let ctx_no = Value::map([("show", Value::Bool(false))]);
    let err = tpl.render(&ctx_no, &|_| None).unwrap_err();
    assert_eq!(err.code, "missing_template");
}

#[test]
fn acceptance_paths_drafts_and_static_copy() {
    let mut mem = MemFs::new();
    mem.insert(
        "templates/page.html",
        "<title>{{ page.title }}</title>{{ page.content | safe }}",
    );
    mem.insert(
        "content/blog/hello.md",
        "---\ntitle: Hello\ndate: 2026-01-10\n---\nHello world",
    );
    mem.insert("content/index.md", "---\ntitle: Home\n---\nWelcome");
    mem.insert(
        "content/blog/secret.md",
        "---\ntitle: Secret\ndraft: true\n---\nHidden",
    );
    let css_bytes = b"body { color: #123456; }\n\x00\xff";
    mem.insert("static/css/site.css", css_bytes.to_vec());

    let mut site = Site::new(Box::new(mem), Config::default());
    let report = site.build().unwrap();
    assert_eq!(report.rendered, 2);
    assert_eq!(report.written, 2);
    assert_eq!(report.copied, 1);

    assert!(
        site.fs()
            .read(Path::new("public/blog/hello/index.html"))
            .is_ok()
    );
    assert!(site.fs().read(Path::new("public/index.html")).is_ok());
    assert!(
        site.fs()
            .read(Path::new("public/blog/secret/index.html"))
            .is_err()
    );
    assert_eq!(
        site.fs().read(Path::new("public/css/site.css")).unwrap(),
        css_bytes
    );
}

#[test]
fn acceptance_incremental_5000_pages_in_10_sections() {
    let mut mem = MemFs::new();
    mem.insert(
        "templates/page.html",
        "<!doctype html><html><body><h1>{{ page.title }}</h1>{{ page.content | safe }}</body></html>",
    );
    mem.insert(
        "templates/post.html",
        "<!doctype html><html><body><article><h1>{{ page.title }}</h1>{{ page.content | safe }}</article></body></html>",
    );
    mem.insert(
        "templates/section.html",
        "<!doctype html><html><body><h1>{{ page.title }}</h1><ul>{% for p in pages %}<li>{{ p.title }}</li>{% endfor %}</ul></body></html>",
    );

    // 5,000 pages across 10 sections (500 per section):
    // In each section:
    // - page 0 has layout: section (the section list page)
    // - page 1 has layout: post
    // - pages 2..500 have layout: page
    for sec in 0..10 {
        for idx in 0..500 {
            let layout = if idx == 0 {
                "section"
            } else if idx == 1 {
                "post"
            } else {
                "page"
            };
            let path = format!("content/sec{sec}/page_{idx:03}.md");
            let src = format!(
                "---\ntitle: Sec {sec} Page {idx:03}\ndate: 2026-01-01\nlayout: {layout}\n---\nBody of {sec}/{idx}\n"
            );
            mem.insert(path, src);
        }
    }

    let mut site = Site::new(Box::new(mem), Config::default());
    let initial = site.build().unwrap();
    assert_eq!(initial.rendered, 5_000);
    assert_eq!(initial.written, 5_000);

    // 1. Rebuild with no changes renders 0 and writes 0 pages
    let r_noop = site.rebuild(&[]).unwrap();
    assert_eq!(r_noop.rendered, 0);
    assert_eq!(r_noop.written, 0);

    // Also passing a path whose content did not change renders 0 and writes 0
    let r_same = site
        .rebuild(&[PathBuf::from("content/sec0/page_010.md")])
        .unwrap();
    assert_eq!(r_same.rendered, 0);
    assert_eq!(r_same.written, 0);

    // 2. Editing one page body renders exactly 1
    site.fs_mut()
        .write(
            Path::new("content/sec0/page_010.md"),
            b"---\ntitle: Sec 0 Page 010\ndate: 2026-01-01\nlayout: page\n---\nUpdated body content!\n",
        )
        .unwrap();
    let r_body = site
        .rebuild(&[PathBuf::from("content/sec0/page_010.md")])
        .unwrap();
    assert_eq!(r_body.rendered, 1);
    assert_eq!(r_body.written, 1);
    assert_eq!(r_body.changed_urls, vec!["/sec0/page_010/".to_owned()]);

    // 3. Editing one title renders 2 (the page and its section list)
    site.fs_mut()
        .write(
            Path::new("content/sec0/page_010.md"),
            b"---\ntitle: Renamed Title 010\ndate: 2026-01-01\nlayout: page\n---\nUpdated body content!\n",
        )
        .unwrap();
    let r_title = site
        .rebuild(&[PathBuf::from("content/sec0/page_010.md")])
        .unwrap();
    assert_eq!(r_title.rendered, 2);
    assert_eq!(r_title.written, 2);

    // 4. Editing templates/post.html renders exactly the pages with layout post (10 pages, 1 per section)
    site.fs_mut()
        .write(
            Path::new("templates/post.html"),
            b"<!doctype html><html><body><article class=\"v2\"><h1>{{ page.title }}</h1>{{ page.content | safe }}</article></body></html>",
        )
        .unwrap();
    let r_tpl = site
        .rebuild(&[PathBuf::from("templates/post.html")])
        .unwrap();
    assert_eq!(r_tpl.rendered, 10);
    assert_eq!(r_tpl.written, 10);

    // 5. Deleting a source removes its output and lists it in report.deleted
    site.fs_mut()
        .remove(Path::new("content/sec0/page_010.md"))
        .unwrap();
    let r_del = site
        .rebuild(&[PathBuf::from("content/sec0/page_010.md")])
        .unwrap();
    assert_eq!(
        r_del.deleted,
        vec!["public/sec0/page_010/index.html".to_owned()]
    );
    assert!(
        site.fs()
            .read(Path::new("public/sec0/page_010/index.html"))
            .is_err()
    );
    // And the section list for sec0 was re-rendered without page_010
    assert_eq!(r_del.rendered, 1);
}

#[test]
fn acceptance_live_reload_and_sse_event() {
    let mut mem = MemFs::new();
    mem.insert(
        "templates/page.html",
        "<html><body><h1>{{ page.title }}</h1></body></html>",
    );
    mem.insert("content/blog/hello.md", "---\ntitle: Hello\n---\nWorld");

    // With live_reload: true
    let mut site_lr = Site::new(
        Box::new(mem.clone()),
        Config {
            live_reload: true,
            ..Config::default()
        },
    );
    let report = site_lr.build().unwrap();
    let html = String::from_utf8(
        site_lr
            .fs()
            .read(Path::new("public/blog/hello/index.html"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        html,
        "<html><body><h1>Hello</h1><script src=\"/__livereload.js\"></script></body></html>"
    );
    assert_eq!(html.matches("__livereload.js").count(), 1);
    assert_eq!(
        reload_event(&report),
        "event: reload\ndata: {\"paths\":[\"/blog/hello/\"]}\n\n"
    );

    // With live_reload: false
    let mut site_no_lr = Site::new(Box::new(mem), Config::default());
    site_no_lr.build().unwrap();
    let html_no_lr = String::from_utf8(
        site_no_lr
            .fs()
            .read(Path::new("public/blog/hello/index.html"))
            .unwrap(),
    )
    .unwrap();
    assert!(!html_no_lr.contains("__livereload"));
}

#[test]
fn acceptance_limits_and_front_matter_errors() {
    // 1. 10_485_761 bytes source gives input_too_large
    let mut mem = MemFs::new();
    mem.insert("templates/page.html", "{{ page.content | safe }}");
    let oversized = vec![b'a'; 10_485_761];
    mem.insert("content/huge.md", oversized);
    let mut site = Site::new(Box::new(mem), Config::default());
    let err = site.build().unwrap_err();
    assert_eq!(err.code, "input_too_large");

    // 2. Front matter without closing --- gives front_matter
    let mut mem2 = MemFs::new();
    mem2.insert("templates/page.html", "{{ page.content | safe }}");
    mem2.insert(
        "content/unclosed.md",
        "---\ntitle: Unclosed\nNo closing delimiter",
    );
    let mut site2 = Site::new(Box::new(mem2), Config::default());
    let err2 = site2.build().unwrap_err();
    assert_eq!(err2.code, "front_matter");

    // 3. Duplicate output paths give front_matter
    let mut mem3 = MemFs::new();
    mem3.insert("templates/page.html", "{{ page.title }}");
    mem3.insert("content/blog/index.md", "---\ntitle: Blog1\n---\nA");
    mem3.insert("content/blog.md", "---\ntitle: Blog2\n---\nB");
    let mut site3 = Site::new(Box::new(mem3), Config::default());
    let err3 = site3.build().unwrap_err();
    assert_eq!(err3.code, "front_matter");

    // 4. Missing layout gives missing_template
    let mut mem4 = MemFs::new();
    mem4.insert(
        "content/index.md",
        "---\ntitle: Home\nlayout: missing\n---\nHi",
    );
    let mut site4 = Site::new(Box::new(mem4), Config::default());
    let err4 = site4.build().unwrap_err();
    assert_eq!(err4.code, "missing_template");

    // 5. CRLF and Unicode titles
    let mut mem5 = MemFs::new();
    mem5.insert(
        "templates/page.html",
        "{{ page.title }}:{{ page.content | safe }}",
    );
    mem5.insert(
        "content/unicode.md",
        "---\r\ntitle: Café & 雪\r\ndate: 2026-04-01\r\n---\r\n# Héllo\r\n",
    );
    let mut site5 = Site::new(Box::new(mem5), Config::default());
    site5.build().unwrap();
    let u_out = String::from_utf8(
        site5
            .fs()
            .read(Path::new("public/unicode/index.html"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(u_out, "Café &amp; 雪:<h1>Héllo</h1>\n");
}

#[test]
fn acceptance_cli_invocations() {
    // CLI tests against a temporary directory
    let tmp_base =
        std::env::temp_dir().join(format!("fast_static_site_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_base);
    let site_dir = tmp_base.join("site");
    std::fs::create_dir_all(site_dir.join("templates")).unwrap();
    std::fs::create_dir_all(site_dir.join("content")).unwrap();
    std::fs::write(
        site_dir.join("templates/page.html"),
        "<html><body>{{ page.title }}</body></html>",
    )
    .unwrap();
    std::fs::write(
        site_dir.join("content/index.md"),
        "---\ntitle: CLI Home\n---\nHello",
    )
    .unwrap();

    // Test cli::run(["build", "site"]) via current_dir or path
    let prev_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp_base).unwrap();
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let code = cli::run(
        &["build".to_owned(), "site".to_owned()],
        &mut out_buf,
        &mut err_buf,
    );
    std::env::set_current_dir(&prev_dir).unwrap();
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err_buf));
    assert!(String::from_utf8_lossy(&out_buf).contains("rendered=1 written=1"));

    // Missing site directory returns 2
    let mut out_buf2 = Vec::new();
    let mut err_buf2 = Vec::new();
    let code_missing = cli::run(
        &[
            "build".to_owned(),
            tmp_base
                .join("nonexistent_site_dir")
                .to_string_lossy()
                .into_owned(),
        ],
        &mut out_buf2,
        &mut err_buf2,
    );
    assert_eq!(code_missing, 2);

    // Usage errors return 2
    assert_eq!(cli::run(&[], &mut out_buf2, &mut err_buf2), 2);
    assert_eq!(
        cli::run(&["unknown".to_owned()], &mut out_buf2, &mut err_buf2),
        2
    );
    assert_eq!(
        cli::run(
            &["build".to_owned(), "--bad-flag".to_owned()],
            &mut out_buf2,
            &mut err_buf2
        ),
        2
    );

    // Domain failure in CLI returns 1
    let bad_site_dir = tmp_base.join("bad_site");
    let mut disk = DiskFs::new(&bad_site_dir);
    disk.write(Path::new("content/bad.md"), b"no front matter")
        .unwrap();
    let code_domain = cli::run(
        &[
            "build".to_owned(),
            bad_site_dir.to_string_lossy().into_owned(),
            "--live-reload".to_owned(),
        ],
        &mut out_buf2,
        &mut err_buf2,
    );
    assert_eq!(code_domain, 1);

    let _ = std::fs::remove_dir_all(&tmp_base);
}

proptest! {
    #[test]
    fn markdown_never_panics_and_escapes_raw_angle_brackets(input in "\\PC*") {
        let html = markdown_to_html(&input);
        prop_assert!(!html.contains("<script>"));
    }

    #[test]
    fn reload_event_is_valid_sse_frame(paths in prop::collection::vec("/[a-z0-9/]{0,16}", 0..8)) {
        let report = BuildReport {
            changed_urls: paths,
            ..BuildReport::default()
        };
        let ev = reload_event(&report);
        let starts_ok = ev.starts_with("event: reload\ndata: {\"paths\":[");
        prop_assert!(starts_ok);
        let ends_ok = ev.ends_with("]}\n\n");
        prop_assert!(ends_ok);
    }
}
