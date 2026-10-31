//! Ask the remote host which account the SSH key authenticates as.

use anyhow::{Context, Result};
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub user: String,
    pub host: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SshResult {
    pub host: String,
    /// Account name reported by the host, when it could be recognised
    pub account: Option<String>,
    /// The host's raw greeting (or error) for display
    pub message: String,
    pub authenticated: bool,
}

/// Parse `git@host:path`, `ssh://git@host:22/path` or `git+ssh://...`.
/// Returns None for non-SSH remotes such as https.
pub fn parse_remote(url: &str) -> Option<SshTarget> {
    if let Some(rest) = url
        .strip_prefix("ssh://")
        .or_else(|| url.strip_prefix("git+ssh://"))
        .or_else(|| url.strip_prefix("ssh+git://"))
    {
        let authority = rest.split('/').next()?;
        let (user, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (u.to_string(), h),
            None => ("git".to_string(), authority),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p.parse().ok()?)),
            None => (hostport, None),
        };
        return Some(SshTarget {
            user,
            host: host.to_string(),
            port,
        });
    }

    if url.contains("://") {
        return None;
    }

    // scp-like syntax: [user@]host:path
    let (authority, _path) = url.split_once(':')?;
    if authority.contains('/') {
        // A local path that happens to contain a colon
        return None;
    }
    let (user, host) = match authority.rsplit_once('@') {
        Some((u, h)) => (u.to_string(), h.to_string()),
        None => ("git".to_string(), authority.to_string()),
    };
    Some(SshTarget {
        user,
        host,
        port: None,
    })
}

/// Connect with the configured ssh command and read the host's greeting
pub fn probe(target: &SshTarget, ssh_command: &str) -> Result<SshResult> {
    let mut cmd = format!("{} -T -o BatchMode=yes -o ConnectTimeout=10", ssh_command);
    if let Some(port) = target.port {
        cmd.push_str(&format!(" -p {}", port));
    }
    cmd.push_str(&format!(
        " {}@{}",
        shell_quote(&target.user),
        shell_quote(&target.host)
    ));

    // git runs core.sshCommand through the shell, so do the same
    let output = Command::new("sh")
        .args(["-c", &cmd])
        .output()
        .context("Failed to run ssh")?;

    // Hosts typically greet on stderr and exit non-zero because no shell is allowed
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let message = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("Warning: Permanently added"))
        .unwrap_or("(no response)")
        .to_string();
    let account = parse_greeting(&text);
    let authenticated = account.is_some() || is_auth_success(&text);

    Ok(SshResult {
        host: target.host.clone(),
        account,
        message,
        authenticated,
    })
}

/// Extract the account name from GitHub, GitLab, Bitbucket, Codeberg/Gitea greetings
pub fn parse_greeting(text: &str) -> Option<String> {
    for line in text.lines().map(str::trim) {
        // GitHub: "Hi user! You've successfully authenticated..."
        // Gitea/Codeberg: "Hi there, user! You've successfully authenticated..."
        if let Some(rest) = line
            .strip_prefix("Hi there, ")
            .or_else(|| line.strip_prefix("Hi "))
            && let Some((name, _)) = rest.split_once('!')
        {
            return Some(name.to_string());
        }
        // GitLab: "Welcome to GitLab, @user!"
        if let Some(rest) = line.strip_prefix("Welcome to GitLab, @") {
            return Some(rest.trim_end_matches('!').to_string());
        }
        // Bitbucket: "authenticated via ssh key." / "logged in as user."
        if let Some((_, rest)) = line.split_once("logged in as ") {
            return Some(rest.trim_end_matches('.').to_string());
        }
    }
    None
}

fn is_auth_success(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("successfully authenticated") || lower.contains("authenticated via")
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(user: &str, host: &str, port: Option<u16>) -> SshTarget {
        SshTarget {
            user: user.into(),
            host: host.into(),
            port,
        }
    }

    #[test]
    fn parses_scp_style() {
        assert_eq!(
            parse_remote("git@github.com:uRadical/git-whoami.git"),
            Some(target("git", "github.com", None))
        );
        assert_eq!(
            parse_remote("github-work:org/repo.git"),
            Some(target("git", "github-work", None))
        );
    }

    #[test]
    fn parses_ssh_url() {
        assert_eq!(
            parse_remote("ssh://git@gitlab.example.com:2222/group/repo.git"),
            Some(target("git", "gitlab.example.com", Some(2222)))
        );
        assert_eq!(
            parse_remote("git+ssh://alan@host/repo"),
            Some(target("alan", "host", None))
        );
    }

    #[test]
    fn ignores_non_ssh() {
        assert_eq!(parse_remote("https://github.com/uRadical/git-whoami"), None);
        assert_eq!(parse_remote("/srv/git/repo.git"), None);
        assert_eq!(parse_remote("./some/dir:with-colon"), None);
    }

    #[test]
    fn parses_greetings() {
        assert_eq!(
            parse_greeting(
                "Hi gangleri! You've successfully authenticated, but GitHub does not provide shell access."
            ),
            Some("gangleri".into())
        );
        assert_eq!(
            parse_greeting("Welcome to GitLab, @alan!"),
            Some("alan".into())
        );
        assert_eq!(
            parse_greeting(
                "Hi there, alan! You've successfully authenticated with the key named x"
            ),
            Some("alan".into())
        );
        assert_eq!(
            parse_greeting("authenticated via ssh key.\n\nlogged in as alan.\n"),
            Some("alan".into())
        );
        assert_eq!(
            parse_greeting("git@github.com: Permission denied (publickey)."),
            None
        );
    }

    #[test]
    fn quotes_shell_args() {
        assert_eq!(shell_quote("github.com"), "github.com");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
