# realm

[![Crates.io](https://img.shields.io/crates/v/realm-cli)](https://crates.io/crates/realm-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/yusukeshib/realm/actions/workflows/ci.yml/badge.svg)](https://github.com/yusukeshib/realm/actions/workflows/ci.yml)

Sandboxed Docker environments for git repos — safe playgrounds for AI coding agents.

![demo](./docs/demo.gif)

## My usage

```sh
> ls -al | grep Dockerfile
.rw-r--r--@ 2.2k yusuke  8 Feb 10:42 Dockerfile

# build my own image for sandbox
> docker build -t mydev

# run
> docker run -it mydev

# process your auth of claude
% claude
% exit

# save image for my auth action for claude
> docker commit <container_id> mydev:authorized

# put my dev image for realm in .zshrc
> export REALM_DEFAULT_IMAGE=mydev:authorized

# Then, you can use realm for claude conveniently
> realm new-quality-improvement

# you can immediately run claude
% claude


```

## Why realm?

AI coding agents (Claude Code, Cursor, Copilot) are powerful — but letting them loose on your actual working tree is risky. Realm gives them a **safe, isolated sandbox** where they can go wild without consequences.

- **Your code stays safe** — an independent clone is used, host files are never modified
- **AI agents can experiment freely** — commit, branch, rewrite, break things — your working tree is untouched
- **Persistent sessions** — exit and resume where you left off, files are preserved
- **Named sessions** — run multiple experiments in parallel
- **Bring your own toolchain** — works with any Docker image

## Quick Start

```bash
curl -fsSL https://raw.githubusercontent.com/yusukeshib/realm/main/install.sh | bash
realm my-feature --image ubuntu:latest -- bash
# You're now in an isolated container with full git access
```

## Claude Code Integration

Realm is the ideal companion for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Run Claude Code inside a realm session and let it make risky changes, experiment with branches, and run tests — all fully isolated from your host.

```bash
realm ai-experiment --image node:20 -- claude
```

Everything the agent does stays inside the container. When you're done, delete the session and it's gone.

## Install

### Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/yusukeshib/realm/main/install.sh | bash
```

### From crates.io

```bash
cargo install realm-cli
```

### From source

```bash
cargo install --git https://github.com/yusukeshib/realm
```

### Nix

```bash
nix run github:yusukeshib/realm
```

### Binary download

Pre-built binaries are available on the [GitHub Releases](https://github.com/yusukeshib/realm/releases) page.

## Usage

```bash
realm                                               List all sessions (TUI)
realm <name> [options] [-- cmd...]                  Create or resume a session
realm <name> -d                                     Delete a session
realm upgrade                                       Upgrade to latest version
```

### Create or resume a session

```bash
# Default: alpine/git image, sh shell, current directory
realm my-feature

# Specify a project directory (only used when creating)
realm my-feature --dir ~/projects/my-app

# Custom image with bash (only used when creating)
realm my-feature --image ubuntu:latest -- bash

# Custom mount path inside container (only used when creating)
realm my-feature --mount /src

# Environment variables (only used when creating)
realm my-feature -e KEY=VALUE -e ANOTHER_KEY

# If session exists, it resumes with original configuration
# If session doesn't exist, it creates a new one
realm my-feature
```

Sessions are automatically created if they don't exist. If a session already exists, create-time options like `--image`, `--mount`, `--dir`, and `-e` are ignored when resuming.

### List sessions

```bash
realm
```

```
NAME                 PROJECT                        IMAGE                CREATED
----                 -------                        -----                -------
my-feature           /Users/you/projects/app        alpine/git           2026-02-07 12:00:00 UTC
test                 /Users/you/projects/other      ubuntu:latest        2026-02-07 12:30:00 UTC
```

### Delete a session

```bash
realm my-feature -d
```

## Options

| Option | Description |
|--------|-------------|
| `-d` | Delete the session |
| `--image <image>` | Docker image to use (default: `alpine/git`) - only used when creating |
| `--mount <path>` | Mount path inside the container (default: `/workspace`) - only used when creating |
| `--dir <path>` | Project directory (default: current directory) - only used when creating |
| `-e, --env <KEY[=VALUE]>` | Environment variable to pass to container - only used when creating |
| `--no-ssh` | Disable SSH agent forwarding (enabled by default) - only used when creating |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `REALM_DEFAULT_IMAGE` | Default Docker image for new sessions (default: `alpine/git`) |
| `REALM_DOCKER_ARGS` | Extra Docker flags (e.g., `--network host`, additional `-v` mounts) |

```bash
# Pass extra Docker flags
REALM_DOCKER_ARGS="--network host -v /data:/data:ro" realm my-session -c
```

## How It Works

On first run, `git clone --local` creates an independent copy of your repo in the workspace directory. The container gets a fully self-contained git repo — no special mounts or entrypoint scripts needed. Your host working directory is never modified.

- **Independent clone** — Each session gets its own complete git repo via `git clone --local`
- **Persistent workspace** — Files survive `exit` and `realm <name>` resume; cleaned up on `realm <name> -d`
- **Any image, any user** — Works with root and non-root container images

| Aspect | Protection |
|--------|------------|
| Host working tree | Never modified — workspace is an independent clone |
| Workspace | Bind-mounted from `~/.realm/workspaces/<name>/`, persists across stop/start |
| Session cleanup | `realm <name> -d` removes container, workspace, and session data |

## SSH Agent Forwarding

SSH agent forwarding is enabled by default. This lets you use `git clone`, `ssh`, and other tools that rely on your SSH keys without copying them into the container.

```bash
realm my-feature --image ubuntu:latest -- bash

# Inside the container
ssh-add -l          # should list your keys
git clone git@github.com:user/repo.git

# To disable SSH forwarding
realm my-feature --no-ssh -- bash
```

On **macOS** (Docker Desktop / OrbStack), the socket at `/run/host-services/ssh-auth.sock` is mounted automatically. On **Linux**, the `$SSH_AUTH_SOCK` environment variable is used.

## License

MIT


