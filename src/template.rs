use std::collections::{BTreeMap, BTreeSet};

use crate::error::SiteError;
use crate::markdown::escape_html;

/// Default maximum include nesting depth.
pub const DEFAULT_MAX_INCLUDE_DEPTH: usize = 16;

/// Dynamic value passed in the rendering context of a [`Template`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// UTF-8 string value.
    Str(String),
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
    /// Sequence of values (used in `{% for x in list %}`).
    List(Vec<Value>),
    /// Key-value map (used for `page`, `site`, or loop item fields).
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// Creates a [`Value::Map`] from an iterator of key-value pairs.
    #[must_use]
    pub fn map<I, K>(iter: I) -> Self
    where
        I: IntoIterator<Item = (K, Value)>,
        K: Into<String>,
    {
        let mut m = BTreeMap::new();
        for (k, v) in iter {
            m.insert(k.into(), v);
        }
        Self::Map(m)
    }

    /// Evaluates truthiness for `{% if path %}`.
    #[must_use]
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Str(s) => !s.is_empty(),
            Self::Int(n) => *n != 0,
            Self::List(l) => !l.is_empty(),
            Self::Map(m) => !m.is_empty(),
        }
    }

    /// Converts a scalar value into its string representation.
    #[must_use]
    pub fn as_display_str(&self) -> Option<String> {
        match self {
            Self::Str(s) => Some(s.clone()),
            Self::Bool(b) => Some(b.to_string()),
            Self::Int(n) => Some(n.to_string()),
            Self::List(_) | Self::Map(_) => None,
        }
    }

    /// Resolves a dotted path like `page.title` against this value.
    #[must_use]
    pub fn resolve_path<'a>(&'a self, path: &str) -> Option<&'a Value> {
        let mut cur = self;
        for segment in path.split('.') {
            match cur {
                Self::Map(m) => {
                    cur = m.get(segment)?;
                }
                _ => return None,
            }
        }
        Some(cur)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::Str(s.to_owned())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Self::Int(n)
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Self::List(v)
    }
}

impl From<BTreeMap<String, Value>> for Value {
    fn from(m: BTreeMap<String, Value>) -> Self {
        Self::Map(m)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Literal(String),
    Var {
        path: String,
        safe: bool,
        line: usize,
    },
    If {
        cond_path: String,
        then_branch: Vec<Node>,
        else_branch: Vec<Node>,
        line: usize,
    },
    For {
        var_name: String,
        list_path: String,
        body: Vec<Node>,
        line: usize,
    },
    Include {
        template_name: String,
        line: usize,
    },
}

/// Compiled strict HTML template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    name: String,
    nodes: Vec<Node>,
}

#[derive(Debug)]
enum Token {
    Text(String),
    Expr(String, usize),
    Tag(String, usize),
}

impl Template {
    /// Parses `src` into a [`Template`] named `name`.
    ///
    /// # Errors
    ///
    /// Returns [`SiteError`] with code `"template_syntax"` and the 1-based line number
    /// if a delimiter or control block is malformed or unclosed.
    pub fn parse(name: &str, src: &str) -> Result<Self, SiteError> {
        let tokens = tokenize(name, src)?;
        let mut pos = 0;
        let (nodes, stop) = parse_nodes(name, &tokens, &mut pos, &[])?;
        if stop.is_some() {
            return Err(SiteError::new("template_syntax", "unexpected closing tag")
                .with_path(name.to_owned())
                .with_line(1));
        }
        Ok(Self {
            name: name.to_owned(),
            nodes,
        })
    }

    /// Returns the template's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether this template (not counting includes) references the `pages` variable.
    #[must_use]
    pub fn references_pages_directly(&self) -> bool {
        nodes_reference_pages(&self.nodes)
    }

    /// Returns the list of template names directly referenced by `{% include "..." %}` in this template.
    #[must_use]
    pub fn direct_includes(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        collect_includes(&self.nodes, &mut out);
        out.into_iter().collect()
    }

