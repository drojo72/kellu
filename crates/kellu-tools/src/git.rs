//! Git and `gh`, on the host.
//!
//! Reading (`git_status`, `git_diff`, `git_log`) runs with an environment that inherits nothing,
//! ignores every configuration file outside the repository and switches off the ways a
//! repository's own configuration can make git run a program (filesystem monitor, pager, hooks,
//! external diff and text conversion, submodules). Files the scope denies are filtered out of
//! the output, because git itself knows nothing about the deny list.
//!
//! Writing needs the user's own git configuration (name, e-mail, credentials), so each write is a
//! fixed command shape with validated arguments, shown exactly before it runs:
//! `git_commit` (risk write), `git_push` and `gh_pr_create` (risk network, which the grant must
//! allow). There is no tool that forces, deletes or rewrites anything. Git hooks are disabled for
//! every call and the confirmation says so.

use std::ffi::OsString;
use std::path::Path;

use kellu_common::text::strip_control_chars;
use kellu_security::Risk;
use serde_json::{Value, json};

use crate::args::Args;
use crate::host::{self, clean, describe, git_prefix, read_env, write_env};
use crate::proposal::{TEMP_PREFIX, plain_relative};
use crate::spec::{Context, Tool, ToolError, ToolOutput, ToolSpec};

const MAX_PATHS: usize = 100;
const MAX_DIFF_FILES: usize = 200;
const MAX_SHOWN_CHARS: usize = 60_000;
const MAX_MESSAGE_BYTES: usize = 4000;
const MAX_LOG: u64 = 50;

fn os(parts: &[&str]) -> Vec<OsString> {
    parts.iter().map(OsString::from).collect()
}

/// Whether a repository-local configuration key can make git run a program that the command line
/// cannot switch off.
///
/// Neutralised with flags, so **not** refused: `core.fsmonitor`, `core.pager`, `core.hooksPath`
/// (`-c`), and `diff.external` / `diff.<name>.textconv` / `diff.<name>.command` (`--no-ext-diff`
/// and `--no-textconv`). Never used by these tools, so not refused: editors and merge drivers.
///
/// What is left is chosen by name inside the repository, or runs at push or sign time:
/// a `filter.<name>.clean` command runs when git compares or stages a file that the repository's
/// attributes assign to it, an include pulls in another file, and `core.sshCommand`,
/// `core.askPass` and `gpg.program` run when pushing or signing. A repository that arrives as an
/// archive can carry its own `.git/config`, so these are refused instead.
fn dangerous_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let mut parts = key.split('.');
    let (Some(section), Some(last)) = (parts.next(), key.rsplit('.').next()) else {
        return false;
    };
    match section {
        "filter" => matches!(last, "clean" | "smudge" | "process"),
        "include" | "includeif" => last == "path",
        "core" => matches!(last, "sshcommand" | "askpass"),
        "gpg" => last == "program",
        _ => false,
    }
}

