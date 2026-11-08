//! Shared fixtures for the integration tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kellu_security::{AuditLog, DenyGlobs, Grant, Scope};
use kellu_tools::Session;

pub const TTL: Duration = Duration::from_secs(600);

pub struct Project {
    pub _tmp: tempfile::TempDir,
    pub home: PathBuf,
    pub root: PathBuf,
    pub outside: PathBuf,
    pub state: PathBuf,
    pub scope: Scope,
    pub audit: AuditLog,
}

/// A project with a few files, a `.git` marker directory and a private state directory.
pub fn project() -> Project {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let home = base.join("home");
    let root = home.join("project");
    let outside = base.join("outside");
    let state = base.join("state");
    for dir in [&root.join(".git"), &root.join("src"), &outside, &state] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub fn one() -> u8 {\n    1\n}\n\npub fn two() -> u8 {\n    2\n}\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "# demo\n").unwrap();
    fs::write(root.join(".env"), "TOKEN=do-not-read\n").unwrap();
    fs::write(outside.join("secret.txt"), "outside").unwrap();
    let scope = Scope::open(&root, &home, DenyGlobs::new(), false).unwrap();
    let audit = AuditLog::new(state.join("audit.log"));
    Project {
        _tmp: tmp,
        home,
        root,
        outside,
        state,
        scope,
        audit,
    }
}

impl Project {
    pub fn grant(&self, files: &[&str], now: Instant) -> Grant {
        Grant::read_only(&self.scope, files.iter().map(PathBuf::from), now, TTL)
    }

    pub fn session<'a>(&'a self, now: Instant) -> Session<'a> {
        Session {
            scope: &self.scope,
            audit: &self.audit,
            origin: "test",
            now,
        }
    }

    pub fn backups(&self) -> PathBuf {
        self.state.join("backups")
    }

    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative)).unwrap()
    }

    pub fn audit_events(&self) -> Vec<String> {
        let text = fs::read_to_string(self.state.join("audit.log")).unwrap_or_default();
        text.lines().map(str::to_owned).collect()
    }
}

pub fn is_root() -> bool {
    rustix::process::getuid().is_root()
}

pub fn no_temp_files_left(dir: &Path) -> bool {
    fs::read_dir(dir).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".kellu-tmp-")
    })
}

// ------------------------------------------------------------------ tool-call harness ----

use std::cell::RefCell;

use kellu_sandbox::{Limits, Sandbox};
use kellu_security::PatternRedactor;
use kellu_tools::host::HostTools;
use kellu_tools::{
    Approver, CallResult, ConfirmRequest, Confirmation, Context, Registry, default_registry,
};
use serde_json::Value;

/// Answers every question with a fixed reply and remembers what it was asked.
pub struct Scripted {
    pub reply: Confirmation,
    pub asked: RefCell<Vec<String>>,
}

impl Scripted {
    pub fn new(reply: Confirmation) -> Self {
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

pub struct Harness {
    pub p: Project,
    pub redactor: PatternRedactor,
    pub host: HostTools,
    pub registry: Registry,
}

impl Harness {
    pub fn new(host: HostTools) -> Self {
        let p = project();
        let registry = default_registry(AuditLog::new(p.state.join("audit.log"))).unwrap();
        Self {
            p,
            redactor: PatternRedactor::new(),
            host,
            registry,
        }
    }

    pub fn call(
        &self,
        grant: &Grant,
        now: Instant,
        sandbox: Option<&Sandbox>,
        approver: &dyn Approver,
        name: &str,
        args: Value,
    ) -> CallResult {
        let ctx = Context {
            scope: &self.p.scope,
            grant,
            now,
            redactor: &self.redactor,
            sandbox,
            staging_base: &self.p.state.join("stage"),
            limits: Limits::default(),
            host: &self.host,
            origin: "test",
        };
        self.registry.call(name, &args, &ctx, approver)
    }
}
