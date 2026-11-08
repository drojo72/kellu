//! The registry: every tool has a risk, and the requirement for it is enforced before it runs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use common::{Project, TTL, project};
use kellu_sandbox::Limits;
use kellu_security::{AuditLog, PatternRedactor, Requirement, Risk};
use kellu_tools::host::HostTools;
use kellu_tools::{
    Approver, ConfirmRequest, Confirmation, Context, Registry, Tool, ToolError, ToolOutput,
    ToolSpec, default_registry,
};
use serde_json::{Value, json};

/// Answers every question with a fixed reply and remembers what it was asked.
struct Scripted {
    reply: Confirmation,
    asked: RefCell<Vec<String>>,
}

impl Scripted {
    fn new(reply: Confirmation) -> Self {
        Self {
            reply,
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl Approver for Scripted {
    fn confirm(&self, request: &ConfirmRequest<'_>) -> Confirmation {
        self.asked.borrow_mut().push(format!(
            "{}|{:?}|{}",
            request.tool, request.requirement, request.summary
        ));
        self.reply.clone()
    }
}

struct Dummy {
    spec: ToolSpec,
    runs: Arc<AtomicUsize>,
    output: Result<String, ToolError>,
}

impl Tool for Dummy {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn summary(&self, _args: &Value) -> Result<String, ToolError> {
        Ok("do the thing\u{1b}[31m\nnow".into())
    }

    fn run(&self, _ctx: &Context<'_>, _args: &Value) -> Result<ToolOutput, ToolError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.output.clone().map(ToolOutput::text)
    }
}

fn dummy(
    name: &'static str,
    risk: Risk,
    output: Result<String, ToolError>,
) -> (Box<dyn Tool>, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    let tool = Dummy {
        spec: ToolSpec::new(
            name,
            risk,
            "a test tool",
            json!({"type":"object","properties":{}}),
        ),
        runs: runs.clone(),
        output,
    };
    (Box::new(tool), runs)
}

struct Env {
    p: Project,
    redactor: PatternRedactor,
    host: HostTools,
}

fn env() -> Env {
    Env {
        p: project(),
        redactor: PatternRedactor::new(),
        host: HostTools::none(),
    }
}

impl Env {
    fn ctx<'a>(&'a self, grant: &'a kellu_security::Grant, now: Instant) -> Context<'a> {
        Context {
            scope: &self.p.scope,
            grant,
            now,
            redactor: &self.redactor,
            sandbox: None,
            staging_base: &self.p.state,
            limits: Limits::default(),
            host: &self.host,
            origin: "test",
        }
    }
}

#[test]
fn every_shipped_tool_has_the_risk_pinned_here() {
    let registry = default_registry(AuditLog::new("/nonexistent/audit".into())).unwrap();
    let got: BTreeMap<&str, Risk> = registry.specs().map(|s| (s.name(), s.risk())).collect();
    let expected = BTreeMap::from([
        ("list_files", Risk::Read),
        ("read_file", Risk::Read),
        ("search_text", Risk::Read),
        ("propose_edit", Risk::Read),
        ("propose_create", Risk::Read),
        ("propose_delete", Risk::Read),
        ("run_command", Risk::Read),
        ("git_status", Risk::Read),
        ("git_diff", Risk::Read),
        ("git_log", Risk::Read),
        ("git_commit", Risk::Write),
        ("git_push", Risk::Network),
        ("gh_pr_create", Risk::Network),
    ]);
    assert_eq!(
        got, expected,
        "a tool was added, removed or its risk changed: update this table on purpose"
    );
    for spec in registry.specs() {
        assert_eq!(spec.parameters()["type"], "object", "{}", spec.name());
        assert!(!spec.description().is_empty());
    }
}

#[test]
fn read_tools_run_without_asking_and_everything_else_asks_first() {
    let e = env();
    let now = Instant::now();
    let grant = e.p.grant(&[], now);
    let ctx = e.ctx(&grant, now);
    let mut registry = Registry::new(AuditLog::new(e.p.state.join("audit.log")));
    let (read, read_runs) = dummy("reader", Risk::Read, Ok("ok".into()));
    let (write, write_runs) = dummy("writer", Risk::Write, Ok("ok".into()));
    registry.register(read).unwrap();
    registry.register(write).unwrap();

    let no = Scripted::new(Confirmation::Denied);
    let result = registry.call("reader", &json!({}), &ctx, &no);
    assert!(!result.is_error && read_runs.load(Ordering::SeqCst) == 1);
    assert!(
        no.asked.borrow().is_empty(),
        "a read must not interrupt the user"
    );

    let result = registry.call("writer", &json!({}), &ctx, &no);
    assert!(
        result.is_error && write_runs.load(Ordering::SeqCst) == 0,
        "declined means it never ran"
    );
    assert_eq!(no.asked.borrow().len(), 1);

    let yes = Scripted::new(Confirmation::Confirmed);
    let result = registry.call("writer", &json!({}), &ctx, &yes);
    assert!(!result.is_error && write_runs.load(Ordering::SeqCst) == 1);
    let shown = yes.asked.borrow()[0].clone();
    assert!(
        !shown.contains('\u{1b}') && !shown.contains('\n'),
        "the prompt text must be clean: {shown:?}"
    );
    let log = e.p.audit_events().join("\n");
    assert!(log.contains("rejected") && log.contains("approved"));
}

#[test]
fn a_destructive_tool_needs_its_name_typed() {
    let e = env();
    let now = Instant::now();
    let grant = e.p.grant(&[], now);
    let ctx = e.ctx(&grant, now);
    let mut registry = Registry::new(AuditLog::new(e.p.state.join("audit.log")));
    let (tool, runs) = dummy("wipe", Risk::Destructive, Ok("done".into()));
    registry.register(tool).unwrap();
    for weak in [
        Confirmation::Confirmed,
        Confirmation::Typed("wipe ".into()),
        Confirmation::Typed("WIPE".into()),
        Confirmation::Denied,
    ] {
        let result = registry.call("wipe", &json!({}), &ctx, &Scripted::new(weak));
        assert!(result.is_error);
    }
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    let result = registry.call(
        "wipe",
        &json!({}),
        &ctx,
        &Scripted::new(Confirmation::Typed("wipe".into())),
    );
    assert!(!result.is_error);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[test]
fn a_network_tool_is_denied_without_network_permission_and_asks_with_it() {
    let e = env();
    let now = Instant::now();
    let mut grant = e.p.grant(&[], now);
    let mut registry = Registry::new(AuditLog::new(e.p.state.join("audit.log")));
    let (tool, runs) = dummy("fetch", Risk::Network, Ok("body".into()));
    registry.register(tool).unwrap();
    let always = Scripted::new(Confirmation::Confirmed);
    {
        let ctx = e.ctx(&grant, now);
        let result = registry.call("fetch", &json!({}), &ctx, &always);
        assert!(result.is_error && result.text.contains("network"));
        assert!(
            always.asked.borrow().is_empty(),
            "a denied call must not even ask"
        );
    }
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    grant.allow_network();
    let ctx = e.ctx(&grant, now);
    let result = registry.call("fetch", &json!({}), &ctx, &always);
    assert!(!result.is_error);
    assert_eq!(always.asked.borrow().len(), 1);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[test]
fn the_requirement_table_is_what_the_registry_relies_on() {
    assert_eq!(
        kellu_security::requirement(Risk::Read, false),
        Requirement::Automatic
    );
    assert_eq!(
        kellu_security::requirement(Risk::Write, false),
        Requirement::Confirm
    );
    assert_eq!(
        kellu_security::requirement(Risk::Network, false),
        Requirement::Denied
    );
    assert_eq!(
        kellu_security::requirement(Risk::Network, true),
        Requirement::Confirm
    );
    assert_eq!(
        kellu_security::requirement(Risk::Destructive, true),
        Requirement::TypedConfirm
    );
}

#[test]
fn nothing_runs_when_the_decision_cannot_be_recorded_or_the_grant_expired() {
    let e = env();
    let now = Instant::now();
    let grant = e.p.grant(&[], now);
    let ctx = e.ctx(&grant, now);
    let broken = Registry::new(AuditLog::new(e.p.root.join("src/main.rs/audit.log")));
    let mut broken = broken;
    let (tool, runs) = dummy("writer", Risk::Write, Ok("ok".into()));
    broken.register(tool).unwrap();
    let result = broken.call(
        "writer",
        &json!({}),
        &ctx,
        &Scripted::new(Confirmation::Confirmed),
    );
    assert!(result.is_error && result.text.contains("audit"));
    assert_eq!(runs.load(Ordering::SeqCst), 0);

    let mut ok = Registry::new(AuditLog::new(e.p.state.join("audit.log")));
    let (tool, runs) = dummy("reader", Risk::Read, Ok("ok".into()));
    ok.register(tool).unwrap();
    let expired = e.ctx(&grant, now + TTL);
    assert!(
        ok.call(
            "reader",
            &json!({}),
            &expired,
            &Scripted::new(Confirmation::Denied)
        )
        .is_error
    );
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    assert!(
        ok.call(
            "missing",
            &json!({}),
            &ctx,
            &Scripted::new(Confirmation::Denied)
        )
        .is_error
    );
}

#[test]
fn what_a_tool_returns_is_redacted_cleaned_and_bounded() {
    let e = env();
    let now = Instant::now();
    let grant = e.p.grant(&[], now);
    let ctx = e.ctx(&grant, now);
    let mut registry = Registry::new(AuditLog::new(e.p.state.join("audit.log")));
    let leak = "api_key = \"sk-proj-abcdefghijklmnopqrstuvwxyz0123456789\"\n\u{1b}[2Jline\r\n";
    let (a, _) = dummy("leaky", Risk::Read, Ok(leak.to_owned()));
    let (b, _) = dummy("huge", Risk::Read, Ok("x".repeat(200_000)));
    let (c, _) = dummy(
        "failing",
        Risk::Read,
        Err(ToolError::Failed(format!("boom {leak}"))),
    );
    for tool in [a, b, c] {
        registry.register(tool).unwrap();
    }
    let none = Scripted::new(Confirmation::Denied);
    let leaky = registry.call("leaky", &json!({}), &ctx, &none);
    assert!(!leaky.text.contains("sk-proj-abcdef"), "{}", leaky.text);
    assert!(!leaky.text.contains('\u{1b}') && !leaky.text.contains('\r'));
    let huge = registry.call("huge", &json!({}), &ctx, &none);
    assert!(huge.text.chars().count() < 61_000 && huge.text.contains("truncated"));
    let failing = registry.call("failing", &json!({}), &ctx, &none);
    assert!(failing.is_error && !failing.text.contains("sk-proj-abcdef"));
}

#[test]
fn a_tool_without_a_valid_name_or_schema_or_with_a_taken_name_is_not_registered() {
    let mut registry = Registry::new(AuditLog::new("/nonexistent".into()));
    for name in ["", "Bad", "with-dash", "has space", "a.b"] {
        let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
        let (tool, _) = dummy(leaked, Risk::Read, Ok(String::new()));
        assert!(registry.register(tool).is_err(), "{name:?}");
    }
    let spec = ToolSpec::new("noschema", Risk::Read, "d", json!({"type":"string"}));
    struct Bad(ToolSpec);
    impl Tool for Bad {
        fn spec(&self) -> &ToolSpec {
            &self.0
        }
        fn summary(&self, _: &Value) -> Result<String, ToolError> {
            Ok(String::new())
        }
        fn run(&self, _: &Context<'_>, _: &Value) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text(""))
        }
    }
    assert!(registry.register(Box::new(Bad(spec))).is_err());
    let (one, _) = dummy("same", Risk::Read, Ok(String::new()));
    let (two, _) = dummy("same", Risk::Read, Ok(String::new()));
    registry.register(one).unwrap();
    assert!(registry.register(two).is_err());
}

// ---------------------------------------------------------------------------- file tools ----

fn call(
    e: &Env,
    grant: &kellu_security::Grant,
    now: Instant,
    name: &str,
    args: Value,
) -> kellu_tools::CallResult {
    let registry = default_registry(AuditLog::new(e.p.state.join("audit.log"))).unwrap();
    let ctx = e.ctx(grant, now);
    registry.call(name, &args, &ctx, &Scripted::new(Confirmation::Denied))
}

#[test]
fn listing_and_reading_follow_the_grant_and_never_reach_denied_or_outside_files() {
    let e = env();
    let now = Instant::now();
    let grant = e.p.grant(&["src/main.rs", "src/lib.rs"], now);
    let listed = call(&e, &grant, now, "list_files", json!({}));
    assert!(
        listed.text.contains("src/main.rs")
            && !listed.text.contains(".env")
            && !listed.text.contains("README")
    );
    let under = call(
        &e,
        &grant,
        now,
        "list_files",
        json!({"prefix": "src/lib.rs"}),
    );
    assert_eq!(under.text.trim(), "src/lib.rs");

    let read = call(&e, &grant, now, "read_file", json!({"path": "src/main.rs"}));
    assert!(read.text.starts_with("1: fn main() {"), "{}", read.text);
    let part = call(
        &e,
        &grant,
        now,
        "read_file",
        json!({"path": "src/lib.rs", "start_line": 5, "max_lines": 1}),
    );
    assert!(part.text.starts_with("5: pub fn two"), "{}", part.text);
    for bad in [
        ".env",
        "README.md",
        "../outside/secret.txt",
        "/etc/passwd",
        ".git/config",
        "src",
        "missing.rs",
    ] {
        let result = call(&e, &grant, now, "read_file", json!({"path": bad}));
        assert!(result.is_error, "{bad} must be refused");
        assert!(!result.text.contains("do-not-read") && !result.text.contains("outside"));
    }
    assert!(
        call(
            &e,
            &grant,
            now,
            "read_file",
            json!({"path": "src/main.rs", "extra": 1})
        )
        .is_error
    );
    assert!(call(&e, &grant, now, "read_file", json!({})).is_error);
}

#[test]
fn a_symlink_inside_the_project_cannot_be_read_through() {
    let e = env();
    std::os::unix::fs::symlink(e.p.outside.join("secret.txt"), e.p.root.join("link.txt")).unwrap();
    let now = Instant::now();
    let grant = e.p.grant(&["link.txt"], now);
    let result = call(&e, &grant, now, "read_file", json!({"path": "link.txt"}));
    assert!(
        result.is_error && !result.text.contains("outside"),
        "{}",
        result.text
    );
}

#[test]
fn file_contents_are_redacted_before_the_model_sees_them() {
    let e = env();
    fs::write(
        e.p.root.join("config.rs"),
        "let api_key = \"sk-proj-abcdefghijklmnopqrstuvwxyz0123456789\";\nlet ok = 1;\n",
    )
    .unwrap();
    let now = Instant::now();
    let grant = e.p.grant(&["config.rs"], now);
    let read = call(&e, &grant, now, "read_file", json!({"path": "config.rs"}));
    assert!(
        !read.text.contains("sk-proj-abcdef") && read.text.contains("let ok = 1"),
        "{}",
        read.text
    );
    let found = call(&e, &grant, now, "search_text", json!({"query": "api_key"}));
    assert!(!found.text.contains("sk-proj-abcdef"), "{}", found.text);
}

#[test]
fn search_is_literal_bounded_and_skips_binary_files() {
    let e = env();
    fs::write(e.p.root.join("bin.dat"), b"needle\0binary").unwrap();
    let now = Instant::now();
    let grant = e.p.grant(&["src/main.rs", "src/lib.rs", "bin.dat"], now);
    let hit = call(
        &e,
        &grant,
        now,
        "search_text",
        json!({"query": "println!(\"hello\")"}),
    );
    assert_eq!(hit.text.trim(), "src/main.rs:2:     println!(\"hello\");");
    assert!(
        call(&e, &grant, now, "search_text", json!({"query": "needle"}))
            .text
            .contains("no matches")
    );
    assert!(
        call(
            &e,
            &grant,
            now,
            "search_text",
            json!({"query": "PUB FN", "ignore_case": true})
        )
        .text
        .contains("src/lib.rs:1")
    );
    assert!(
        call(&e, &grant, now, "search_text", json!({"query": ".*"}))
            .text
            .contains("no matches"),
        "not a regex"
    );
    assert!(call(&e, &grant, now, "search_text", json!({"query": "a\nb"})).is_error);
    assert!(
        call(
            &e,
            &grant,
            now,
            "search_text",
            json!({"query": "x".repeat(201)})
        )
        .is_error
    );
    let capped = call(
        &e,
        &grant,
        now,
        "search_text",
        json!({"query": "fn", "max_matches": 1}),
    );
    assert!(capped.text.lines().any(|l| l.contains(":")));
    assert!(capped.text.contains("more matches may exist"));
}

#[test]
fn propose_tools_return_proposals_log_them_and_change_nothing() {
    let e = env();
    let now = Instant::now();
    let grant = e.p.grant(&["src/main.rs"], now);
    let before = e.p.read("src/main.rs");
    let edit = call(
        &e,
        &grant,
        now,
        "propose_edit",
        json!({"path": "src/main.rs", "edits": [{"old": "hello", "new": "hola"}], "rationale": "greet"}),
    );
    assert!(!edit.is_error, "{}", edit.text);
    let proposal = edit.proposal.unwrap();
    assert_eq!(proposal.items().len(), 1);
    assert!(edit.text.contains("Nothing was changed"));
    let create = call(
        &e,
        &grant,
        now,
        "propose_create",
        json!({"path": "src/new.rs", "content": "x"}),
    );
    assert!(create.proposal.is_some());
    let delete = call(
        &e,
        &grant,
        now,
        "propose_delete",
        json!({"path": "src/main.rs"}),
    );
    assert!(delete.proposal.is_some());
    assert_eq!(e.p.read("src/main.rs"), before);
    assert!(!e.p.root.join("src/new.rs").exists());
    assert!(e.p.audit_events().join("\n").contains("proposed"));

    let bad = call(
        &e,
        &grant,
        now,
        "propose_edit",
        json!({"path": "src/main.rs", "edits": [{"old": "hello", "new": "x", "extra": true}]}),
    );
    assert!(bad.is_error && bad.proposal.is_none());
    for args in [
        json!({"path": ".env", "content": "x"}),
        json!({"path": "../x", "content": "x"}),
        json!({"path": "src/main.rs", "content": "x"}),
    ] {
        assert!(call(&e, &grant, now, "propose_create", args).is_error);
    }
    assert!(
        call(
            &e,
            &grant,
            now,
            "propose_edit",
            json!({"path": ".env", "edits": [{"old": "TOKEN", "new": "X"}]})
        )
        .is_error
    );
}
