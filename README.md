# git-whoami

Verify which Git identity is active, and whether it matches the [git-persona](https://github.com/uRadical/git-persona-cli) persona applied to the current directory.

`git-persona status` tells you which persona *should* be active. `git whoami` asks **git itself** what it will actually use and checks that the two agree. Use it to catch the cases where they don't:

- you're outside a git repository, so `includeIf "gitdir:..."` rules don't apply
- the `includeIf` entry is missing from `~/.gitconfig`
- a repo-local `.git/config` overrides `user.email`
- `GIT_AUTHOR_EMAIL` or another environment variable overrides the config
- the SSH key or GPG signing setup doesn't match the persona
- the SSH key authenticates as the wrong account on GitHub or GitLab (`--ssh`)

## Installation

### From releases

Download the latest release for your platform from the [releases page](https://github.com/uRadical/git-whoami/releases).

### From crates.io

```bash
cargo install git-whoami-cli
```

### From source

```bash
cargo install --git https://github.com/uRadical/git-whoami
```

## Usage

The binary is named `git-whoami`, so git runs it as a subcommand. By default it prints just the persona name, in the persona's color when output is a terminal:

```bash
cd ~/code/work-project
git whoami
# work
```

If git isn't actually using that persona, the name is still printed but a warning goes to stderr and the exit code is `1`. Where no persona is applied, it falls back to the email git will commit with, uncolored so it reads differently from a persona name. Outside a git repository there's nothing to commit, so it prints nothing.

Use `-v` / `--verbose` for the full report:

```bash
git whoami -v
# Persona    work (~/code/work-project)
# Identity   Alan <alan@company.com>
# Source     ~/code/work-project/.gitconfig
# SSH        ssh -i /Users/alan/.ssh/work -o IdentitiesOnly=yes
#
# ✓ Identity matches persona 'work'
```

When git resolves something else, the differences are listed along with likely causes:

```
✗ Identity does not match persona 'work'
    author: expected Alan <alan@company.com>, got Alan <alan@home.com>
    committer: expected Alan <alan@company.com>, got Alan <alan@home.com>
  hint: not inside a git repository: includeIf "gitdir:" rules only apply within a repo
```

To check another directory, use `git -C <path> whoami`.

### Check the SSH account

`--ssh` connects to the remote with the effective `core.sshCommand` and reports the account the host says you are:

```bash
git whoami --ssh
# Persona    work (~/code/work-project)
# ...
# Remote     alan-work @ github.com
```

It recognises GitHub, GitLab, Bitbucket and Gitea/Codeberg greetings, and uses `origin` unless you pass `--remote <name>`. A failed authentication counts as a mismatch. `--ssh` always shows the full report.

### Shell prompt

`--prompt <bash|zsh|fish|oh-my-posh>` prints the persona name in its color, with the escape codes wrapped so the shell measures the prompt width correctly. Without a persona it prints the fallback email inside a repository and nothing elsewhere. It never writes to stderr.

```bash
# zsh (~/.zshrc)
setopt PROMPT_SUBST
PROMPT='%~ $(git whoami --prompt zsh)%# '

# bash (~/.bashrc)
PS1='\w $(git whoami --prompt bash)\$ '
```

```fish
# fish (~/.config/fish/config.fish)
function fish_prompt
    echo -n (prompt_pwd) (git whoami --prompt fish)'> '
end
```

#### oh-my-posh

oh-my-posh no longer has a command segment, so export the persona from its `set_poshcontext` hook and read it in a `text` segment. `--prompt oh-my-posh` prints oh-my-posh color tags (`<blue>work</>`).

```zsh
# ~/.zshrc, after `eval "$(oh-my-posh init zsh ...)"`
function set_poshcontext() {
  export GIT_PERSONA="$(git whoami --prompt oh-my-posh)"
}
```

```json
{
  "type": "text",
  "style": "plain",
  "template": "{{ if .Env.GIT_PERSONA }}--[{{ .Env.GIT_PERSONA }}]{{ end }}"
}
```

`set_poshcontext` runs after oh-my-posh has captured the last exit status, so the status segment is unaffected. The segment shows the fallback email in a repository with no persona, and is hidden outside repositories.

For plain text, use `$(git whoami)`; when output isn't a terminal, color and the stderr notes are turned off automatically, so it prints the name, the fallback email, or nothing. `NO_COLOR` is respected.

### Options

| Flag | Description |
| --- | --- |
| `-v`, `--verbose` | Full identity report with sources and mismatch details |
| `--prompt <shell>` | Colored persona name for a `bash`, `zsh`, `fish` or `oh-my-posh` prompt |
| `-q`, `--quiet` | No output; exit code only |
| `--json` | Machine-readable report |
| `-r`, `--require-persona` | Fail when no persona is applied here |
| `--ssh` | Check which account the SSH key authenticates as |
| `--remote <name>` | Remote for `--ssh` (default `origin`) |

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Identity matches the applied persona, or no persona is applied and an identity is configured |
| `1` | Mismatch, no identity configured, SSH authentication failed, or an error occurred |

With `--require-persona`, "no persona applied" also exits `1`.

### Pre-commit hook

Block commits made with the wrong identity:

```bash
#!/bin/sh
# .git/hooks/pre-commit
git whoami --quiet || { git whoami -v; exit 1; }
```

## Configuration

git-whoami reads the git-persona config from `~/.config/git-persona/config.json` (respects `XDG_CONFIG_HOME`) and never modifies it or any git config.

## License

MIT
