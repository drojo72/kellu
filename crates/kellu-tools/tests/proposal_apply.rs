//! Proposals, approval and application: the only path that writes to the project.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;
use std::time::Instant;

use common::{Project, TTL, is_root, no_temp_files_left, project};
use kellu_security::{AuditLog, Grant};
use kellu_tools::{Approval, Choice, Item, Operation, Proposal, ToolError, apply, approve};

fn edit(old: &str, new: &str) -> (String, String) {
    (old.to_owned(), new.to_owned())
}

fn modify(
    p: &Project,
    grant: &Grant,
    now: Instant,
    path: &str,
    edits: &[(String, String)],
) -> Result<Item, ToolError> {
    Item::modify(&p.scope, grant, now, Path::new(path), edits, "test")
}

fn approve_all(proposal: &Proposal) -> Approval {
    let mut approval = Approval::default();
    for item in proposal.items() {
        approval.items.insert(item.id().to_owned(), Choice::Full);
        if item.operation() == Operation::Delete {
            approval
                .typed
                .insert(item.id().to_owned(), item.path().display().to_string());
        }
    }
    approval
}

#[test]
fn an_approved_edit_is_applied_keeps_the_mode_and_leaves_a_private_backup() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs", "src/lib.rs"], now);
    fs::set_permissions(
        p.root.join("src/main.rs"),
        fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    let item = modify(
        &p,
        &grant,
        now,
        "src/main.rs",
        &[edit("\"hello\"", "\"hola\"")],
    )
    .unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    assert_eq!(
        p.read("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
        "proposing writes nothing"
    );

    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    let report = apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap();

    assert_eq!(
        p.read("src/main.rs"),
        "fn main() {\n    println!(\"hola\");\n}\n"
    );
    assert_eq!(
        fs::metadata(p.root.join("src/main.rs")).unwrap().mode() & 0o7777,
        0o750
    );
    assert!(report.audit_complete);
    assert!(no_temp_files_left(&p.root.join("src")));
    let backup = report.backups.join("001-main.rs");
    assert_eq!(
        fs::read_to_string(&backup).unwrap(),
        "fn main() {\n    println!(\"hello\");\n}\n"
    );
    assert_eq!(fs::metadata(&backup).unwrap().mode() & 0o777, 0o600);
    assert_eq!(fs::metadata(&report.backups).unwrap().mode() & 0o777, 0o700);
    // proposed -> approved -> applied are all in a log whose hash chain verifies.
    let log = p.audit_events().join("\n");
    for event in ["approved", "applied"] {
        assert!(log.contains(event), "{event} missing from the audit log");
    }
    assert!(kellu_security::audit::verify(&p.state.join("audit.log")).unwrap() >= 2);
}

#[test]
fn only_the_approved_hunks_are_applied() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/lib.rs"], now);
    let item = modify(
        &p,
        &grant,
        now,
        "src/lib.rs",
        &[edit("    1\n", "    10\n"), edit("    2\n", "    20\n")],
    )
    .unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let mut approval = Approval::default();
    approval
        .items
        .insert("1".into(), Choice::Hunks(BTreeSet::from([2])));
    let session = p.session(now);
    let approved = approve(&proposal, &approval, &session, &mut grant).unwrap();
    apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap();
    assert_eq!(
        p.read("src/lib.rs"),
        "pub fn one() -> u8 {\n    1\n}\n\npub fn two() -> u8 {\n    20\n}\n"
    );
}

#[test]
fn a_file_that_changed_after_the_proposal_is_refused_and_nothing_else_is_applied() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs", "src/lib.rs"], now);
    let a = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let b = modify(
        &p,
        &grant,
        now,
        "src/lib.rs",
        &[edit("    1\n", "    11\n")],
    )
    .unwrap();
    let proposal = Proposal::new(&p.scope, vec![a, b]).unwrap();
    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    // The user edits the second file while looking at the proposal.
    fs::write(p.root.join("src/lib.rs"), "pub fn one() -> u8 { 1 }\n").unwrap();
    let error = apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap_err();
    assert!(
        error.to_string().contains("changed after the user saw it"),
        "{error}"
    );
    assert!(
        p.read("src/main.rs").contains("hello"),
        "the first item must not be applied either"
    );
    assert_eq!(p.read("src/lib.rs"), "pub fn one() -> u8 { 1 }\n");
}