    /// Renders the template with `ctx` and an include `loader`.
    ///
    /// # Errors
    ///
    /// Returns [`SiteError`] with code:
    /// - `"unknown_variable"` when a referenced variable path does not exist in `ctx`
    /// - `"missing_template"` when an included template is not found by `loader`
    /// - `"include_cycle"` when cyclic includes or `DEFAULT_MAX_INCLUDE_DEPTH` is exceeded
    /// - `"template_syntax"` when an included template fails to parse
    pub fn render(
        &self,
        ctx: &Value,
        loader: &dyn Fn(&str) -> Option<String>,
    ) -> Result<String, SiteError> {
        self.render_with_max_depth(ctx, loader, DEFAULT_MAX_INCLUDE_DEPTH)
    }

    /// Renders the template with an explicit maximum include depth.
    ///
    /// # Errors
    ///
    /// Same error conditions as [`Template::render`].
    pub fn render_with_max_depth(
        &self,
        ctx: &Value,
        loader: &dyn Fn(&str) -> Option<String>,
        max_depth: usize,
    ) -> Result<String, SiteError> {
        let mut stack = vec![self.name.clone()];
        let mut scopes = vec![ctx.clone()];
        let mut out = String::new();
        render_nodes(
            &self.name,
            &self.nodes,
            &mut scopes,
            loader,
            &mut stack,
            max_depth,
            &mut out,
        )?;
        Ok(out)
    }
}