/// Refuse to run git in a repository whose own configuration can run a program.
///
/// Reading the configuration (`git config --local --list`) executes nothing.
fn ensure_safe_repository(ctx: &Context<'_>) -> Result<(), ToolError> {
    let mut args = git_prefix();
    args.extend(os(&["config", "--local", "--list", "-z"]));
    let out = host::run(ctx.host.git()?, &args, &read_env(), ctx.scope.root(), None)?;
    if !out.success() {
        return Err(ToolError::Failed(format!(
            "git: this is not a usable git repository ({})",
            clean(&out.stderr, 200)
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let bad: Vec<String> = text
        .split('\0')
        .filter_map(|entry| entry.split('\n').next())
        .filter(|key| dangerous_key(key))
        .map(|key| shown(key).chars().take(80).collect())
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(ToolError::Refused(format!(
            "the repository's own git configuration sets {}, which can make git run a program; \
             remove it, or run git yourself",
            bad.join(", ")
        )))
    }
}

/// Run a read-only git command and return its stdout.
fn git_read(ctx: &Context<'_>, subcommand: &[OsString]) -> Result<Vec<u8>, ToolError> {
    ensure_safe_repository(ctx)?;
    let mut args = git_prefix();
    args.extend(subcommand.iter().cloned());
    let out = host::run(ctx.host.git()?, &args, &read_env(), ctx.scope.root(), None)?;
    if out.success() {
        Ok(out.stdout)
    } else {
        Err(ToolError::Failed(format!(
            "git {}: {}",
            describe(out.outcome),
            clean(&out.stderr, 500)
        )))
    }
}

/// A pathspec the model may name: relative, inside the project, not denied.
fn checked_path(ctx: &Context<'_>, text: &str) -> Result<OsString, ToolError> {
    let path = Path::new(text);
    if !plain_relative(path) {
        return Err(ToolError::Refused(
            "paths must be relative, inside the project, without `..` or `.`".into(),
        ));
    }
    if ctx.scope.is_denied(path) {
        return Err(ToolError::Refused(
            format!("{text} is denied by policy")
                .chars()
                .take(200)
                .collect(),
        ));
    }
    if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with(TEMP_PREFIX))
    {
        return Err(ToolError::Refused("that name is reserved".into()));
    }
    Ok(path.as_os_str().to_owned())
}

fn shown(text: &str) -> String {
    strip_control_chars(text).replace('\n', "\\n")
}

/// `git_status`.
#[derive(Debug)]
pub struct GitStatus(ToolSpec);

impl GitStatus {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "git_status",
            Risk::Read,
            "Show the branch and the changed files of the project's git repository.",
            json!({"type":"object","properties":{}}),
        ))
    }
}

impl Default for GitStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for GitStatus {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        Args::new(args, &[])?;
        Ok("git status".into())
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        Args::new(args, &[])?;
        let raw = git_read(
            ctx,
            &os(&[
                "status",
                "--porcelain=v1",
                "-z",
                "--branch",
                "--no-renames",
                "--untracked-files=normal",
                "--ignore-submodules=all",
            ]),
        )?;
        let text = String::from_utf8_lossy(&raw);
        let (mut lines, mut hidden) = (Vec::new(), 0_usize);
        for entry in text.split('\0').filter(|e| !e.is_empty()) {
            if let Some(branch) = entry.strip_prefix("## ") {
                lines.insert(0, format!("branch: {}", shown(branch)));
            } else if let Some((flags, path)) = entry.split_at_checked(3) {
                if ctx.scope.is_denied(Path::new(path)) {
                    hidden += 1;
                } else {
                    lines.push(format!("{} {}", &flags[..2], shown(path)));
                }
            }
        }
        if lines.len() <= 1 {
            lines.push("working tree clean".into());
        }
        if hidden > 0 {
            lines.push(format!(
                "({hidden} entr{} hidden by policy)",
                if hidden == 1 { "y" } else { "ies" }
            ));
        }
        Ok(ToolOutput::text(lines.join("\n")))
    }
}

/// `git_diff`.
#[derive(Debug)]
pub struct GitDiff(ToolSpec);

impl GitDiff {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "git_diff",
            Risk::Read,
            "Show the uncommitted changes (or the staged ones) of the git repository, optionally for some paths.",
            json!({"type":"object","properties":{
                "staged":{"type":"boolean"},
                "paths":{"type":"array","items":{"type":"string"}}
            }}),
        ))
    }
}

