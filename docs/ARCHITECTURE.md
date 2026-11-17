# Architecture

`fast_static_site` is organized into five cohesive modules using only the Rust standard library:

1. `src/fs.rs` (`SiteFs`, `DiskFs`, `MemFs`): Abstracts all filesystem operations behind relative POSIX-normalized paths so the generator runs deterministically in memory during unit tests or against a directory on disk via the CLI.
2. `src/markdown.rs` (`markdown_to_html`, `escape_html`): Implements a two-phase CommonMark 0.31.2 subset parser (block structure followed by inline runs) with raw HTML escaping and a nesting cap of 32 on block quotes.
3. `src/template.rs` (`Template`, `Value`): Tokenizes and compiles templates into an AST (`Literal`, `Var`, `If`, `For`, `Include`), inspects ASTs for direct `pages` usage and `{% include %}` edges, and evaluates variables strictly.
4. `src/site.rs` (`Site`, `Config`, `BuildReport`, `reload_event`): Tracks 64-bit FNV-1a hashes separately for each page's front matter and body, plus template hashes and output hashes. Re-renders only pages affected by changed files and writes outputs only when the resulting HTML bytes differ.
5. `src/cli.rs` (`run`): Thin command-line driver for `fast-static-site build <dir> [--live-reload]`.