#[test]
fn items_the_user_did_not_select_stay_untouched_and_are_logged_as_rejected() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs", "src/lib.rs"], now);
    let a = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let b = modify(
        &p,
        &grant,
        now,
        "src/lib.rs",
        &[edit("    1\n", "    11\n")],
    )
    .unwrap();
    let proposal = Proposal::new(&p.scope, vec![a, b]).unwrap();
    let mut approval = Approval::default();
    approval.items.insert("2".into(), Choice::Full);
    let session = p.session(now);
    let approved = approve(&proposal, &approval, &session, &mut grant).unwrap();
    apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap();
    assert!(p.read("src/main.rs").contains("hello"));
    assert!(p.read("src/lib.rs").contains("11"));
    assert!(p.audit_events().join("\n").contains("rejected"));
}

#[test]
fn an_empty_decision_rejects_everything_and_applying_it_is_an_error() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs"], now);
    let item = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let session = p.session(now);
    let approved = approve(&proposal, &Approval::default(), &session, &mut grant).unwrap();
    assert!(approved.is_empty());
    assert!(apply(&proposal, &approved, &session, &grant, &p.backups()).is_err());
    assert!(p.read("src/main.rs").contains("hello"));
}

#[test]
fn deleting_needs_the_path_typed_and_keeps_a_backup() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["README.md"], now);
    let item = Item::delete(&p.scope, &grant, now, Path::new("README.md"), "x").unwrap();
    assert_eq!(item.operation().risk(), kellu_security::Risk::Destructive);
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let session = p.session(now);

    let mut wrong = approve_all(&proposal);
    wrong.typed.insert("1".into(), "readme.md".into());
    assert!(approve(&proposal, &wrong, &session, &mut grant).is_err());
    let mut none = approve_all(&proposal);
    none.typed.clear();
    assert!(approve(&proposal, &none, &session, &mut grant).is_err());
    assert!(p.root.join("README.md").exists());

    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    let report = apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap();
    assert!(!p.root.join("README.md").exists());
    assert_eq!(
        fs::read_to_string(report.backups.join("001-README.md")).unwrap(),
        "# demo\n"
    );
}

#[test]
fn a_new_file_is_created_without_overwriting_and_not_over_one_that_appeared_later() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["README.md"], now);
    assert!(
        Item::create(&p.scope, Path::new("README.md"), "x", "x").is_err(),
        "exists already"
    );
    let item = Item::create(&p.scope, Path::new("src/new.rs"), "pub fn n() {}\n", "x").unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    fs::write(p.root.join("src/new.rs"), "someone else's\n").unwrap();
    let error = apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap_err();
    assert!(error.to_string().contains("now exists"), "{error}");
    assert_eq!(p.read("src/new.rs"), "someone else's\n");
    fs::remove_file(p.root.join("src/new.rs")).unwrap();
    apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap();
    assert_eq!(p.read("src/new.rs"), "pub fn n() {}\n");
}

#[test]
fn a_file_swapped_for_a_symlink_after_the_proposal_is_not_followed() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs"], now);
    let item = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    fs::remove_file(p.root.join("src/main.rs")).unwrap();
    symlink(p.outside.join("secret.txt"), p.root.join("src/main.rs")).unwrap();
    assert!(apply(&proposal, &approved, &session, &grant, &p.backups()).is_err());
    assert_eq!(
        fs::read_to_string(p.outside.join("secret.txt")).unwrap(),
        "outside"
    );
}

