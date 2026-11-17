# trysac-fast-static-site

Incremental static site generator with a hand-written `CommonMark` 0.31.2 subset, strict HTML templates, and a fine-grained dependency graph that rebuilds only the pages touched by a change.

## Usage

```rust
use std::path::PathBuf;
use fast_static_site::{Config, MemFs, Site, markdown_to_html};

let html = markdown_to_html("# Hi *there*\n\nA `x<y` [link](https://example.test \"T\")");
assert!(html.starts_with("<h1>Hi <em>there</em></h1>"));

let mut fs = MemFs::new();
fs.insert("templates/page.html", "<h1>{{ page.title }}</h1>{{ page.content | safe }}");
fs.insert("content/index.md", "---\ntitle: Home\n---\nHello!");
let mut site = Site::new(Box::new(fs), Config::default());
assert_eq!(site.build().unwrap().rendered, 1);
assert_eq!(site.rebuild(&Vec::<PathBuf>::new()).unwrap().rendered, 0);
```

## Features

- **`CommonMark` 0.31.2 Subset**: Thematic breaks, ATX headings, fenced code blocks with `language-<info>`, paragraphs, block quotes (up to depth 32), tight bullet and ordered lists, code spans, emphasis/strong (`*` and `_` without intraword `_`), inline links with titles, autolinks, and hard line breaks. Raw HTML in Markdown is always escaped.
- **Strict Templates**: `{{ path }}` (HTML-escaped), `{{ path | safe }}`, `{% if path %}` / `{% else %}` / `{% endif %}`, `{% for x in path %}` / `{% endfor %}`, and `{% include "name.html" %}` with cycle detection and unknown-variable errors.
- **Incremental Dependency Graph**: 64-bit FNV-1a content hashing of front matter, Markdown bodies, templates, and outputs. Editing a page body re-renders only that page; editing front matter re-renders that page and section index pages in the same section.
- **Live Reload Protocol**: Injects `<script src="/__livereload.js"></script>` before `</body>` when enabled and formats Server-Sent Events (`reload_event`).
