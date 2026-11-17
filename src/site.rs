use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::error::SiteError;
use crate::fs::{SiteFs, normalize_rel_path, path_to_posix};
use crate::markdown::markdown_to_html;
use crate::template::{DEFAULT_MAX_INCLUDE_DEPTH, Template, Value};

/// Default maximum size of a source or template file in bytes (10 MiB).
pub const DEFAULT_MAX_SOURCE_BYTES: usize = 10_485_760;

/// Snippet injected immediately before the last `</body>` when `live_reload` is enabled.
pub const LIVE_RELOAD_SNIPPET: &str = "<script src=\"/__livereload.js\"></script>";

/// Configuration for a [`Site`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Whether to inject the live-reload `<script>` tag before `</body>`.
    pub live_reload: bool,
    /// Maximum allowed byte length of any content or template source file.
    pub max_source_bytes: usize,
    /// Maximum allowed template `{% include %}` nesting depth.
    pub max_include_depth: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            live_reload: false,
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
            max_include_depth: DEFAULT_MAX_INCLUDE_DEPTH,
        }
    }
}

/// Summary of work performed during [`Site::build`] or [`Site::rebuild`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildReport {
    /// Number of Markdown pages rendered through their layout template.
    pub rendered: usize,
    /// Number of HTML page outputs written to `public/` (only when bytes changed).
    pub written: usize,
    /// Number of static assets copied to `public/` (only when bytes changed).
    pub copied: usize,
    /// Sorted POSIX paths (under `public/...`) deleted during this build.
    pub deleted: Vec<String>,
    /// Sorted canonical URLs (such as `"/blog/hello/"` or `"/"`) whose output changed.
    pub changed_urls: Vec<String>,
}

/// Formats a Server-Sent Events (HTML Living Standard section 9.2) `reload` message
/// listing `report.changed_urls` in sorted order.
#[must_use]
pub fn reload_event(report: &BuildReport) -> String {
    let mut urls = report.changed_urls.clone();
    urls.sort();
    urls.dedup();
    let mut json_items = Vec::with_capacity(urls.len());
    for u in &urls {
        let escaped = u.replace('\\', "\\\\").replace('"', "\\\"");
        json_items.push(format!("\"{escaped}\""));
    }
    format!(
        "event: reload\ndata: {{\"paths\":[{}]}}\n\n",
        json_items.join(",")
    )
}