#[test]
fn a_directory_swapped_for_a_symlink_after_the_proposal_is_not_followed() {
    let p = project();
    let now = Instant::now();
    fs::write(p.outside.join("main.rs"), "outside file\n").unwrap();
    let mut grant = p.grant(&["src/main.rs"], now);
    let item = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    fs::rename(p.root.join("src"), p.root.join("src.real")).unwrap();
    symlink(&p.outside, p.root.join("src")).unwrap();
    let error = apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap_err();
    assert!(
        error.to_string().contains("symlink") || error.to_string().contains("leaves the project"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(p.outside.join("main.rs")).unwrap(),
        "outside file\n"
    );
}

#[test]
fn paths_outside_the_project_denied_paths_and_reserved_names_cannot_be_proposed() {
    let p = project();
    let now = Instant::now();
    let grant = p.grant(&["src/main.rs", ".env"], now);
    for bad in [
        "../outside/secret.txt",
        "/etc/passwd",
        "src/../../outside/secret.txt",
        "./src/main.rs",
        "src//main.rs",
        ".env",
        ".git/config",
        ".git/hooks/pre-commit",
        "",
    ] {
        assert!(
            Item::create(&p.scope, Path::new(bad), "x", "x").is_err(),
            "create {bad:?}"
        );
        assert!(
            Item::delete(&p.scope, &grant, now, Path::new(bad), "x").is_err(),
            "delete {bad:?}"
        );
        assert!(
            modify(&p, &grant, now, bad, &[edit("a", "b")]).is_err(),
            "modify {bad:?}"
        );
    }
    assert!(Item::create(&p.scope, Path::new("src/.kellu-tmp-1"), "x", "x").is_err());
    assert!(
        Item::create(&p.scope, Path::new("missing_dir/x.rs"), "x", "x").is_err(),
        "no implicit mkdir"
    );
    // A file the grant does not list cannot be modified or deleted even if it exists.
    assert!(modify(&p, &grant, now, "README.md", &[edit("demo", "x")]).is_err());
    assert!(Item::delete(&p.scope, &grant, now, Path::new("README.md"), "x").is_err());
}

#[test]
fn edits_must_match_exactly_once_and_errors_never_repeat_the_text() {
    let p = project();
    let now = Instant::now();
    fs::write(p.root.join("src/dup.rs"), "aaa\nbbb\naaa\n").unwrap();
    let grant = p.grant(&["src/dup.rs", "src/main.rs"], now);
    let secret = "sk-live-0123456789abcdef";
    for (edits, why) in [
        (vec![edit("aaa", "x")], "more than once"),
        (vec![edit(secret, "x")], "does not occur"),
        (vec![edit("", "x")], "is empty"),
        (vec![edit("bbb", "bbb")], "the same"),
        (
            vec![edit("bbb\naaa", "x"), edit("aaa\nbbb", "y")],
            "overlap",
        ),
    ] {
        let error = modify(&p, &grant, now, "src/dup.rs", &edits)
            .unwrap_err()
            .to_string();
        assert!(error.contains(why), "{error} should say {why}");
        assert!(
            !error.contains(secret),
            "the error repeated the model's text"
        );
    }
    let overlapping = modify(
        &p,
        &grant,
        now,
        "src/main.rs",
        &[edit("fn main", "fn start"), edit("main() {", "start() {")],
    );
    assert!(overlapping.unwrap_err().to_string().contains("overlap"));
    assert!(modify(&p, &grant, now, "src/dup.rs", &[]).is_err());
}

#[test]
fn files_that_are_not_text_are_not_proposed() {
    let p = project();
    let now = Instant::now();
    fs::write(p.root.join("blob.bin"), [0xff, 0xfe, 0x00, 0x41]).unwrap();
    let grant = p.grant(&["blob.bin"], now);
    assert!(
        modify(&p, &grant, now, "blob.bin", &[edit("A", "B")])
            .unwrap_err()
            .to_string()
            .contains("UTF-8")
    );
}

#[test]
fn a_hard_linked_file_is_refused_because_replacing_it_would_silently_unlink_it() {
    let p = project();
    let now = Instant::now();
    fs::hard_link(p.root.join("src/main.rs"), p.outside.join("alias.rs")).unwrap();
    let mut grant = p.grant(&["src/main.rs"], now);
    let item = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    let error = apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap_err();
    assert!(error.to_string().contains("hard link"), "{error}");
    assert!(p.read("src/main.rs").contains("hello"));
}

#[test]
fn a_file_owned_by_another_user_is_refused() {
    if !is_root() {
        eprintln!("SKIPPED: needs root to create a file owned by another user");
        return;
    }
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs"], now);
    let item = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    std::os::unix::fs::chown(p.root.join("src/main.rs"), Some(65534), Some(65534)).unwrap();
    let error = apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap_err();
    assert!(error.to_string().contains("another user"), "{error}");
    assert!(p.read("src/main.rs").contains("hello"));
}

#[test]
fn an_expired_grant_cannot_approve_or_apply() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs"], now);
    let item = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let late = p.session(now + TTL);
    assert!(approve(&proposal, &approve_all(&proposal), &late, &mut grant).is_err());
    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    assert!(apply(&proposal, &approved, &late, &grant, &p.backups()).is_err());
    assert!(p.read("src/main.rs").contains("hello"));
}

