//! Tools that look at the project and tools that propose changes to it. None of them can change
//! anything: reading goes through the grant and the scope, and a proposal is only data.

use std::path::{Path, PathBuf};

use kellu_common::text::strip_control_chars;
use kellu_security::Risk;
use serde_json::{Value, json};

use crate::args::{Args, object};
use crate::proposal::{Item, MAX_FILE_BYTES, Proposal};
use crate::spec::{Context, Tool, ToolError, ToolOutput, ToolSpec};

const MAX_LIST: u64 = 500;
const MAX_READ_LINES: u64 = 2000;
const MAX_MATCHES: u64 = 100;
const MAX_SEARCH_BYTES: u64 = 16 << 20;
const MAX_QUERY_CHARS: usize = 200;
const MAX_LINE_CHARS: usize = 200;

fn rel(text: &str) -> Result<PathBuf, ToolError> {
    let path = PathBuf::from(text);
    if crate::proposal::plain_relative(&path) {
        Ok(path)
    } else {
        Err(ToolError::Refused(
            "the path must be relative, inside the project, without `..` or `.`".into(),
        ))
    }
}

fn shown(path: &Path) -> String {
    strip_control_chars(&path.display().to_string()).replace('\n', "\\n")
}

/// `list_files`: the files this session may read.
#[derive(Debug)]
pub struct ListFiles(ToolSpec);

impl ListFiles {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "list_files",
            Risk::Read,
            "List the project files you may read, optionally only those under a directory prefix.",
            json!({"type":"object","properties":{
                "prefix":{"type":"string","description":"Only files under this relative directory"},
                "max":{"type":"integer","description":"Most files to list (default 200, at most 500)"}
            }}),
        ))
    }
}

impl Default for ListFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ListFiles {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["prefix", "max"])?;
        Ok(format!(
            "list files under {}",
            a.opt_str("prefix")?
                .map_or("the project".into(), |p| shown(Path::new(p)))
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["prefix", "max"])?;
        let prefix = a.opt_str("prefix")?.map(rel).transpose()?;
        let max = a.opt_u64("max")?.unwrap_or(200).clamp(1, MAX_LIST) as usize;
        let mut lines = Vec::new();
        let mut total = 0_usize;
        for file in ctx.grant.readable() {
            if prefix.as_ref().is_some_and(|p| !file.starts_with(p)) {
                continue;
            }
            total += 1;
            if lines.len() < max {
                lines.push(shown(file));
            }
        }
        let mut text = lines.join("\n");
        if total > lines.len() {
            text.push_str(&format!("\n... {} more file(s)", total - lines.len()));
        }
        if total == 0 {
            text = "no files".into();
        }
        Ok(ToolOutput::text(text))
    }
}

/// `read_file`: one file, optionally a range of lines.
#[derive(Debug)]
pub struct ReadFile(ToolSpec);

impl ReadFile {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "read_file",
            Risk::Read,
            "Read a project file as numbered lines. Use start_line and max_lines for a part.",
            json!({"type":"object","properties":{
                "path":{"type":"string"},
                "start_line":{"type":"integer","description":"First line, 1-based (default 1)"},
                "max_lines":{"type":"integer","description":"Most lines (default 400, at most 2000)"}
            },"required":["path"]}),
        ))
    }
}

