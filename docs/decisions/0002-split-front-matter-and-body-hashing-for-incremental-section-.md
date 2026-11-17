# Split Front-Matter and Body Hashing for Incremental Section Rebuilds

Status: accepted (generated, pending human review)

## Context

In documentation and blog sites with thousands of pages, section index pages iterate over `pages` in their section to render titles, dates, and URLs. If every edit to a page triggered section index rebuilds, body-only edits would still re-render index pages unnecessarily.

## Decision

Compute separate 64-bit FNV-1a hashes for each Markdown file's front matter (`fm_hash`) and body (`body_hash`), and statically inspect compiled template ASTs (and their transitive includes) for references to `pages`. During `Site::rebuild`, mark a section dirty only when a page is added, removed, changes section, or changes its `fm_hash`. Body-only edits mark only the edited page dirty.

## Consequences

Editing a single page body in a 5,000-page site re-renders and writes exactly 1 page; editing a page title re-renders the page plus its section list page; editing a layout or included partial re-renders only pages whose transitive template dependency set contains that template.
