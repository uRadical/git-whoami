mod git;
mod persona;
mod ssh;
mod verify;

use anyhow::Result;
use clap::Parser;
use colored::{Color, Colorize};
use serde::Serialize;
use std::env;
use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;

use crate::git::{Effective, IDENTITY_ENV_VARS};
use crate::persona::{Config, Persona, display_path};
use crate::ssh::SshResult;
use crate::verify::Mismatch;

#[derive(Parser)]
#[command(name = "git-whoami")]
#[command(about = "Print the active git-persona persona and verify git is using it")]
#[command(version)]
pub struct Cli {
    /// Show the full identity report instead of just the persona name
    #[arg(short, long, conflicts_with = "quiet")]
    verbose: bool,
    /// Output as JSON
    #[arg(long, conflicts_with_all = ["verbose", "quiet"])]
    json: bool,
    /// Print nothing; report the result through the exit code only
    #[arg(short, long)]
    quiet: bool,
    /// Also fail when no persona is applied to this directory
    #[arg(short = 'r', long)]
    require_persona: bool,
    /// Connect to the remote over SSH and report which account the key authenticates as (implies --verbose)
    #[arg(long, conflicts_with = "quiet")]
    ssh: bool,
    /// Remote to use with --ssh
    #[arg(long, default_value = "origin", requires = "ssh")]
    remote: String,
    /// Print the persona name colored for a shell prompt, with zero-width escapes
    #[arg(long, value_enum, value_name = "SHELL", conflicts_with_all = ["verbose", "json", "quiet", "ssh"])]
    prompt: Option<PromptShell>,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum PromptShell {
    Bash,
    Zsh,
    Fish,
    /// oh-my-posh color tags, for a text segment reading an env var
    OhMyPosh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    /// Git's identity matches the persona applied to this directory
    Verified,
    /// A persona is applied here but git resolves something else
    Mismatch,
    /// git-persona has nothing applied here
    NoPersona,
}

#[derive(Serialize)]
struct Report {
    status: Status,
    ok: bool,
    /// Persona git-persona has applied to this directory
    persona: Option<String>,
    #[serde(skip)]
    persona_color: Option<String>,
    /// Where that persona was applied
    location: Option<String>,
    /// Personas whose name/email match the effective identity
    matching_personas: Vec<String>,
    in_repository: bool,
    identity_source: Option<String>,
    git: Effective,
    mismatches: Vec<Mismatch>,
    warnings: Vec<String>,
    ssh: Option<SshResult>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            // Never let an error leak into the prompt
            if !cli.quiet && cli.prompt.is_none() {
                eprintln!("{} {:#}", "error:".red(), e);
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<bool> {
    // Prompts run in every directory: where no persona applies, only the
    // fallback email is needed, so skip building the full report
    if let Some(shell) = cli.prompt
        && Config::load()?
            .persona_for_path(&env::current_dir()?)
            .is_none()
    {
        if let Some(email) = fallback_email()? {
            print!("{}", prompt_fallback(&email, shell));
        }
        return Ok(true);
    }

    let report = build_report(cli)?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if let Some(shell) = cli.prompt {
        if let Some(name) = &report.persona {
            print!(
                "{}",
                prompt_name(name, report.persona_color.as_deref(), shell)
            );
        }
    } else if cli.verbose || cli.ssh {
        print_report(&report);
    } else if !cli.quiet {
        print_name(&report);
    }

    Ok(report.ok)
}

fn build_report(cli: &Cli) -> Result<Report> {
    let config = Config::load()?;
    let cwd = env::current_dir()?;
    let eff = Effective::resolve()?;
    let repo_root = git::work_tree_root()?;
    let applied = config.persona_for_path(&cwd);

    let mut mismatches = applied
        .map(|(_, p)| verify::check(p, &eff))
        .unwrap_or_default();

    let matching_personas = eff
        .author
        .as_ref()
        .map(|a| {
            config
                .personas_matching(&a.name, &a.email)
                .into_iter()
                .map(|p| p.name.clone())
                .collect()
        })
        .unwrap_or_default();

    let mut warnings: Vec<String> = IDENTITY_ENV_VARS
        .iter()
        .filter(|v| env::var_os(v).is_some_and(|s| !s.is_empty()))
        .map(|v| format!("${} is set and overrides git config", v))
        .collect();

    if let Some((loc, persona)) = applied
        && !mismatches.is_empty()
    {
        warnings.extend(mismatch_hints(&loc.path, persona, repo_root.is_some()));
    }

    let ssh = if cli.ssh {
        let result = probe_ssh(&cli.remote, &eff, &mut warnings)?;
        if let Some(ref r) = result
            && !r.authenticated
        {
            mismatches.push(Mismatch {
                field: "ssh auth",
                expected: format!("authenticated at {}", r.host),
                actual: r.message.clone(),
            });
        }
        result
    } else {
        None
    };

    let status = match applied {
        Some(_) if mismatches.is_empty() => Status::Verified,
        Some(_) => Status::Mismatch,
        None => Status::NoPersona,
    };

    let ok = match status {
        Status::Verified => true,
        Status::Mismatch => false,
        Status::NoPersona => !cli.require_persona && eff.author.is_some() && mismatches.is_empty(),
    };

    Ok(Report {
        status,
        ok,
        persona: applied.map(|(_, p)| p.name.clone()),
        persona_color: applied.map(|(_, p)| p.color.clone()),
        location: applied.map(|(l, _)| display_path(&l.path)),
        matching_personas,
        in_repository: repo_root.is_some(),
        identity_source: identity_source(&eff),
        git: eff,
        mismatches,
        warnings,
        ssh,
    })
}

/// Where the author email came from, following git's precedence:
/// $GIT_AUTHOR_EMAIL, then user.email, then $EMAIL
fn identity_source(eff: &Effective) -> Option<String> {
    let env_set = |var: &str| env::var_os(var).is_some_and(|s| !s.is_empty());

    if env_set("GIT_AUTHOR_EMAIL") {
        Some("env $GIT_AUTHOR_EMAIL".to_string())
    } else if let Some(email) = &eff.user_email {
        Some(email.origin.clone())
    } else if env_set("EMAIL") {
        Some("env $EMAIL".to_string())
    } else {
        None
    }
}

/// Explain the usual reasons git-persona's setup isn't taking effect
fn mismatch_hints(location: &Path, persona: &Persona, in_repo: bool) -> Vec<String> {
    let mut hints = Vec::new();
    let loc = display_path(location);

    if !in_repo {
        hints.push(
            "not inside a git repository: includeIf \"gitdir:\" rules only apply within a repo"
                .to_string(),
        );
    }

    if !location.join(".gitconfig").exists() {
        hints.push(format!(
            "{}/.gitconfig is missing; run `git persona apply {}` there",
            loc, persona.name
        ));
    }

    let global = dirs::home_dir().map(|h| h.join(".gitconfig"));
    let has_include = global
        .and_then(|g| std::fs::read_to_string(g).ok())
        .is_some_and(|c| has_include_if(&c, &[&loc, &location.display().to_string()]));
    if !has_include {
        hints.push(format!(
            "~/.gitconfig has no includeIf for {}; `git persona apply --manual {}` there prints the line to add",
            loc, persona.name
        ));
    }

    hints
}

/// Whether `gitconfig` has an `includeIf "gitdir:<dir>/"` for any spelling of the directory.
/// Matches the whole directive so a nested location doesn't count for its parent.
fn has_include_if(gitconfig: &str, dirs: &[&str]) -> bool {
    dirs.iter().any(|dir| {
        let dir = dir.trim_end_matches('/');
        ["gitdir:", "gitdir/i:"]
            .iter()
            .any(|kw| gitconfig.contains(&format!("{}{}/\"]", kw, dir)))
    })
}

fn probe_ssh(
    remote: &str,
    eff: &Effective,
    warnings: &mut Vec<String>,
) -> Result<Option<SshResult>> {
    let Some(url) = git::remote_url(remote)? else {
        warnings.push(format!("--ssh: no remote named '{}'", remote));
        return Ok(None);
    };
    let Some(target) = ssh::parse_remote(&url) else {
        warnings.push(format!(
            "--ssh: remote '{}' is not an SSH URL ({})",
            remote, url
        ));
        return Ok(None);
    };
    let command = eff.ssh_command.as_ref().map_or("ssh", |c| c.value.as_str());
    ssh::probe(&target, command).map(Some)
}

/// Default output: just the persona name. Notes about mismatches or a missing
/// persona go to stderr, and only when run interactively, so `$(git whoami)`
/// captures the name or nothing at all.
fn print_name(r: &Report) {
    if let Some(name) = &r.persona {
        let color = color_to_colored(r.persona_color.as_deref().unwrap_or("white"));
        println!("{}", name.color(color));
    } else if let Some(author) = r.git.author.as_ref().filter(|_| r.in_repository) {
        println!("{}", author.email);
    }
    if !std::io::stdout().is_terminal() {
        return;
    }
    let note = match r.status {
        Status::Verified => return,
        Status::Mismatch => format!(
            "{} git is not using persona '{}'",
            "warning:".yellow(),
            r.persona.as_deref().unwrap_or_default()
        ),
        Status::NoPersona => "no persona applied here".dimmed().to_string(),
    };
    eprintln!("{} {}", note, "(git whoami -v for details)".dimmed());
}

/// Persona name in its color, with the escape codes marked zero-width so the
/// shell measures the prompt correctly
fn prompt_name(name: &str, color: Option<&str>, shell: PromptShell) -> String {
    let color = color.unwrap_or("white");
    if shell == PromptShell::OhMyPosh {
        return format!("<{}>{}</>", omp_color(color), name);
    }
    let (open, close, name) = match shell {
        PromptShell::Zsh => ("%{", "%}", name.replace('%', "%%")),
        PromptShell::Bash => ("\x01", "\x02", name.to_string()),
        PromptShell::Fish | PromptShell::OhMyPosh => ("", "", name.to_string()),
    };
    let fg = color_to_colored(color).to_fg_str();
    format!("{open}\x1b[{fg}m{close}{name}{open}\x1b[0m{close}")
}

/// Without a persona, the email git will commit with, but only inside a repo:
/// elsewhere there is nothing to commit, so the prompt stays quiet
fn fallback_email() -> Result<Option<String>> {
    if git::work_tree_root()?.is_none() {
        return Ok(None);
    }
    Ok(git::author_ident()?.map(|a| a.email))
}

/// The fallback email, uncolored so it reads differently from a persona name
fn prompt_fallback(email: &str, shell: PromptShell) -> String {
    match shell {
        PromptShell::Zsh => email.replace('%', "%%"),
        _ => email.to_string(),
    }
}

/// git-persona color name to oh-my-posh's: `bright-green` is `lightGreen`
fn omp_color(color: &str) -> String {
    match color.to_lowercase().strip_prefix("bright-") {
        Some(base) if !base.is_empty() => {
            format!("light{}{}", base[..1].to_uppercase(), &base[1..])
        }
        _ => color.to_lowercase(),
    }
}

fn print_report(r: &Report) {
    let row = |label: &str, value: String| println!("{:<10} {}", label.bold(), value);

    match &r.persona {
        Some(name) => {
            let color = color_to_colored(r.persona_color.as_deref().unwrap_or("white"));
            row(
                "Persona",
                format!(
                    "{} {}",
                    name.color(color).bold(),
                    format!("({})", r.location.as_deref().unwrap_or("")).dimmed()
                ),
            );
        }
        None => row("Persona", "(none applied here)".dimmed().to_string()),
    }

    match &r.git.author {
        Some(a) => row("Identity", a.to_string()),
        None => row("Identity", "(not configured)".red().to_string()),
    }
    if let Some(src) = &r.identity_source {
        row("Source", src.dimmed().to_string());
    }
    if let Some(c) = r
        .git
        .committer
        .as_ref()
        .filter(|c| r.git.author.as_ref() != Some(*c))
    {
        row("Committer", c.to_string());
    }
    if let Some(key) = &r.git.signing_key {
        let signing = if r.git.signs_commits() {
            "commits signed"
        } else {
            "commit.gpgsign off"
        };
        row(
            "Signing",
            format!("{} {}", key.value, format!("({})", signing).dimmed()),
        );
    }
    if let Some(cmd) = &r.git.ssh_command {
        row("SSH", cmd.value.clone());
    }
    if let Some(s) = &r.ssh {
        let value = match (&s.account, s.authenticated) {
            (Some(acct), _) => format!("{} {}", acct.bold(), format!("@ {}", s.host).dimmed()),
            (None, true) => format!("{} {}", s.message, format!("@ {}", s.host).dimmed()),
            (None, false) => format!("{} {}", s.message.red(), format!("@ {}", s.host).dimmed()),
        };
        row("Remote", value);
    }

    println!();
    match r.status {
        Status::Verified => println!(
            "{} Identity matches persona '{}'",
            "✓".green(),
            r.persona.as_deref().unwrap_or_default()
        ),
        Status::Mismatch => {
            println!(
                "{} Identity does not match persona '{}'",
                "✗".red(),
                r.persona.as_deref().unwrap_or_default()
            );
            for m in &r.mismatches {
                println!(
                    "    {}: expected {}, got {}",
                    m.field,
                    m.expected.green(),
                    m.actual.red()
                );
            }
        }
        Status::NoPersona => {
            let mark = if r.ok { "•".yellow() } else { "✗".red() };
            if r.git.author.is_none() {
                println!(
                    "{} No persona applied here and no identity configured",
                    mark
                );
            } else if r.matching_personas.is_empty() {
                println!(
                    "{} No persona applied here; identity is not from git-persona",
                    mark
                );
            } else {
                println!(
                    "{} No persona applied here; identity matches {}",
                    mark,
                    r.matching_personas
                        .iter()
                        .map(|n| format!("'{}'", n))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            for m in &r.mismatches {
                println!("    {}: {}", m.field, m.actual.red());
            }
        }
    }

    for w in &r.warnings {
        println!("  {} {}", "hint:".yellow(), w);
    }
}

/// Convert git-persona color name to colored crate Color
fn color_to_colored(color: &str) -> Color {
    match color.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "bright-black" => Color::BrightBlack,
        "bright-red" => Color::BrightRed,
        "bright-green" => Color::BrightGreen,
        "bright-yellow" => Color::BrightYellow,
        "bright-blue" => Color::BrightBlue,
        "bright-magenta" => Color::BrightMagenta,
        "bright-cyan" => Color::BrightCyan,
        "bright-white" => Color::BrightWhite,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_name_wraps_escapes_per_shell() {
        assert_eq!(
            prompt_name("work", Some("blue"), PromptShell::Zsh),
            "%{\x1b[34m%}work%{\x1b[0m%}"
        );
        assert_eq!(
            prompt_name("work", Some("bright-green"), PromptShell::Bash),
            "\x01\x1b[92m\x02work\x01\x1b[0m\x02"
        );
        assert_eq!(
            prompt_name("work", Some("red"), PromptShell::Fish),
            "\x1b[31mwork\x1b[0m"
        );
    }

    #[test]
    fn prompt_name_uses_oh_my_posh_color_tags() {
        assert_eq!(
            prompt_name("work", Some("blue"), PromptShell::OhMyPosh),
            "<blue>work</>"
        );
        assert_eq!(
            prompt_name("work", Some("bright-green"), PromptShell::OhMyPosh),
            "<lightGreen>work</>"
        );
    }

    #[test]
    fn prompt_name_escapes_percent_for_zsh() {
        assert!(prompt_name("100%", None, PromptShell::Zsh).contains("100%%"));
    }

    #[test]
    fn include_if_must_match_exact_directory() {
        let cfg =
            "[includeIf \"gitdir:~/code/site/deploy/\"]\n\tpath = ~/code/site/deploy/.gitconfig\n";
        assert!(has_include_if(cfg, &["~/code/site/deploy"]));
        assert!(!has_include_if(cfg, &["~/code/site"]));
    }

    #[test]
    fn include_if_accepts_case_insensitive_form() {
        let cfg = "[includeIf \"gitdir/i:/Users/a/work/\"]\n";
        assert!(has_include_if(cfg, &["~/work", "/Users/a/work"]));
    }
}