#[test]
fn a_decision_naming_an_unknown_item_or_hunk_is_refused_and_the_grant_is_unchanged() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs"], now);
    let item = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let session = p.session(now);
    let mut ghost = Approval::default();
    ghost.items.insert("9".into(), Choice::Full);
    assert!(approve(&proposal, &ghost, &session, &mut grant).is_err());
    for hunks in [BTreeSet::from([0]), BTreeSet::from([2]), BTreeSet::new()] {
        let mut bad = Approval::default();
        bad.items.insert("1".into(), Choice::Hunks(hunks));
        assert!(approve(&proposal, &bad, &session, &mut grant).is_err());
    }
    assert!(
        grant
            .write_approval(Path::new("src/main.rs"), now)
            .is_none(),
        "a refused decision must freeze nothing"
    );
}

#[test]
fn if_the_audit_log_cannot_be_written_nothing_is_approved() {
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs"], now);
    let item = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![item]).unwrap();
    let broken = AuditLog::new(p.root.join("src/main.rs/audit.log"));
    let session = kellu_tools::Session {
        scope: &p.scope,
        audit: &broken,
        origin: "test",
        now,
    };
    assert!(approve(&proposal, &approve_all(&proposal), &session, &mut grant).is_err());
    assert!(
        grant
            .write_approval(Path::new("src/main.rs"), now)
            .is_none()
    );
}

#[test]
fn an_approval_for_another_proposal_or_project_is_refused() {
    let p = project();
    let q = project();
    let now = Instant::now();
    let mut grant = p.grant(&["src/main.rs"], now);
    let make = |p: &Project, grant: &Grant| {
        let item = modify(p, grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
        Proposal::new(&p.scope, vec![item]).unwrap()
    };
    let one = make(&p, &grant);
    let two = make(&p, &grant);
    assert_ne!(one.id(), two.id());
    let session = p.session(now);
    let approved = approve(&one, &approve_all(&one), &session, &mut grant).unwrap();
    assert!(apply(&two, &approved, &session, &grant, &p.backups()).is_err());
    let foreign = q.session(now);
    assert!(approve(&one, &approve_all(&one), &foreign, &mut grant).is_err());
}

#[test]
fn proposals_are_bounded_and_do_not_allow_two_items_for_one_path() {
    let p = project();
    let now = Instant::now();
    let grant = p.grant(&["src/main.rs"], now);
    let a = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let b = modify(&p, &grant, now, "src/main.rs", &[edit("main", "start")]).unwrap();
    assert!(Proposal::new(&p.scope, vec![a, b]).is_err());
    assert!(Proposal::new(&p.scope, vec![]).is_err());
    let many: Vec<Item> = (0..51)
        .map(|i| Item::create(&p.scope, Path::new(&format!("f{i}.txt")), "x", "x").unwrap())
        .collect();
    assert!(Proposal::new(&p.scope, many).is_err());
}

#[test]
fn rendering_strips_terminal_escapes_from_everything_the_model_wrote() {
    let p = project();
    let now = Instant::now();
    let grant = p.grant(&["src/main.rs"], now);
    let item = Item::modify(
        &p.scope,
        &grant,
        now,
        Path::new("src/main.rs"),
        &[edit("hello", "hi\u{1b}[2J\u{1b}]0;pwned\u{7}\r there")],
        "because\u{1b}[31m red\nline two",
    )
    .unwrap();
    let text = Proposal::new(&p.scope, vec![item]).unwrap().render();
    assert!(
        !text.contains('\u{1b}') && !text.contains('\r') && !text.contains('\u{7}'),
        "{text:?}"
    );
}

#[test]
fn a_write_that_cannot_complete_is_undone() {
    // The directory of the second item is replaced by an empty one with no permission to create
    // files, so its write fails after the first item was already written.
    if is_root() {
        // root ignores directory permissions; the undo path is covered by the unit test in apply.rs
        eprintln!("SKIPPED: needs a normal user (root ignores directory permissions)");
        return;
    }
    let p = project();
    let now = Instant::now();
    let mut grant = p.grant(&["README.md", "src/main.rs"], now);
    let a = modify(&p, &grant, now, "README.md", &[edit("demo", "changed")]).unwrap();
    let b = modify(&p, &grant, now, "src/main.rs", &[edit("hello", "hola")]).unwrap();
    let proposal = Proposal::new(&p.scope, vec![a, b]).unwrap();
    let session = p.session(now);
    let approved = approve(&proposal, &approve_all(&proposal), &session, &mut grant).unwrap();
    fs::set_permissions(p.root.join("src"), fs::Permissions::from_mode(0o500)).unwrap();
    let error = apply(&proposal, &approved, &session, &grant, &p.backups()).unwrap_err();
    fs::set_permissions(p.root.join("src"), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(error.to_string().contains("nothing was applied"), "{error}");
    assert_eq!(p.read("README.md"), "# demo\n");
}