impl Default for GitDiff {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for GitDiff {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["staged", "paths"])?;
        Ok(format!(
            "git diff{}",
            if a.bool_or("staged", false)? {
                " --cached"
            } else {
                ""
            }
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["staged", "paths"])?;
        let staged = a.bool_or("staged", false)?;
        let mut specs: Vec<OsString> = Vec::new();
        if a.has("paths") {
            for text in a.str_list("paths", MAX_PATHS)? {
                specs.push(checked_path(ctx, text)?);
            }
        }
        // First learn which files differ, drop the denied ones, then diff exactly those.
        let mut names = os(&[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--ignore-submodules=all",
            "--no-ext-diff",
            "--no-textconv",
        ]);
        if staged {
            names.push("--cached".into());
        }
        names.push("--".into());
        names.extend(specs.iter().cloned());
        let listed = git_read(ctx, &names)?;
        let all: Vec<String> = String::from_utf8_lossy(&listed)
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect();
        let allowed: Vec<&String> = all
            .iter()
            .filter(|p| plain_relative(Path::new(p)) && !ctx.scope.is_denied(Path::new(p)))
            .collect();
        let hidden = all.len() - allowed.len();
        if allowed.is_empty() {
            let note = if hidden > 0 {
                format!(" ({hidden} file(s) hidden by policy)")
            } else {
                String::new()
            };
            return Ok(ToolOutput::text(format!("no changes{note}")));
        }
        let truncated_files = allowed.len() > MAX_DIFF_FILES;
        let mut diff = os(&[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--ignore-submodules=all",
        ]);
        if staged {
            diff.push("--cached".into());
        }
        diff.push("--".into());
        diff.extend(
            allowed
                .iter()
                .take(MAX_DIFF_FILES)
                .map(|p| OsString::from(p.as_str())),
        );
        let body = git_read(ctx, &diff)?;
        let mut text = clean(&body, MAX_SHOWN_CHARS);
        if truncated_files {
            text.push_str(&format!(
                "\n... only the first {MAX_DIFF_FILES} of {} files are shown",
                allowed.len()
            ));
        }
        if hidden > 0 {
            text.push_str(&format!("\n({hidden} file(s) hidden by policy)"));
        }
        Ok(ToolOutput::text(text))
    }
}

/// `git_log`.
#[derive(Debug)]
pub struct GitLog(ToolSpec);

impl GitLog {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "git_log",
            Risk::Read,
            "Show recent commits (hash, date, author, subject), optionally for one path.",
            json!({"type":"object","properties":{
                "max":{"type":"integer","description":"Default 20, at most 50"},
                "path":{"type":"string"}
            }}),
        ))
    }
}

impl Default for GitLog {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for GitLog {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        Args::new(args, &["max", "path"])?;
        Ok("git log".into())
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["max", "path"])?;
        let max = a.opt_u64("max")?.unwrap_or(20).clamp(1, MAX_LOG);
        let mut command = os(&[
            "-c",
            "log.showSignature=false",
            "log",
            "--no-color",
            "--no-decorate",
            "--no-notes",
            "--date=short",
            "--pretty=format:%h %ad %an %s",
        ]);
        command.push(format!("-n{max}").into());
        if let Some(path) = a.opt_str("path")? {
            command.push("--".into());
            command.push(checked_path(ctx, path)?);
        }
        let out = git_read(ctx, &command)?;
        let text = clean(&out, MAX_SHOWN_CHARS);
        Ok(ToolOutput::text(if text.is_empty() {
            "no commits".into()
        } else {
            text
        }))
    }
}