fn nodes_reference_pages(nodes: &[Node]) -> bool {
    for node in nodes {
        match node {
            Node::Literal(_) | Node::Include { .. } => {}
            Node::Var { path, .. } => {
                if path == "pages" || path.starts_with("pages.") {
                    return true;
                }
            }
            Node::If {
                cond_path,
                then_branch,
                else_branch,
                ..
            } => {
                if cond_path == "pages"
                    || cond_path.starts_with("pages.")
                    || nodes_reference_pages(then_branch)
                    || nodes_reference_pages(else_branch)
                {
                    return true;
                }
            }
            Node::For {
                list_path, body, ..
            } => {
                if list_path == "pages"
                    || list_path.starts_with("pages.")
                    || nodes_reference_pages(body)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn collect_includes(nodes: &[Node], out: &mut BTreeSet<String>) {
    for node in nodes {
        match node {
            Node::Include { template_name, .. } => {
                out.insert(template_name.clone());
            }
            Node::If {
                then_branch,
                else_branch,
                ..
            } => {
                collect_includes(then_branch, out);
                collect_includes(else_branch, out);
            }
            Node::For { body, .. } => {
                collect_includes(body, out);
            }
            Node::Literal(_) | Node::Var { .. } => {}
        }
    }
}

fn line_at_offset(src: &str, offset: usize) -> usize {
    1 + src[..offset].bytes().filter(|&b| b == b'\n').count()
}

fn tokenize(name: &str, src: &str) -> Result<Vec<Token>, SiteError> {
    let mut tokens = Vec::new();
    let mut i = 0;
    let len = src.len();

    while i < len {
        let rest = &src[i..];
        let next_expr = rest.find("{{");
        let next_tag = rest.find("{%");

        let next_open = match (next_expr, next_tag) {
            (Some(a), Some(b)) => Some((a.min(b), a <= b)),
            (Some(a), None) => Some((a, true)),
            (None, Some(b)) => Some((b, false)),
            (None, None) => None,
        };

        if let Some((rel_pos, is_expr)) = next_open {
            if rel_pos > 0 {
                tokens.push(Token::Text(rest[..rel_pos].to_owned()));
            }
            let open_abs = i + rel_pos;
            let line = line_at_offset(src, open_abs);
            let after_open = open_abs + 2;
            let close_pat = if is_expr { "}}" } else { "%}" };
            if let Some(close_rel) = src[after_open..].find(close_pat) {
                let inner = src[after_open..after_open + close_rel].trim().to_owned();
                if is_expr {
                    tokens.push(Token::Expr(inner, line));
                } else {
                    tokens.push(Token::Tag(inner, line));
                }
                i = after_open + close_rel + 2;
            } else {
                return Err(
                    SiteError::new("template_syntax", "unclosed template delimiter")
                        .with_path(name.to_owned())
                        .with_line(line),
                );
            }
        } else {
            tokens.push(Token::Text(rest.to_owned()));
            break;
        }
    }

    Ok(tokens)
}

type StopTag = Option<(String, usize)>;
type ParsedNodes = (Vec<Node>, StopTag);

fn parse_if_tag(
    name: &str,
    rest: &str,
    line: usize,
    tokens: &[Token],
    pos: &mut usize,
) -> Result<Node, SiteError> {
    let cond_path = rest.trim();
    if !is_valid_path(cond_path) {
        return Err(
            SiteError::new("template_syntax", "invalid if condition path")
                .with_path(name.to_owned())
                .with_line(line),
        );
    }
    *pos += 1;
    let (then_branch, stop) = parse_nodes(name, tokens, pos, &["else", "endif"])?;
    match stop.as_ref() {
        Some((tag, _)) if tag == "endif" => Ok(Node::If {
            cond_path: cond_path.to_owned(),
            then_branch,
            else_branch: Vec::new(),
            line,
        }),
        Some((tag, _)) if tag == "else" => {
            let (else_branch, stop2) = parse_nodes(name, tokens, pos, &["endif"])?;
            if !matches!(stop2.as_ref(), Some((tag2, _)) if tag2 == "endif") {
                return Err(SiteError::new("template_syntax", "unclosed if block")
                    .with_path(name.to_owned())
                    .with_line(line));
            }
            Ok(Node::If {
                cond_path: cond_path.to_owned(),
                then_branch,
                else_branch,
                line,
            })
        }
        _ => Err(SiteError::new("template_syntax", "unclosed if block")
            .with_path(name.to_owned())
            .with_line(line)),
    }
}

fn parse_for_tag(
    name: &str,
    rest: &str,
    line: usize,
    tokens: &[Token],
    pos: &mut usize,
) -> Result<Node, SiteError> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() != 3 || parts[1] != "in" || !is_valid_ident(parts[0]) || !is_valid_path(parts[2])
    {
        return Err(SiteError::new("template_syntax", "malformed for tag")
            .with_path(name.to_owned())
            .with_line(line));
    }
    let var_name = parts[0].to_owned();
    let list_path = parts[2].to_owned();
    *pos += 1;
    let (body, stop) = parse_nodes(name, tokens, pos, &["endfor"])?;
    if !matches!(stop.as_ref(), Some((tag, _)) if tag == "endfor") {
        return Err(SiteError::new("template_syntax", "unclosed for block")
            .with_path(name.to_owned())
            .with_line(line));
    }
    Ok(Node::For {
        var_name,
        list_path,
        body,
        line,
    })
}

fn parse_include_tag(name: &str, rest: &str, line: usize) -> Result<Node, SiteError> {
    let arg = rest.trim();
    if arg.len() < 2 || !arg.starts_with('"') || !arg.ends_with('"') {
        return Err(
            SiteError::new("template_syntax", "include requires quoted template name")
                .with_path(name.to_owned())
                .with_line(line),
        );
    }
    let include_name = arg[1..arg.len() - 1].to_owned();
    if include_name.is_empty() {
        return Err(
            SiteError::new("template_syntax", "empty include template name")
                .with_path(name.to_owned())
                .with_line(line),
        );
    }
    Ok(Node::Include {
        template_name: include_name,
        line,
    })
}

fn parse_nodes(
    name: &str,
    tokens: &[Token],
    pos: &mut usize,
    stop_tags: &[&str],
) -> Result<ParsedNodes, SiteError> {
    let mut nodes = Vec::new();

    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Text(t) => {
                nodes.push(Node::Literal(t.clone()));
                *pos += 1;
            }
            Token::Expr(expr, line) => {
                let line = *line;
                let (path, safe) = parse_expr(name, expr, line)?;
                nodes.push(Node::Var { path, safe, line });
                *pos += 1;
            }
            Token::Tag(tag, line) => {
                let line = *line;
                let trimmed = tag.trim();
                if stop_tags.contains(&trimmed) {
                    *pos += 1;
                    return Ok((nodes, Some((trimmed.to_owned(), line))));
                }

                if let Some(rest) = trimmed.strip_prefix("if ") {
                    let node = parse_if_tag(name, rest, line, tokens, pos)?;
                    nodes.push(node);
                } else if let Some(rest) = trimmed.strip_prefix("for ") {
                    let node = parse_for_tag(name, rest, line, tokens, pos)?;
                    nodes.push(node);
                } else if let Some(rest) = trimmed.strip_prefix("include ") {
                    let node = parse_include_tag(name, rest, line)?;
                    nodes.push(node);
                    *pos += 1;
                } else {
                    return Err(SiteError::new(
                        "template_syntax",
                        format!("unknown or misplaced tag: {trimmed}"),
                    )
                    .with_path(name.to_owned())
                    .with_line(line));
                }
            }
        }
    }

    Ok((nodes, None))
}

fn parse_expr(name: &str, expr: &str, line: usize) -> Result<(String, bool), SiteError> {
    if expr.is_empty() {
        return Err(SiteError::new("template_syntax", "empty expression")
            .with_path(name.to_owned())
            .with_line(line));
    }
    if let Some((left, right)) = expr.split_once('|') {
        let path = left.trim();
        let filter = right.trim();
        if filter != "safe" || !is_valid_path(path) {
            return Err(
                SiteError::new("template_syntax", "invalid filter or variable path")
                    .with_path(name.to_owned())
                    .with_line(line),
            );
        }
        Ok((path.to_owned(), true))
    } else if is_valid_path(expr) {
        Ok((expr.to_owned(), false))
    } else {
        Err(SiteError::new("template_syntax", "invalid variable path")
            .with_path(name.to_owned())
            .with_line(line))
    }
}

fn is_valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !s.chars().next().unwrap_or('0').is_ascii_digit()
}