/// Computes the 64-bit FNV-1a hash of `bytes`.
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrontMatter {
    title: String,
    layout: String,
    date: String,
    draft: bool,
    extra: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct PageState {
    source_path: String,
    output_path: String,
    url: String,
    section: String,
    fm: FrontMatter,
    fm_hash: u64,
    full_hash: u64,
    body_html: String,
    uses_pages: bool,
    template_deps: BTreeSet<String>,
}

/// Incremental static site generator with dependency tracking.
pub struct Site {
    fs: Box<dyn SiteFs>,
    config: Config,
    built_once: bool,
    pages: BTreeMap<String, PageState>,
    templates: BTreeMap<String, (u64, String, Template)>,
    static_hashes: BTreeMap<String, u64>,
    output_hashes: BTreeMap<String, u64>,
}

impl Site {
    /// Creates a new [`Site`] backed by `fs` and `config`.
    #[must_use]
    pub fn new(fs: Box<dyn SiteFs>, config: Config) -> Self {
        Self {
            fs,
            config,
            built_once: false,
            pages: BTreeMap::new(),
            templates: BTreeMap::new(),
            static_hashes: BTreeMap::new(),
            output_hashes: BTreeMap::new(),
        }
    }

    /// Returns an immutable reference to the underlying [`SiteFs`].
    #[must_use]
    pub fn fs(&self) -> &dyn SiteFs {
        &*self.fs
    }

    /// Returns a mutable reference to the underlying [`SiteFs`].
    pub fn fs_mut(&mut self) -> &mut dyn SiteFs {
        &mut *self.fs
    }

    /// Performs a full build of the site.
    ///
    /// # Errors
    ///
    /// Returns [`SiteError`] if any file exceeds `max_source_bytes`, front matter or
    /// templates are invalid, duplicate output paths exist, or I/O fails.
    pub fn build(&mut self) -> Result<BuildReport, SiteError> {
        self.pages.clear();
        self.templates.clear();
        self.static_hashes.clear();
        self.output_hashes.clear();
        self.built_once = false;

        let mut report = BuildReport::default();

        // 1. Load all templates under templates/
        let tpl_files = self
            .fs
            .list(Path::new("templates"))
            .map_err(|e| SiteError::new("io", e.to_string()).with_path("templates"))?;

        for rel in tpl_files {
            let posix = path_to_posix(&rel);
            if !Path::new(&posix)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
            {
                continue;
            }
            let name = posix
                .strip_prefix("templates/")
                .unwrap_or(&posix)
                .to_owned();
            let bytes = self.read_checked(&rel, &posix)?;
            let src = String::from_utf8(bytes).map_err(|e| {
                SiteError::new("io", format!("invalid UTF-8: {e}")).with_path(posix.clone())
            })?;
            let tpl = Template::parse(&name, &src)?;
            let h = fnv1a64(src.as_bytes());
            self.templates.insert(name, (h, src, tpl));
        }

        // 2. Load all content pages under content/**/*.md
        let content_files = self
            .fs
            .list(Path::new("content"))
            .map_err(|e| SiteError::new("io", e.to_string()).with_path("content"))?;

        for rel in content_files {
            let posix = path_to_posix(&rel);
            if !Path::new(&posix)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                continue;
            }
            let page_state = self.load_page(&rel, &posix)?;
            self.pages.insert(posix, page_state);
        }

        // Check for duplicate output paths among non-draft pages
        self.check_duplicate_outputs()?;

        // 3. Render all non-draft pages
        let page_keys: Vec<String> = self.pages.keys().cloned().collect();
        let section_index = self.build_section_index();
        for key in page_keys {
            let Some(p) = self.pages.get(&key).cloned() else {
                continue;
            };
            if p.fm.draft {
                continue;
            }
            let (html, tpl_deps, uses_pages) = self.render_single_page(&p, &section_index)?;
            report.rendered += 1;
            if let Some(stored) = self.pages.get_mut(&key) {
                stored.template_deps = tpl_deps;
                stored.uses_pages = uses_pages;
            }
            let out_bytes = html.into_bytes();
            let out_hash = fnv1a64(&out_bytes);
            if self.output_hashes.get(&p.output_path) != Some(&out_hash) {
                self.fs
                    .write(Path::new(&p.output_path), &out_bytes)
                    .map_err(|e| {
                        SiteError::new("io", e.to_string()).with_path(p.output_path.clone())
                    })?;
                self.output_hashes.insert(p.output_path.clone(), out_hash);
                report.written += 1;
                report.changed_urls.push(p.url.clone());
            }
        }

        // 4. Copy static files under static/**
        let static_files = self
            .fs
            .list(Path::new("static"))
            .map_err(|e| SiteError::new("io", e.to_string()).with_path("static"))?;

        for rel in static_files {
            let posix = path_to_posix(&rel);
            let bytes = self.read_checked(&rel, &posix)?;
            let h = fnv1a64(&bytes);
            self.static_hashes.insert(posix.clone(), h);
            let out_rel = static_output_path(&posix);
            if self.output_hashes.get(&out_rel) != Some(&h) {
                self.fs
                    .write(Path::new(&out_rel), &bytes)
                    .map_err(|e| SiteError::new("io", e.to_string()).with_path(out_rel.clone()))?;
                self.output_hashes.insert(out_rel, h);
                report.copied += 1;
            }
        }

        report.deleted.sort();
        report.changed_urls.sort();
        report.changed_urls.dedup();
        self.built_once = true;
        Ok(report)
    }

    fn handle_template_change(
        &mut self,
        posix: &str,
        dirty_templates: &mut BTreeSet<String>,
    ) -> Result<(), SiteError> {
        let name = posix.strip_prefix("templates/").unwrap_or(posix).to_owned();
        match self.fs.read(Path::new(posix)) {
            Ok(bytes) => {
                if bytes.len() > self.config.max_source_bytes {
                    return Err(SiteError::new(
                        "input_too_large",
                        format!(
                            "file exceeds max_source_bytes ({})",
                            self.config.max_source_bytes
                        ),
                    )
                    .with_path(posix));
                }
                let src = String::from_utf8(bytes).map_err(|e| {
                    SiteError::new("io", format!("invalid UTF-8: {e}")).with_path(posix.to_owned())
                })?;
                let new_hash = fnv1a64(src.as_bytes());
                let unchanged = self
                    .templates
                    .get(&name)
                    .is_some_and(|(h, _, _)| *h == new_hash);
                if !unchanged {
                    let tpl = Template::parse(&name, &src)?;
                    self.templates.insert(name.clone(), (new_hash, src, tpl));
                    dirty_templates.insert(name);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if self.templates.remove(&name).is_some() {
                    dirty_templates.insert(name);
                }
            }
            Err(e) => {
                return Err(SiteError::new("io", e.to_string()).with_path(posix));
            }
        }
        Ok(())
    }

    fn handle_content_change(
        &mut self,
        posix: &str,
        dirty_pages: &mut BTreeSet<String>,
        dirty_sections: &mut BTreeSet<String>,
        report: &mut BuildReport,
    ) -> Result<(), SiteError> {
        match self.fs.read(Path::new(posix)) {
            Ok(bytes) => {
                if bytes.len() > self.config.max_source_bytes {
                    return Err(SiteError::new(
                        "input_too_large",
                        format!(
                            "file exceeds max_source_bytes ({})",
                            self.config.max_source_bytes
                        ),
                    )
                    .with_path(posix));
                }
                let new_full_hash = fnv1a64(&bytes);
                if let Some(old_page) = self.pages.get(posix).cloned() {
                    if old_page.full_hash == new_full_hash {
                        return Ok(());
                    }
                    let new_page = Self::parse_page_from_bytes(posix, &bytes)?;
                    if !old_page.fm.draft
                        && (new_page.fm.draft || new_page.output_path != old_page.output_path)
                        && self.output_hashes.remove(&old_page.output_path).is_some()
                    {
                        let _ = self.fs.remove(Path::new(&old_page.output_path));
                        report.deleted.push(old_page.output_path.clone());
                        report.changed_urls.push(old_page.url.clone());
                    }
                    if old_page.fm_hash != new_page.fm_hash || old_page.section != new_page.section
                    {
                        dirty_sections.insert(old_page.section.clone());
                        dirty_sections.insert(new_page.section.clone());
                    }
                    let mut merged = new_page;
                    merged.template_deps = old_page.template_deps;
                    merged.uses_pages = old_page.uses_pages;
                    self.pages.insert(posix.to_owned(), merged);
                    dirty_pages.insert(posix.to_owned());
                } else {
                    let new_page = Self::parse_page_from_bytes(posix, &bytes)?;
                    dirty_sections.insert(new_page.section.clone());
                    self.pages.insert(posix.to_owned(), new_page);
                    dirty_pages.insert(posix.to_owned());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(old_page) = self.pages.remove(posix) {
                    dirty_sections.insert(old_page.section.clone());
                    if !old_page.fm.draft
                        && self.output_hashes.remove(&old_page.output_path).is_some()
                    {
                        self.fs
                            .remove(Path::new(&old_page.output_path))
                            .map_err(|err| {
                                SiteError::new("io", err.to_string())
                                    .with_path(old_page.output_path.clone())
                            })?;
                        report.deleted.push(old_page.output_path);
                        report.changed_urls.push(old_page.url);
                    }
                }
            }
            Err(e) => {
                return Err(SiteError::new("io", e.to_string()).with_path(posix));
            }
        }
        Ok(())
    }

    fn handle_static_change(
        &mut self,
        posix: &str,
        report: &mut BuildReport,
    ) -> Result<(), SiteError> {
        let out_rel = static_output_path(posix);
        match self.fs.read(Path::new(posix)) {
            Ok(bytes) => {
                if bytes.len() > self.config.max_source_bytes {
                    return Err(SiteError::new(
                        "input_too_large",
                        format!(
                            "file exceeds max_source_bytes ({})",
                            self.config.max_source_bytes
                        ),
                    )
                    .with_path(posix));
                }
                let h = fnv1a64(&bytes);
                self.static_hashes.insert(posix.to_owned(), h);
                if self.output_hashes.get(&out_rel) != Some(&h) {
                    self.fs.write(Path::new(&out_rel), &bytes).map_err(|e| {
                        SiteError::new("io", e.to_string()).with_path(out_rel.clone())
                    })?;
                    self.output_hashes.insert(out_rel, h);
                    report.copied += 1;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.static_hashes.remove(posix);
                if self.output_hashes.remove(&out_rel).is_some() {
                    self.fs.remove(Path::new(&out_rel)).map_err(|err| {
                        SiteError::new("io", err.to_string()).with_path(out_rel.clone())
                    })?;
                    report.deleted.push(out_rel);
                }
            }
            Err(e) => {
                return Err(SiteError::new("io", e.to_string()).with_path(posix));
            }
        }
        Ok(())
    }

    /// Incrementally rebuilds only the outputs affected by `changed` paths.
    ///
    /// If [`Site::build`] has not been called yet, performs a full build.
    ///
    /// # Errors
    ///
    /// Returns [`SiteError`] on template, front-matter, size-limit, or I/O errors.
    pub fn rebuild(&mut self, changed: &[PathBuf]) -> Result<BuildReport, SiteError> {
        if !self.built_once {
            return self.build();
        }

        let mut report = BuildReport::default();
        if changed.is_empty() {
            return Ok(report);
        }

        let mut dirty_pages: BTreeSet<String> = BTreeSet::new();
        let mut dirty_sections: BTreeSet<String> = BTreeSet::new();
        let mut dirty_templates: BTreeSet<String> = BTreeSet::new();

        let mut unique_paths = BTreeSet::new();
        for p in changed {
            unique_paths.insert(path_to_posix(&normalize_rel_path(p)));
        }

        for posix in unique_paths {
            if posix.starts_with("templates/") {
                self.handle_template_change(&posix, &mut dirty_templates)?;
            } else if posix.starts_with("content/")
                && Path::new(&posix)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                self.handle_content_change(
                    &posix,
                    &mut dirty_pages,
                    &mut dirty_sections,
                    &mut report,
                )?;
            } else if posix.starts_with("static/") {
                self.handle_static_change(&posix, &mut report)?;
            }
        }

        // Expand dirty pages from dirty templates and dirty sections
        if !dirty_templates.is_empty() || !dirty_sections.is_empty() {
            for (key, page) in &self.pages {
                if page.fm.draft {
                    continue;
                }
                if !dirty_templates.is_empty()
                    && page
                        .template_deps
                        .iter()
                        .any(|t| dirty_templates.contains(t))
                {
                    dirty_pages.insert(key.clone());
                    continue;
                }
                if !dirty_sections.is_empty()
                    && page.uses_pages
                    && dirty_sections.contains(&page.section)
                {
                    dirty_pages.insert(key.clone());
                }
            }
        }

        if !dirty_pages.is_empty() {
            self.check_duplicate_outputs()?;
            let section_index = self.build_section_index();
            for key in dirty_pages {
                let Some(p) = self.pages.get(&key).cloned() else {
                    continue;
                };
                if p.fm.draft {
                    continue;
                }
                let (html, tpl_deps, uses_pages) = self.render_single_page(&p, &section_index)?;
                report.rendered += 1;
                if let Some(stored) = self.pages.get_mut(&key) {
                    stored.template_deps = tpl_deps;
                    stored.uses_pages = uses_pages;
                }
                let out_bytes = html.into_bytes();
                let out_hash = fnv1a64(&out_bytes);
                if self.output_hashes.get(&p.output_path) != Some(&out_hash) {
                    self.fs
                        .write(Path::new(&p.output_path), &out_bytes)
                        .map_err(|e| {
                            SiteError::new("io", e.to_string()).with_path(p.output_path.clone())
                        })?;
                    self.output_hashes.insert(p.output_path.clone(), out_hash);
                    report.written += 1;
                    report.changed_urls.push(p.url.clone());
                }
            }
        }

        report.deleted.sort();
        report.changed_urls.sort();
        report.changed_urls.dedup();
        Ok(report)
    }

    fn read_checked(&self, rel: &Path, posix: &str) -> Result<Vec<u8>, SiteError> {
        let bytes = self
            .fs
            .read(rel)
            .map_err(|e| SiteError::new("io", e.to_string()).with_path(posix.to_owned()))?;
        if bytes.len() > self.config.max_source_bytes {
            return Err(SiteError::new(
                "input_too_large",
                format!(
                    "file exceeds max_source_bytes ({})",
                    self.config.max_source_bytes
                ),
            )
            .with_path(posix.to_owned()));
        }
        Ok(bytes)
    }

    fn load_page(&self, rel: &Path, posix: &str) -> Result<PageState, SiteError> {
        let bytes = self.read_checked(rel, posix)?;
        Self::parse_page_from_bytes(posix, &bytes)
    }

    fn parse_page_from_bytes(posix: &str, bytes: &[u8]) -> Result<PageState, SiteError> {
        let src = std::str::from_utf8(bytes).map_err(|e| {
            SiteError::new("io", format!("invalid UTF-8: {e}")).with_path(posix.to_owned())
        })?;
        let full_hash = fnv1a64(bytes);
        let (fm, fm_raw, body_raw) = parse_front_matter(posix, src)?;
        let fm_hash = fnv1a64(fm_raw.as_bytes());
        let body_html = markdown_to_html(body_raw);
        let (output_path, url, section) = compute_page_routes(posix, &fm);

        Ok(PageState {
            source_path: posix.to_owned(),
            output_path,
            url,
            section,
            fm,
            fm_hash,
            full_hash,
            body_html,
            uses_pages: false,
            template_deps: BTreeSet::new(),
        })
    }

    fn check_duplicate_outputs(&self) -> Result<(), SiteError> {
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for p in self.pages.values() {
            if p.fm.draft {
                continue;
            }
            if let Some(prev_src) = seen.insert(&p.output_path, &p.source_path) {
                return Err(SiteError::new(
                    "front_matter",
                    format!(
                        "duplicate output path '{}' from '{}' and '{}'",
                        p.output_path, prev_src, p.source_path
                    ),
                )
                .with_path(p.source_path.clone()));
            }
        }
        Ok(())
    }

    fn build_section_index(&self) -> BTreeMap<String, Vec<Value>> {
        let mut by_section: BTreeMap<String, Vec<&PageState>> = BTreeMap::new();
        for p in self.pages.values() {
            if p.fm.draft {
                continue;
            }
            by_section.entry(p.section.clone()).or_default().push(p);
        }
        let mut out = BTreeMap::new();
        for (sec, mut list) in by_section {
            // Sorted by date descending, then title ascending
            list.sort_by(|a, b| {
                b.fm.date
                    .cmp(&a.fm.date)
                    .then_with(|| a.fm.title.cmp(&b.fm.title))
            });
            let vals = list.into_iter().map(page_summary_value).collect();
            out.insert(sec, vals);
        }
        out
    }

    fn render_single_page(
        &self,
        page: &PageState,
        section_index: &BTreeMap<String, Vec<Value>>,
    ) -> Result<(String, BTreeSet<String>, bool), SiteError> {
        let layout_file = if Path::new(&page.fm.layout)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
        {
            page.fm.layout.clone()
        } else {
            format!("{}.html", page.fm.layout)
        };

        let Some((_, _, tpl)) = self.templates.get(&layout_file) else {
            return Err(SiteError::new(
                "missing_template",
                format!("missing layout template '{layout_file}'"),
            )
            .with_path(page.source_path.clone()));
        };

        // Collect transitive template dependencies and check whether any template in the chain uses `pages`
        let mut tpl_deps = BTreeSet::new();
        let mut uses_pages = false;
        let mut queue = vec![layout_file.clone()];
        while let Some(cur) = queue.pop() {
            if !tpl_deps.insert(cur.clone()) {
                continue;
            }
            if let Some((_, _, t)) = self.templates.get(&cur) {
                if t.references_pages_directly() {
                    uses_pages = true;
                }
                for inc in t.direct_includes() {
                    queue.push(inc);
                }
            }
        }

        let mut page_map = BTreeMap::new();
        page_map.insert("title".to_owned(), Value::Str(page.fm.title.clone()));
        page_map.insert("date".to_owned(), Value::Str(page.fm.date.clone()));
        page_map.insert("url".to_owned(), Value::Str(page.url.clone()));
        page_map.insert("content".to_owned(), Value::Str(page.body_html.clone()));
        for (k, v) in &page.fm.extra {
            page_map
                .entry(k.clone())
                .or_insert_with(|| Value::Str(v.clone()));
        }

        let section_pages = section_index
            .get(&page.section)
            .cloned()
            .unwrap_or_default();

        let mut site_map = BTreeMap::new();
        site_map.insert(
            "live_reload".to_owned(),
            Value::Bool(self.config.live_reload),
        );
        site_map.insert(
            "max_source_bytes".to_owned(),
            Value::Int(i64::try_from(self.config.max_source_bytes).unwrap_or(i64::MAX)),
        );
        site_map.insert(
            "max_include_depth".to_owned(),
            Value::Int(i64::try_from(self.config.max_include_depth).unwrap_or(i64::MAX)),
        );

        let mut root_ctx = BTreeMap::new();
        root_ctx.insert("page".to_owned(), Value::Map(page_map));
        root_ctx.insert("pages".to_owned(), Value::List(section_pages));
        root_ctx.insert("site".to_owned(), Value::Map(site_map));
        let ctx = Value::Map(root_ctx);

        let templates_ref = &self.templates;
        let loader = |inc_name: &str| -> Option<String> {
            templates_ref.get(inc_name).map(|(_, src, _)| src.clone())
        };

        let mut rendered =
            tpl.render_with_max_depth(&ctx, &loader, self.config.max_include_depth)?;

        if self.config.live_reload {
            rendered = inject_live_reload(&rendered);
        }

        Ok((rendered, tpl_deps, uses_pages))
    }
}

fn page_summary_value(p: &PageState) -> Value {
    let mut m = BTreeMap::new();
    m.insert("title".to_owned(), Value::Str(p.fm.title.clone()));
    m.insert("date".to_owned(), Value::Str(p.fm.date.clone()));
    m.insert("url".to_owned(), Value::Str(p.url.clone()));
    m.insert("content".to_owned(), Value::Str(p.body_html.clone()));
    for (k, v) in &p.fm.extra {
        m.entry(k.clone()).or_insert_with(|| Value::Str(v.clone()));
    }
    Value::Map(m)
}

fn inject_live_reload(html: &str) -> String {
    if let Some(pos) = html.rfind("</body>") {
        let mut out = String::with_capacity(html.len() + LIVE_RELOAD_SNIPPET.len());
        out.push_str(&html[..pos]);
        out.push_str(LIVE_RELOAD_SNIPPET);
        out.push_str(&html[pos..]);
        out
    } else {
        let mut out = String::with_capacity(html.len() + LIVE_RELOAD_SNIPPET.len());
        out.push_str(html);
        out.push_str(LIVE_RELOAD_SNIPPET);
        out
    }
}

fn static_output_path(posix: &str) -> String {
    let rel = posix.strip_prefix("static/").unwrap_or(posix);
    format!("public/{rel}")
}

fn compute_page_routes(posix: &str, fm: &FrontMatter) -> (String, String, String) {
    let rel = posix.strip_prefix("content/").unwrap_or(posix);
    let without_ext = rel.strip_suffix(".md").unwrap_or(rel);

    // Section is the top-level directory under content/ (or "" for root pages)
    let section = match without_ext.split_once('/') {
        Some((first, _)) => first.to_owned(),
        None => String::new(),
    };

    // Allow custom output override via `slug` or `permalink` in front matter if provided
    if let Some(custom_url) = fm.extra.get("url").or_else(|| fm.extra.get("permalink")) {
        let trimmed = custom_url.trim_matches('/');
        if trimmed.is_empty() {
            return ("public/index.html".to_owned(), "/".to_owned(), section);
        }
        return (
            format!("public/{trimmed}/index.html"),
            format!("/{trimmed}/"),
            section,
        );
    }

    if without_ext == "index" {
        ("public/index.html".to_owned(), "/".to_owned(), section)
    } else if let Some(dir) = without_ext.strip_suffix("/index") {
        (
            format!("public/{dir}/index.html"),
            format!("/{dir}/"),
            section,
        )
    } else {
        (
            format!("public/{without_ext}/index.html"),
            format!("/{without_ext}/"),
            section,
        )
    }
}

fn parse_front_matter_fields(posix: &str, fm_raw: &str) -> Result<FrontMatter, SiteError> {
    let mut title = String::new();
    let mut layout = "page".to_owned();
    let mut date = String::new();
    let mut draft = false;
    let mut extra = BTreeMap::new();

    for (idx, raw_line) in fm_raw.lines().enumerate() {
        let line_no = idx + 2;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            return Err(
                SiteError::new("front_matter", "expected key: value pair in front matter")
                    .with_path(posix.to_owned())
                    .with_line(line_no),
            );
        };
        let key = k.trim();
        if key.is_empty() {
            return Err(SiteError::new("front_matter", "empty key in front matter")
                .with_path(posix.to_owned())
                .with_line(line_no));
        }
        let mut val = v.trim();
        if val.len() >= 2
            && ((val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\'')))
        {
            val = &val[1..val.len() - 1];
        }

        match key {
            "title" => val.clone_into(&mut title),
            "layout" => {
                if val.is_empty() {
                    return Err(SiteError::new("front_matter", "layout cannot be empty")
                        .with_path(posix.to_owned())
                        .with_line(line_no));
                }
                val.clone_into(&mut layout);
            }
            "date" => {
                if !val.is_empty() && !is_valid_date(val) {
                    return Err(SiteError::new("front_matter", "date must be YYYY-MM-DD")
                        .with_path(posix.to_owned())
                        .with_line(line_no));
                }
                val.clone_into(&mut date);
            }
            "draft" => match val {
                "true" => draft = true,
                "false" => draft = false,
                _ => {
                    return Err(
                        SiteError::new("front_matter", "draft must be true or false")
                            .with_path(posix.to_owned())
                            .with_line(line_no),
                    );
                }
            },
            other => {
                extra.insert(other.to_owned(), val.to_owned());
            }
        }
    }

    Ok(FrontMatter {
        title,
        layout,
        date,
        draft,
        extra,
    })
}