/// A branch name that cannot be taken for an option, a refspec trick or a path.
fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with(['-', '/', '.'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && !name.ends_with(".lock")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
}

fn run_write(
    ctx: &Context<'_>,
    program: &Path,
    args: &[OsString],
    stdin: Option<&[u8]>,
) -> Result<kellu_sandbox::Output, ToolError> {
    ensure_safe_repository(ctx)?;
    let env = write_env(std::env::vars_os());
    host::run(program, args, &env, ctx.scope.root(), stdin)
}

fn finish(label: &str, out: &kellu_sandbox::Output) -> Result<ToolOutput, ToolError> {
    let stdout = clean(&out.stdout, 20_000);
    let stderr = clean(&out.stderr, 20_000);
    let body = [stdout, stderr]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    if out.success() {
        Ok(ToolOutput::text(format!(
            "{label}: {}\n{body}",
            describe(out.outcome)
        )))
    } else {
        Err(ToolError::Failed(format!(
            "{label}: {}\n{body}",
            describe(out.outcome)
        )))
    }
}

/// `git_commit`: stage the named paths and commit them.
#[derive(Debug)]
pub struct GitCommit(ToolSpec);

impl GitCommit {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "git_commit",
            Risk::Write,
            "Commit the named files with a message. Git hooks are not run. The user confirms the \
             exact files and message first.",
            json!({"type":"object","properties":{
                "message":{"type":"string"},
                "paths":{"type":"array","items":{"type":"string"}}
            },"required":["message","paths"]}),
        ))
    }

    fn parse<'a>(
        ctx: &Context<'_>,
        args: &'a Value,
    ) -> Result<(&'a str, Vec<OsString>), ToolError> {
        let a = Args::new(args, &["message", "paths"])?;
        let message = a.str("message")?;
        if message.trim().is_empty() || message.len() > MAX_MESSAGE_BYTES {
            return Err(ToolError::Invalid(format!(
                "message: not blank and at most {MAX_MESSAGE_BYTES} bytes"
            )));
        }
        let paths = a
            .str_list("paths", MAX_PATHS)?
            .into_iter()
            .map(|p| checked_path(ctx, p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((message, paths))
    }
}

impl Default for GitCommit {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for GitCommit {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["message", "paths"])?;
        let paths = a
            .str_list("paths", MAX_PATHS)?
            .iter()
            .map(|p| shown(p))
            .collect::<Vec<_>>()
            .join(" ");
        let subject: String = shown(a.str("message")?.lines().next().unwrap_or(""))
            .chars()
            .take(100)
            .collect();
        Ok(format!(
            "git add --all -- {paths}; git commit -F - -- {paths} (hooks disabled); subject: \"{subject}\""
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let (message, paths) = Self::parse(ctx, args)?;
        let git = ctx.host.git()?;
        let mut add = git_prefix();
        add.extend(os(&["add", "--all", "--"]));
        add.extend(paths.iter().cloned());
        finish("git add", &run_write(ctx, git, &add, None)?)?;
        let mut commit = git_prefix();
        commit.extend(os(&["commit", "--no-verify", "-F", "-", "--"]));
        commit.extend(paths);
        finish(
            "git commit",
            &run_write(ctx, git, &commit, Some(message.as_bytes()))?,
        )
    }
}

/// `git_push`: push the current commit to a branch of `origin`. Never forces.
#[derive(Debug)]
pub struct GitPush(ToolSpec);

impl GitPush {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "git_push",
            Risk::Network,
            "Push the current commit (HEAD) to a branch of the remote `origin`. It never forces. \
             Needs network permission and the user's confirmation.",
            json!({"type":"object","properties":{"branch":{"type":"string"}},"required":["branch"]}),
        ))
    }
}

impl Default for GitPush {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for GitPush {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["branch"])?;
        let branch = a.str("branch")?;
        if !valid_branch(branch) {
            return Err(ToolError::Invalid("branch: not a plain branch name".into()));
        }
        Ok(format!(
            "git push origin HEAD:refs/heads/{branch} (no force)"
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["branch"])?;
        let branch = a.str("branch")?;
        if !valid_branch(branch) {
            return Err(ToolError::Invalid("branch: not a plain branch name".into()));
        }
        let mut push = git_prefix();
        push.extend(os(&[
            "push",
            "--porcelain",
            "--no-recurse-submodules",
            "--no-verify",
            "origin",
            "--",
        ]));
        push.push(format!("HEAD:refs/heads/{branch}").into());
        finish("git push", &run_write(ctx, ctx.host.git()?, &push, None)?)
    }
}

/// `gh_pr_create`: open a pull request for the current branch.
#[derive(Debug)]
pub struct GhPrCreate(ToolSpec);

