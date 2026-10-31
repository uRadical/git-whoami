//! Compare what git resolves against what git-persona expects.

use serde::Serialize;

use crate::git::Effective;
use crate::persona::{Persona, expand_tilde};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mismatch {
    pub field: &'static str,
    pub expected: String,
    pub actual: String,
}

const NOT_SET: &str = "(not set)";

/// All the ways the effective identity differs from `persona`
pub fn check(persona: &Persona, eff: &Effective) -> Vec<Mismatch> {
    let mut out = Vec::new();

    for (field, ident) in [("author", &eff.author), ("committer", &eff.committer)] {
        let expected = format!("{} <{}>", persona.user_name, persona.email);
        match ident {
            Some(i)
                if i.name == persona.user_name && i.email.eq_ignore_ascii_case(&persona.email) => {}
            Some(i) => out.push(Mismatch {
                field,
                expected,
                actual: i.to_string(),
            }),
            None => out.push(Mismatch {
                field,
                expected,
                actual: NOT_SET.to_string(),
            }),
        }
    }

    if let Some(key) = persona.ssh_key.as_deref().filter(|k| !k.is_empty()) {
        let uses_key = eff
            .ssh_command
            .as_ref()
            .is_some_and(|c| c.value.contains(&expand_tilde(key)) || c.value.contains(key));
        if !uses_key {
            out.push(Mismatch {
                field: "ssh key",
                expected: key.to_string(),
                actual: eff
                    .ssh_command
                    .as_ref()
                    .map_or(NOT_SET.to_string(), |c| c.value.clone()),
            });
        }
    }

    if let Some(key) = persona.gpg_key.as_deref().filter(|k| !k.is_empty()) {
        let actual = eff.signing_key.as_ref().map(|s| s.value.as_str());
        if actual != Some(key) {
            out.push(Mismatch {
                field: "signing key",
                expected: key.to_string(),
                actual: actual.unwrap_or(NOT_SET).to_string(),
            });
        }
        if !eff.signs_commits() {
            out.push(Mismatch {
                field: "commit.gpgsign",
                expected: "true".to_string(),
                actual: eff
                    .gpg_sign
                    .as_ref()
                    .map_or(NOT_SET.to_string(), |s| s.value.clone()),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Ident, Setting};

    fn persona() -> Persona {
        Persona {
            name: "work".into(),
            user_name: "Alan".into(),
            email: "alan@work.com".into(),
            ssh_key: Some("/keys/work".into()),
            gpg_key: Some("ABC123".into()),
            color: "blue".into(),
        }
    }

    fn setting(value: &str) -> Option<Setting> {
        Some(Setting {
            value: value.into(),
            origin: "test".into(),
        })
    }

    fn matching() -> Effective {
        let ident = Some(Ident {
            name: "Alan".into(),
            email: "alan@work.com".into(),
        });
        Effective {
            author: ident.clone(),
            committer: ident,
            signing_key: setting("ABC123"),
            gpg_sign: setting("true"),
            ssh_command: setting("ssh -i /keys/work -o IdentitiesOnly=yes"),
            ..Default::default()
        }
    }

    #[test]
    fn matching_identity_has_no_mismatches() {
        assert!(check(&persona(), &matching()).is_empty());
    }

    #[test]
    fn detects_wrong_email() {
        let mut eff = matching();
        eff.author = Some(Ident {
            name: "Alan".into(),
            email: "alan@home.com".into(),
        });
        let m = check(&persona(), &eff);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].field, "author");
        assert_eq!(m[0].actual, "Alan <alan@home.com>");
    }

    #[test]
    fn detects_missing_identity() {
        let mut eff = matching();
        eff.author = None;
        eff.committer = None;
        let m = check(&persona(), &eff);
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|m| m.actual == NOT_SET));
    }

    #[test]
    fn detects_wrong_ssh_key() {
        let mut eff = matching();
        eff.ssh_command = setting("ssh -i /keys/home");
        let m = check(&persona(), &eff);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].field, "ssh key");
    }

    #[test]
    fn detects_signing_disabled() {
        let mut eff = matching();
        eff.gpg_sign = setting("false");
        let m = check(&persona(), &eff);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].field, "commit.gpgsign");
    }

    #[test]
    fn optional_fields_not_checked_when_persona_lacks_them() {
        let mut p = persona();
        p.ssh_key = None;
        p.gpg_key = None;
        let eff = Effective {
            ssh_command: None,
            signing_key: None,
            gpg_sign: None,
            ..matching()
        };
        assert!(check(&p, &eff).is_empty());
    }
}