impl Default for ReadFile {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ReadFile {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["path", "start_line", "max_lines"])?;
        Ok(format!("read {}", shown(Path::new(a.str("path")?))))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["path", "start_line", "max_lines"])?;
        let path = rel(a.str("path")?)?;
        let start = a.opt_u64("start_line")?.unwrap_or(1).max(1) as usize;
        let count = a
            .opt_u64("max_lines")?
            .unwrap_or(400)
            .clamp(1, MAX_READ_LINES) as usize;
        if !ctx.grant.can_read(&path, ctx.now) {
            return Err(ToolError::Refused(format!(
                "{} is not a file this session may read",
                shown(&path)
            )));
        }
        let bytes = ctx.scope.read_file(&path, MAX_FILE_BYTES)?;
        if bytes.contains(&0) {
            return Err(ToolError::Refused(format!(
                "{} is a binary file",
                shown(&path)
            )));
        }
        let text = String::from_utf8_lossy(&bytes);
        let total = text.lines().count();
        let mut out = String::new();
        for (index, line) in text.lines().enumerate().skip(start - 1).take(count) {
            out.push_str(&format!("{}: {}\n", index + 1, line));
        }
        if out.is_empty() {
            out = format!("(the file has {total} line(s); nothing at line {start})\n");
        } else if start - 1 + count < total {
            out.push_str(&format!(
                "... {} more line(s)\n",
                total - (start - 1 + count)
            ));
        }
        Ok(ToolOutput::text(out))
    }
}

/// `search_text`: a literal search across the readable files.
#[derive(Debug)]
pub struct SearchText(ToolSpec);

impl SearchText {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "search_text",
            Risk::Read,
            "Search the readable project files for a literal text (not a regular expression).",
            json!({"type":"object","properties":{
                "query":{"type":"string"},
                "prefix":{"type":"string","description":"Only files under this relative directory"},
                "ignore_case":{"type":"boolean"},
                "max_matches":{"type":"integer","description":"Default 50, at most 100"}
            },"required":["query"]}),
        ))
    }
}

impl Default for SearchText {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for SearchText {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["query", "prefix", "ignore_case", "max_matches"])?;
        let query: String = strip_control_chars(a.str("query")?)
            .chars()
            .take(60)
            .collect();
        Ok(format!("search for \"{query}\""))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["query", "prefix", "ignore_case", "max_matches"])?;
        let query = a.str("query")?;
        if query.chars().count() > MAX_QUERY_CHARS || query.contains('\n') {
            return Err(ToolError::Invalid(format!(
                "query: one line of at most {MAX_QUERY_CHARS} characters"
            )));
        }
        let prefix = a.opt_str("prefix")?.map(rel).transpose()?;
        let ignore_case = a.bool_or("ignore_case", false)?;
        let max = a
            .opt_u64("max_matches")?
            .unwrap_or(50)
            .clamp(1, MAX_MATCHES) as usize;
        let needle = if ignore_case {
            query.to_lowercase()
        } else {
            query.to_owned()
        };
        let (mut hits, mut budget, mut skipped) = (Vec::new(), MAX_SEARCH_BYTES, 0_usize);
        'files: for file in ctx.grant.readable() {
            if prefix.as_ref().is_some_and(|p| !file.starts_with(p)) {
                continue;
            }
            let Ok(bytes) = ctx.scope.read_file(file, MAX_FILE_BYTES.min(budget)) else {
                skipped += 1;
                continue;
            };
            budget = budget.saturating_sub(bytes.len() as u64);
            if bytes.contains(&0) {
                continue;
            }
            for (index, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
                let found = if ignore_case {
                    line.to_lowercase().contains(&needle)
                } else {
                    line.contains(&needle)
                };
                if found {
                    let line: String = strip_control_chars(line)
                        .chars()
                        .take(MAX_LINE_CHARS)
                        .collect();
                    hits.push(format!(
                        "{}:{}: {}",
                        shown(file),
                        index + 1,
                        line.trim_end()
                    ));
                    if hits.len() >= max {
                        break 'files;
                    }
                }
            }
            if budget == 0 {
                break;
            }
        }
        let mut text = if hits.is_empty() {
            "no matches".to_owned()
        } else {
            hits.join("\n")
        };
        if hits.len() >= max {
            text.push_str("\n... more matches may exist");
        }
        if skipped > 0 {
            text.push_str(&format!(
                "\n({skipped} file(s) too large or unreadable were skipped)"
            ));
        }
        Ok(ToolOutput::text(text))
    }
}

fn proposal_result(scope: &kellu_security::Scope, item: Item) -> Result<ToolOutput, ToolError> {
    let proposal = Proposal::new(scope, vec![item])?;
    let text = format!(
        "Proposal {} recorded with {} item(s). Nothing was changed: the user decides whether to apply it.",
        proposal.id(),
        proposal.items().len()
    );
    Ok(ToolOutput::with_proposal(text, proposal))
}