fn parse_front_matter<'a>(
    posix: &str,
    src: &'a str,
) -> Result<(FrontMatter, &'a str, &'a str), SiteError> {
    let after_open = if let Some(r) = src.strip_prefix("---\r\n") {
        r
    } else if let Some(r) = src.strip_prefix("---\n") {
        r
    } else {
        return Err(
            SiteError::new("front_matter", "missing opening --- front matter delimiter")
                .with_path(posix.to_owned())
                .with_line(1),
        );
    };

    // Find closing `---` on its own line
    let mut search_idx = 0;
    let mut found_close: Option<(usize, usize)> = None;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            found_close = Some((search_idx, search_idx + line.len()));
            break;
        }
        search_idx += line.len();
    }

    let Some((fm_end, body_start)) = found_close else {
        return Err(SiteError::new(
            "front_matter",
            "unclosed front matter (missing closing ---)",
        )
        .with_path(posix.to_owned())
        .with_line(1));
    };

    let fm_raw = &after_open[..fm_end];
    let body_raw = &after_open[body_start..];
    let fm = parse_front_matter_fields(posix, fm_raw)?;

    Ok((fm, fm_raw, body_raw))
}

fn is_valid_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let b = s.as_bytes();
    b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && b[7] == b'-'
        && b[8].is_ascii_digit()
        && b[9].is_ascii_digit()
}
