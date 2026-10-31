//! Ask git what it will actually use, rather than trusting any config file.

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::persona::display_path;

/// Environment variables that silently override the configured identity
pub const IDENTITY_ENV_VARS: &[&str] = &[
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "EMAIL",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ident {
    pub name: String,
    pub email: String,
}

impl std::fmt::Display for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} <{}>", self.name, self.email)
    }
}

/// A resolved config value and where it came from
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Setting {
    pub value: String,
    pub origin: String,
}

/// Everything git will use for identity in the current directory
#[derive(Debug, Clone, Default, Serialize)]
pub struct Effective {
    pub author: Option<Ident>,
    pub committer: Option<Ident>,
    pub user_name: Option<Setting>,
    pub user_email: Option<Setting>,
    pub signing_key: Option<Setting>,
    pub gpg_sign: Option<Setting>,
    pub ssh_command: Option<Setting>,
}

impl Effective {
    pub fn resolve() -> Result<Self> {
        Ok(Effective {
            author: author_ident()?,
            committer: var_ident("GIT_COMMITTER_IDENT")?,
            user_name: config_get("user.name")?,
            user_email: config_get("user.email")?,
            signing_key: config_get("user.signingkey")?,
            gpg_sign: config_get("commit.gpgsign")?,
            ssh_command: match std::env::var("GIT_SSH_COMMAND") {
                Ok(cmd) if !cmd.is_empty() => Some(Setting {
                    value: cmd,
                    origin: "env GIT_SSH_COMMAND".to_string(),
                }),
                _ => config_get("core.sshCommand")?,
            },
        })
    }

    /// Whether commits will be signed (commit.gpgsign is a git boolean)
    pub fn signs_commits(&self) -> bool {
        self.gpg_sign
            .as_ref()
            .is_some_and(|s| is_git_true(&s.value))
    }
}

fn git(args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        .args(args)
        .output()
        .context("Failed to run git. Is it installed and on your PATH?")
}

/// The identity commits will be authored with
pub fn author_ident() -> Result<Option<Ident>> {
    var_ident("GIT_AUTHOR_IDENT")
}

/// Identity from `git var`, which accounts for config *and* environment overrides
fn var_ident(var: &str) -> Result<Option<Ident>> {
    let output = git(&["var", var])?;
    if !output.status.success() {
        // git refuses when no identity is configured
        return Ok(None);
    }
    Ok(parse_ident(&String::from_utf8_lossy(&output.stdout)))
}

/// Parse `Name <email> 1700000000 +0000`
pub fn parse_ident(line: &str) -> Option<Ident> {
    let line = line.trim();
    let open = line.find('<')?;
    let close = open + line[open..].find('>')?;
    Some(Ident {
        name: line[..open].trim().to_string(),
        email: line[open + 1..close].trim().to_string(),
    })
}

fn config_get(key: &str) -> Result<Option<Setting>> {
    let output = git(&["config", "--show-origin", "-z", "--get", key])?;
    match output.status.code() {
        Some(0) => Ok(parse_config_z(&String::from_utf8_lossy(&output.stdout))),
        // 1 means the key isn't set
        Some(1) => Ok(None),
        _ => anyhow::bail!(
            "git config --get {} failed: {}",
            key,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

/// Parse `git config --show-origin -z --get` output: `origin\0value\0`
pub fn parse_config_z(out: &str) -> Option<Setting> {
    let mut parts = out.split('\0');
    let origin = parts.next()?;
    let value = parts.next()?;
    Some(Setting {
        value: value.to_string(),
        origin: display_origin(origin),
    })
}

/// Turn `file:/Users/x/.gitconfig` into `~/.gitconfig`
fn display_origin(origin: &str) -> String {
    match origin.strip_prefix("file:") {
        Some(path) => {
            let path = Path::new(path);
            // Repo-local origins like `.git/config` are relative to the cwd.
            // Check for a root rather than is_relative(): on Windows a
            // drive-less path like `/etc/gitconfig` is "relative" but rooted.
            let path = if !path.has_root() {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .unwrap_or_else(|_| path.to_path_buf())
            } else {
                path.to_path_buf()
            };
            display_path(&path)
        }
        None => origin.trim_end_matches(':').to_string(),
    }
}

/// Git's boolean parsing for config values
pub fn is_git_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "on" | "1"
    )
}

/// The top level of the enclosing work tree, if any
pub fn work_tree_root() -> Result<Option<PathBuf>> {
    let output = git(&["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!root.is_empty()).then(|| PathBuf::from(root)))
}

/// URL for a remote, or None if it doesn't exist
pub fn remote_url(remote: &str) -> Result<Option<String>> {
    let output = git(&["remote", "get-url", remote])?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_var_ident() {
        let ident = parse_ident("Alan Smith <alan@example.com> 1793360629 +0000\n").unwrap();
        assert_eq!(ident.name, "Alan Smith");
        assert_eq!(ident.email, "alan@example.com");
        assert_eq!(ident.to_string(), "Alan Smith <alan@example.com>");
    }

    #[test]
    fn rejects_malformed_ident() {
        assert!(parse_ident("no email here").is_none());
        assert!(parse_ident("broken <email").is_none());
    }

    #[test]
    fn parses_config_with_origin() {
        let setting = parse_config_z("file:/etc/gitconfig\0alan@example.com\0").unwrap();
        assert_eq!(setting.value, "alan@example.com");
        assert_eq!(setting.origin, "/etc/gitconfig");
    }

    #[test]
    fn resolves_repo_local_origin_against_cwd() {
        let setting = parse_config_z("file:.git/config\0x\0").unwrap();
        let expected = display_path(&std::env::current_dir().unwrap().join(".git/config"));
        assert_eq!(setting.origin, expected);
    }

    #[test]
    fn parses_non_file_origin() {
        let setting = parse_config_z("command line:\0x\0").unwrap();
        assert_eq!(setting.origin, "command line");
    }

    #[test]
    fn git_booleans() {
        for v in ["true", "YES", "on", "1"] {
            assert!(is_git_true(v), "{v}");
        }
        for v in ["false", "no", "off", "0", ""] {
            assert!(!is_git_true(v), "{v}");
        }
    }
}