impl GhPrCreate {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "gh_pr_create",
            Risk::Network,
            "Open a pull request for the current branch with the GitHub CLI. Needs network \
             permission and the user's confirmation.",
            json!({"type":"object","properties":{
                "title":{"type":"string"},"body":{"type":"string"},
                "base":{"type":"string"},"draft":{"type":"boolean"}
            },"required":["title"]}),
        ))
    }

    fn parse(args: &Value) -> Result<(String, &str, Option<&str>, bool), ToolError> {
        let a = Args::new(args, &["title", "body", "base", "draft"])?;
        let title = a.str("title")?;
        if title.trim().is_empty() || title.len() > 256 || title.contains(['\n', '\r']) {
            return Err(ToolError::Invalid(
                "title: one non-blank line of at most 256 bytes".into(),
            ));
        }
        let body = a.opt_str("body")?.unwrap_or("");
        if body.len() > 20_000 {
            return Err(ToolError::Invalid("body: at most 20000 bytes".into()));
        }
        let base = a.opt_str("base")?;
        if base.is_some_and(|b| !valid_branch(b)) {
            return Err(ToolError::Invalid("base: not a plain branch name".into()));
        }
        Ok((
            strip_control_chars(title),
            body,
            base,
            a.bool_or("draft", false)?,
        ))
    }
}

impl Default for GhPrCreate {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for GhPrCreate {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let (title, body, base, draft) = Self::parse(args)?;
        Ok(format!(
            "gh pr create --title \"{title}\"{}{} (body: {} bytes)",
            base.map_or(String::new(), |b| format!(" --base {b}")),
            if draft { " --draft" } else { "" },
            body.len()
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let (title, body, base, draft) = Self::parse(args)?;
        let mut command: Vec<OsString> = vec![
            "pr".into(),
            "create".into(),
            format!("--title={title}").into(),
            "--body-file=-".into(),
        ];
        if let Some(base) = base {
            command.push(format!("--base={base}").into());
        }
        if draft {
            command.push("--draft".into());
        }
        finish(
            "gh pr create",
            &run_write(ctx, ctx.host.gh()?, &command, Some(body.as_bytes()))?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_that_can_run_a_program_are_recognised() {
        for bad in [
            "filter.evil.clean",
            "FILTER.evil.Smudge",
            "filter.a.b.process",
            "include.path",
            "includeIf.gitdir:/x/.path",
            "core.sshCommand",
            "core.askPass",
            "gpg.program",
            "gpg.ssh.program",
        ] {
            assert!(dangerous_key(bad), "{bad}");
        }
        for fine in [
            "user.name",
            "user.email",
            "remote.origin.url",
            "branch.main.remote",
            "core.bare",
            "core.filemode",
            "filter.evil.required",
            "diff.algorithm",
            "credential.helper",
            "commit.gpgsign",
            "",
            // neutralised on the command line
            "core.fsmonitor",
            "core.pager",
            "core.hooksPath",
            "diff.external",
            "diff.evil.textconv",
            "diff.evil.command",
            // never used by these tools
            "core.editor",
            "sequence.editor",
            "merge.x.driver",
        ] {
            assert!(!dangerous_key(fine), "{fine}");
        }
    }

    #[test]
    fn branch_names_that_could_act_as_options_or_refspec_tricks_are_refused() {
        for good in ["main", "feature/m5-tools", "release-1.2", "a_b"] {
            assert!(valid_branch(good), "{good}");
        }
        for bad in [
            "",
            "-f",
            "--force",
            "+main",
            "main:other",
            "a..b",
            "a//b",
            "/x",
            ".x",
            "x.",
            "x.lock",
            "x@{1}",
            "a b",
            "a;b",
            "a$(b)",
            "ñ",
            "a\nb",
            "refs/heads/../x",
            &"a".repeat(101),
        ] {
            assert!(!valid_branch(bad), "{bad:?}");
        }
    }
}