/// `propose_edit`: exact replacements in an existing file.
#[derive(Debug)]
pub struct ProposeEdit(ToolSpec);

impl ProposeEdit {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "propose_edit",
            Risk::Read,
            "Propose replacing exact text in an existing file. Each `old` must occur exactly once \
             in the file. This changes nothing: the user reviews and approves the proposal.",
            json!({"type":"object","properties":{
                "path":{"type":"string"},
                "edits":{"type":"array","items":{"type":"object","properties":{
                    "old":{"type":"string"},"new":{"type":"string"}},"required":["old","new"]}},
                "rationale":{"type":"string"}
            },"required":["path","edits"]}),
        ))
    }
}

impl Default for ProposeEdit {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ProposeEdit {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["path", "edits", "rationale"])?;
        Ok(format!(
            "propose an edit of {}",
            shown(Path::new(a.str("path")?))
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["path", "edits", "rationale"])?;
        let path = rel(a.str("path")?)?;
        let mut edits = Vec::new();
        for edit in a.array("edits", crate::proposal::MAX_HUNKS)? {
            let e = object(edit, &["old", "new"])?;
            // `old` and `new` may be empty strings, so read them without the non-empty rule.
            let old = e
                .opt_str("old")?
                .ok_or_else(|| ToolError::Invalid("edit: `old` is required".into()))?;
            let new = e
                .opt_str("new")?
                .ok_or_else(|| ToolError::Invalid("edit: `new` is required".into()))?;
            edits.push((old.to_owned(), new.to_owned()));
        }
        let item = Item::modify(
            ctx.scope,
            ctx.grant,
            ctx.now,
            &path,
            &edits,
            a.opt_str("rationale")?.unwrap_or(""),
        )?;
        proposal_result(ctx.scope, item)
    }
}

/// `propose_create`: a new file.
#[derive(Debug)]
pub struct ProposeCreate(ToolSpec);

impl ProposeCreate {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "propose_create",
            Risk::Read,
            "Propose creating a new file in an existing directory. It must not exist yet. This \
             changes nothing: the user reviews and approves the proposal.",
            json!({"type":"object","properties":{
                "path":{"type":"string"},"content":{"type":"string"},"rationale":{"type":"string"}
            },"required":["path","content"]}),
        ))
    }
}

impl Default for ProposeCreate {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ProposeCreate {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["path", "content", "rationale"])?;
        Ok(format!(
            "propose creating {}",
            shown(Path::new(a.str("path")?))
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["path", "content", "rationale"])?;
        let path = rel(a.str("path")?)?;
        let content = a
            .opt_str("content")?
            .ok_or_else(|| ToolError::Invalid("content: required".into()))?;
        let item = Item::create(
            ctx.scope,
            &path,
            content,
            a.opt_str("rationale")?.unwrap_or(""),
        )?;
        proposal_result(ctx.scope, item)
    }
}

/// `propose_delete`: removing a file.
#[derive(Debug)]
pub struct ProposeDelete(ToolSpec);

impl ProposeDelete {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "propose_delete",
            Risk::Read,
            "Propose deleting a file. This changes nothing: the user must type the path to \
             confirm, and the file is backed up first.",
            json!({"type":"object","properties":{
                "path":{"type":"string"},"rationale":{"type":"string"}
            },"required":["path"]}),
        ))
    }
}

impl Default for ProposeDelete {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ProposeDelete {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["path", "rationale"])?;
        Ok(format!(
            "propose deleting {}",
            shown(Path::new(a.str("path")?))
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["path", "rationale"])?;
        let path = rel(a.str("path")?)?;
        let item = Item::delete(
            ctx.scope,
            ctx.grant,
            ctx.now,
            &path,
            a.opt_str("rationale")?.unwrap_or(""),
        )?;
        proposal_result(ctx.scope, item)
    }
}
