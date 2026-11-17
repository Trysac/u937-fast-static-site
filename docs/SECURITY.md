# Security & Threat Model

## Untrusted Input Limits

- **Source Size Limit**: Every source, template, and static file read by `Site` enforces `Config::max_source_bytes` (default `10_485_760` bytes / 10 MiB) and returns `SiteError` with code `"input_too_large"` before parsing.
- **Recursion & Nesting Limits**:
  - Template `{% include %}` chains detect cycles and enforce `Config::max_include_depth` (default `16`), returning `"include_cycle"`.
  - Markdown block quotes cap nesting at depth `32`.
- **XSS Prevention**:
  - Raw HTML inside Markdown sources is always escaped (`&`, `<`, `>`, `"`).
  - Template expressions `{{ path }}` escape `&`, `<`, `>`, `"`, and `'` (`&#39;`) unless explicitly marked `{{ path | safe }}`.
- **No Unsafe Code**: `#![forbid(unsafe_code)]` is enforced across the crate.