fn is_valid_path(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(is_valid_ident)
}

fn lookup_in_scopes<'a>(scopes: &'a [Value], path: &str) -> Option<&'a Value> {
    for scope in scopes.iter().rev() {
        if let Some(val) = scope.resolve_path(path) {
            return Some(val);
        }
    }
    None
}

fn render_var(
    template_name: &str,
    path: &str,
    safe: bool,
    line: usize,
    scopes: &[Value],
    out: &mut String,
) -> Result<(), SiteError> {
    let val = lookup_in_scopes(scopes, path).ok_or_else(|| {
        SiteError::new("unknown_variable", format!("unknown variable '{path}'"))
            .with_path(template_name.to_owned())
            .with_line(line)
    })?;
    let rendered = val.as_display_str().ok_or_else(|| {
        SiteError::new(
            "unknown_variable",
            format!("variable '{path}' is not a printable scalar"),
        )
        .with_path(template_name.to_owned())
        .with_line(line)
    })?;
    if safe {
        out.push_str(&rendered);
    } else {
        out.push_str(&escape_html(&rendered));
    }
    Ok(())
}

struct RenderContext<'a> {
    template_name: &'a str,
    scopes: &'a mut Vec<Value>,
    loader: &'a dyn Fn(&str) -> Option<String>,
    include_stack: &'a mut Vec<String>,
    max_depth: usize,
    out: &'a mut String,
}

fn render_if(
    cond_path: &str,
    then_branch: &[Node],
    else_branch: &[Node],
    line: usize,
    ctx: &mut RenderContext<'_>,
) -> Result<(), SiteError> {
    let val = lookup_in_scopes(ctx.scopes, cond_path).ok_or_else(|| {
        SiteError::new(
            "unknown_variable",
            format!("unknown variable '{cond_path}'"),
        )
        .with_path(ctx.template_name.to_owned())
        .with_line(line)
    })?;
    let branch = if val.is_truthy() {
        then_branch
    } else {
        else_branch
    };
    render_nodes(
        ctx.template_name,
        branch,
        ctx.scopes,
        ctx.loader,
        ctx.include_stack,
        ctx.max_depth,
        ctx.out,
    )
}

