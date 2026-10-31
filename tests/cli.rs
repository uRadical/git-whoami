//! End-to-end tests: a sandboxed HOME with a git-persona setup, checked by the real binary.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Sandbox {
    _tmp: tempfile::TempDir,
    home: PathBuf,
}

impl Sandbox {
    /// HOME with persona `work` applied to ~/work, the way `git persona apply` leaves it
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let work = home.join("work");
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(home.join(".config/git-persona")).unwrap();

        fs::write(
            home.join(".config/git-persona/config.json"),
            format!(
                r#"{{
                  "personas": {{
                    "work": {{"name": "work", "user_name": "Alan", "email": "alan@work.com", "color": "blue"}},
                    "home": {{"name": "home", "user_name": "Alan", "email": "alan@home.com", "color": "green"}}
                  }},
                  "locations": [{{"path": "{}", "persona_name": "work"}}]
                }}"#,
                work.display()
            ),
        )
        .unwrap();

        fs::write(
            home.join(".gitconfig"),
            format!(
                "[user]\n\tname = Alan\n\temail = alan@home.com\n[includeIf \"gitdir:{0}/\"]\n\tpath = {0}/.gitconfig\n",
                work.display()
            ),
        )
        .unwrap();
        fs::write(
            work.join(".gitconfig"),
            "[user]\n\tname = Alan\n\temail = alan@work.com\n",
        )
        .unwrap();

        Sandbox { _tmp: tmp, home }
    }

    fn cmd(&self, dir: &Path, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(dir)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("NO_COLOR", "1");
        for var in [
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
            "EMAIL",
            "GIT_SSH_COMMAND",
            "GIT_CONFIG_GLOBAL",
            "GIT_DIR",
        ] {
            cmd.env_remove(var);
        }
        cmd
    }

    fn whoami(&self, dir: &Path, args: &[&str]) -> Output {
        self.cmd(dir, env!("CARGO_BIN_EXE_git-whoami"))
            .args(args)
            .output()
            .unwrap()
    }

    fn git_init(&self, dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let status = self.cmd(dir, "git").args(["init", "-q"]).status().unwrap();
        assert!(status.success());
    }

    fn json(&self, dir: &Path, extra: &[&str]) -> (serde_json::Value, bool) {
        let mut args = vec!["--json"];
        args.extend_from_slice(extra);
        let out = self.whoami(dir, &args);
        let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "bad json ({e}): {}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (value, out.status.success())
    }
}

#[test]
fn verifies_persona_inside_repo() {
    let sb = Sandbox::new();
    let repo = sb.home.join("work/project");
    sb.git_init(&repo);

    let (v, ok) = sb.json(&repo, &[]);
    assert!(ok);
    assert_eq!(v["status"], "verified");
    assert_eq!(v["persona"], "work");
    assert_eq!(v["git"]["author"]["email"], "alan@work.com");
    assert_eq!(v["identity_source"], "~/work/.gitconfig");

    let out = sb.whoami(&repo, &[]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "work\n");
    assert!(out.stderr.is_empty());
}

#[test]
fn default_output_signals_mismatch_by_exit_code() {
    let sb = Sandbox::new();
    let dir = sb.home.join("work/not-a-repo");
    fs::create_dir_all(&dir).unwrap();

    let out = sb.whoami(&dir, &[]);
    assert!(!out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "work\n");
    // Notes are for interactive use only; captured output stays clean
    assert!(out.stderr.is_empty());
}

#[test]
fn default_output_falls_back_to_email_without_persona() {
    let sb = Sandbox::new();
    let repo = sb.home.join("oss/project");
    sb.git_init(&repo);

    let out = sb.whoami(&repo, &[]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "alan@home.com\n");
    assert!(out.stderr.is_empty());

    // Outside a repo there is nothing to commit, so no fallback
    let plain = sb.home.join("downloads");
    fs::create_dir_all(&plain).unwrap();
    let out = sb.whoami(&plain, &[]);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn reports_mismatch_outside_repo() {
    let sb = Sandbox::new();
    let dir = sb.home.join("work/not-a-repo");
    fs::create_dir_all(&dir).unwrap();

    let (v, ok) = sb.json(&dir, &[]);
    assert!(!ok);
    assert_eq!(v["status"], "mismatch");
    assert_eq!(v["mismatches"][0]["field"], "author");
    assert_eq!(v["mismatches"][0]["actual"], "Alan <alan@home.com>");
    let warnings = v["warnings"].to_string();
    assert!(
        warnings.contains("not inside a git repository"),
        "{warnings}"
    );
}

#[test]
fn repo_local_override_is_a_mismatch() {
    let sb = Sandbox::new();
    let repo = sb.home.join("work/project");
    sb.git_init(&repo);
    let status = sb
        .cmd(&repo, "git")
        .args(["config", "user.email", "oops@elsewhere.com"])
        .status()
        .unwrap();
    assert!(status.success());

    let (v, ok) = sb.json(&repo, &[]);
    assert!(!ok);
    assert_eq!(v["status"], "mismatch");
    assert_eq!(v["identity_source"], "~/work/project/.git/config");
}

#[test]
fn env_override_is_detected() {
    let sb = Sandbox::new();
    let repo = sb.home.join("work/project");
    sb.git_init(&repo);

    let out = sb
        .cmd(&repo, env!("CARGO_BIN_EXE_git-whoami"))
        .env("GIT_AUTHOR_EMAIL", "ci@bot.com")
        .arg("--json")
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!out.status.success());
    assert_eq!(v["mismatches"][0]["field"], "author");
    assert_eq!(v["identity_source"], "env $GIT_AUTHOR_EMAIL");
    assert!(v["warnings"].to_string().contains("GIT_AUTHOR_EMAIL"));
}

#[test]
fn no_persona_is_ok_unless_required() {
    let sb = Sandbox::new();
    let repo = sb.home.join("oss/project");
    sb.git_init(&repo);

    let (v, ok) = sb.json(&repo, &[]);
    assert!(ok);
    assert_eq!(v["status"], "no_persona");
    assert_eq!(v["matching_personas"], serde_json::json!(["home"]));

    let out = sb.whoami(&repo, &["--quiet", "--require-persona"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn verbose_output_shows_report() {
    let sb = Sandbox::new();
    let repo = sb.home.join("work/project");
    sb.git_init(&repo);

    let out = sb.whoami(&repo, &["--verbose"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("Persona    work (~/work)"), "{text}");
    assert!(text.contains("✓ Identity matches persona 'work'"), "{text}");
}

#[test]
fn prompt_mode_colors_name_and_falls_back_to_email() {
    let sb = Sandbox::new();
    let repo = sb.home.join("work/project");
    sb.git_init(&repo);

    // Prompt mode colors even though stdout is a pipe and NO_COLOR is set by the sandbox
    let out = sb.whoami(&repo, &["--prompt", "zsh"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "%{\x1b[34m%}work%{\x1b[0m%}"
    );

    let other = sb.home.join("oss/project");
    sb.git_init(&other);
    let out = sb.whoami(&other, &["--prompt", "oh-my-posh"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "alan@home.com");
    assert!(out.stderr.is_empty());

    let plain = sb.home.join("downloads");
    fs::create_dir_all(&plain).unwrap();
    let out = sb.whoami(&plain, &["--prompt", "bash"]);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}
