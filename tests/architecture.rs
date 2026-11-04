//! Workspace architecture rules, checked against `cargo metadata`.
//!
//! Direction (see docs/ARCHITECTURE.md): frontends -> core -> tools -> sandbox -> security
//! -> common, with `providers` and `projects` also below `core`. Cargo already rejects cycles;
//! this test rejects upward and sideways edges that would still compile.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// Every crate the workspace may contain, with the internal crates it may depend on.
/// A crate is added here in the same change that creates it (when its milestone starts).
const ALLOWED: &[(&str, &[&str])] = &[
    ("kellu-common", &[]),
    ("kellu-security", &["kellu-common"]),
    // The sandbox derives its mounts from the `Grant`, which lives in `kellu-security`.
    ("kellu-sandbox", &["kellu-common", "kellu-security"]),
    ("kellu-providers", &["kellu-common"]),
    ("kellu-projects", &["kellu-common"]),
    (
        "kellu-tools",
        &["kellu-common", "kellu-security", "kellu-sandbox"],
    ),
    (
        "kellu-core",
        &[
            "kellu-common",
            "kellu-security",
            "kellu-sandbox",
            "kellu-providers",
            "kellu-projects",
            "kellu-tools",
        ],
    ),
    (
        "kellu-frontends",
        &[
            "kellu-common",
            "kellu-security",
            "kellu-sandbox",
            "kellu-providers",
            "kellu-projects",
            "kellu-tools",
            "kellu-core",
        ],
    ),
    (
        "kellu",
        &[
            "kellu-common",
            "kellu-security",
            "kellu-sandbox",
            "kellu-providers",
            "kellu-projects",
            "kellu-tools",
            "kellu-core",
            "kellu-frontends",
        ],
    ),
];

struct Package {
    version: String,
    manifest: PathBuf,
    internal_deps: BTreeSet<String>,
}

fn workspace() -> BTreeMap<String, Package> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success(), "cargo metadata failed");
    let meta: Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = meta["packages"].as_array().unwrap();
    let names: BTreeSet<String> = packages
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_owned())
        .collect();
    packages
        .iter()
        .map(|p| {
            let internal_deps = p["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                // Normal and build dependencies only: dev-dependencies are test support.
                .filter(|d| d["kind"].is_null() || d["kind"] == "build")
                .map(|d| d["name"].as_str().unwrap().to_owned())
                .filter(|name| names.contains(name))
                .collect();
            let package = Package {
                version: p["version"].as_str().unwrap().to_owned(),
                manifest: PathBuf::from(p["manifest_path"].as_str().unwrap()),
                internal_deps,
            };
            (p["name"].as_str().unwrap().to_owned(), package)
        })
        .collect()
}

#[test]
fn every_workspace_crate_is_a_declared_layer() {
    let declared: BTreeSet<&str> = ALLOWED.iter().map(|(name, _)| *name).collect();
    for name in workspace().keys() {
        assert!(
            declared.contains(name.as_str()),
            "crate `{name}` is not declared in ALLOWED"
        );
    }
}

#[test]
fn crates_only_depend_on_allowed_layers() {
    let packages = workspace();
    for (name, allowed) in ALLOWED {
        let Some(package) = packages.get(*name) else {
            continue;
        };
        let allowed: BTreeSet<String> = allowed.iter().map(|s| (*s).to_owned()).collect();
        let forbidden: Vec<_> = package.internal_deps.difference(&allowed).collect();
        assert!(
            forbidden.is_empty(),
            "`{name}` must not depend on {forbidden:?}"
        );
    }
}

#[test]
fn library_crates_live_under_crates_and_match_their_directory() {
    for (name, package) in workspace() {
        let dir = package.manifest.parent().unwrap();
        assert_eq!(
            dir.file_name().unwrap().to_str().unwrap(),
            name,
            "directory name of `{name}`"
        );
        if name != "kellu" {
            assert_eq!(
                dir.parent().unwrap().file_name().unwrap(),
                "crates",
                "`{name}` must be in crates/"
            );
        }
    }
}

#[test]
fn every_crate_inherits_the_workspace_lints() {
    // `unsafe_code = "forbid"` and the unwrap/expect/panic denies live in the workspace manifest.
    for (name, package) in workspace() {
        let manifest = fs::read_to_string(&package.manifest).unwrap();
        assert!(
            manifest.contains("[lints]\nworkspace = true"),
            "`{name}` must contain `[lints]` with `workspace = true`"
        );
    }
}

#[test]
fn the_version_is_single_sourced() {
    let packages = workspace();
    let versions: BTreeSet<&str> = packages.values().map(|p| p.version.as_str()).collect();
    assert_eq!(
        versions.len(),
        1,
        "all crates must share the workspace version: {versions:?}"
    );
}