fn render_for(
    var_name: &str,
    list_path: &str,
    body: &[Node],
    line: usize,
    ctx: &mut RenderContext<'_>,
) -> Result<(), SiteError> {
    let val = lookup_in_scopes(ctx.scopes, list_path)
        .cloned()
        .ok_or_else(|| {
            SiteError::new(
                "unknown_variable",
                format!("unknown variable '{list_path}'"),
            )
            .with_path(ctx.template_name.to_owned())
            .with_line(line)
        })?;
    let Value::List(items) = val else {
        return Err(SiteError::new(
            "unknown_variable",
            format!("variable '{list_path}' is not a list"),
        )
        .with_path(ctx.template_name.to_owned())
        .with_line(line));
    };
    for item in items {
        let mut loop_scope = BTreeMap::new();
        loop_scope.insert(var_name.to_owned(), item);
        ctx.scopes.push(Value::Map(loop_scope));
        let res = render_nodes(
            ctx.template_name,
            body,
            ctx.scopes,
            ctx.loader,
            ctx.include_stack,
            ctx.max_depth,
            ctx.out,
        );
        ctx.scopes.pop();
        res?;
    }
    Ok(())
}

fn render_include(
    inc_name: &str,
    line: usize,
    ctx: &mut RenderContext<'_>,
) -> Result<(), SiteError> {
    if ctx.include_stack.contains(&inc_name.to_owned()) || ctx.include_stack.len() > ctx.max_depth {
        return Err(SiteError::new(
            "include_cycle",
            format!("include cycle or max depth exceeded for '{inc_name}'"),
        )
        .with_path(ctx.template_name.to_owned())
        .with_line(line));
    }
    let src = (ctx.loader)(inc_name).ok_or_else(|| {
        SiteError::new(
            "missing_template",
            format!("missing included template '{inc_name}'"),
        )
        .with_path(ctx.template_name.to_owned())
        .with_line(line)
    })?;
    let parsed = Template::parse(inc_name, &src)?;
    ctx.include_stack.push(inc_name.to_owned());
    let res = render_nodes(
        inc_name,
        &parsed.nodes,
        ctx.scopes,
        ctx.loader,
        ctx.include_stack,
        ctx.max_depth,
        ctx.out,
    );
    ctx.include_stack.pop();
    res
}

fn render_nodes(
    template_name: &str,
    nodes: &[Node],
    scopes: &mut Vec<Value>,
    loader: &dyn Fn(&str) -> Option<String>,
    include_stack: &mut Vec<String>,
    max_depth: usize,
    out: &mut String,
) -> Result<(), SiteError> {
    for node in nodes {
        match node {
            Node::Literal(text) => out.push_str(text),
            Node::Var { path, safe, line } => {
                render_var(template_name, path, *safe, *line, scopes, out)?;
            }
            Node::If {
                cond_path,
                then_branch,
                else_branch,
                line,
            } => {
                let mut ctx = RenderContext {
                    template_name,
                    scopes,
                    loader,
                    include_stack,
                    max_depth,
                    out,
                };
                render_if(cond_path, then_branch, else_branch, *line, &mut ctx)?;
            }
            Node::For {
                var_name,
                list_path,
                body,
                line,
            } => {
                let mut ctx = RenderContext {
                    template_name,
                    scopes,
                    loader,
                    include_stack,
                    max_depth,
                    out,
                };
                render_for(var_name, list_path, body, *line, &mut ctx)?;
            }
            Node::Include {
                template_name: inc_name,
                line,
            } => {
                let mut ctx = RenderContext {
                    template_name,
                    scopes,
                    loader,
                    include_stack,
                    max_depth,
                    out,
                };
                render_include(inc_name, *line, &mut ctx)?;
            }
        }
    }
    Ok(())
}
