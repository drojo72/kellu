//! Tool risk levels and what each one requires before it may run.
//!
//! Every tool declares exactly one [`Risk`]; a tool without one cannot run (the registry in M5
//! enforces it by making the level a required argument). This module is the single place that
//! maps a risk to a requirement.

/// What a tool can do to the user's machine or accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Risk {
    /// List, read, search, `git status`, `git log`.
    Read,
    /// Edit a file, `git commit`.
    Write,
    /// Install packages, fetch URLs.
    Network,
    /// Delete files or branches, force-push, repository deletion.
    Destructive,
}

impl Risk {
    /// Lowercase name, as written in audit records.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Network => "network",
            Self::Destructive => "destructive",
        }
    }
}

/// What must happen before a tool of a given risk runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// It may run without asking (inside the sandbox or path policy).
    Automatic,
    /// The user must confirm.
    Confirm,
    /// The user must type an explicit confirmation; never automatic.
    TypedConfirm,
    /// It may not run in the current configuration.
    Denied,
}

/// The requirement for a tool of risk `risk`.
///
/// `network_profile` says whether the sandbox `network` profile is enabled. Without it a
/// network tool is denied outright: confirmation alone is not enough, because the sandbox would
/// have no network to give it (fail closed).
#[must_use]
pub fn requirement(risk: Risk, network_profile: bool) -> Requirement {
    match risk {
        Risk::Read => Requirement::Automatic,
        Risk::Write => Requirement::Confirm,
        Risk::Network if network_profile => Requirement::Confirm,
        Risk::Network => Requirement::Denied,
        Risk::Destructive => Requirement::TypedConfirm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_matches_the_security_policy() {
        assert_eq!(requirement(Risk::Read, false), Requirement::Automatic);
        assert_eq!(requirement(Risk::Write, false), Requirement::Confirm);
        assert_eq!(requirement(Risk::Network, true), Requirement::Confirm);
        assert_eq!(
            requirement(Risk::Destructive, false),
            Requirement::TypedConfirm
        );
    }

    #[test]
    fn network_without_its_profile_is_denied() {
        assert_eq!(requirement(Risk::Network, false), Requirement::Denied);
    }

    #[test]
    fn only_reads_are_ever_automatic() {
        for risk in [Risk::Write, Risk::Network, Risk::Destructive] {
            for profile in [false, true] {
                assert_ne!(
                    requirement(risk, profile),
                    Requirement::Automatic,
                    "{risk:?}"
                );
            }
        }
    }

    #[test]
    fn destructive_never_downgrades_with_the_network_profile() {
        assert_eq!(
            requirement(Risk::Destructive, true),
            Requirement::TypedConfirm
        );
    }

    #[test]
    fn risks_have_stable_names() {
        let names: Vec<_> = [Risk::Read, Risk::Write, Risk::Network, Risk::Destructive]
            .map(Risk::as_str)
            .into();
        assert_eq!(names, ["read", "write", "network", "destructive"]);
    }
}
