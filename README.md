# realm

[![Crates.io](https://img.shields.io/crates/v/realm-cli)](https://crates.io/crates/realm-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/yusukeshib/realm/actions/workflows/ci.yml/badge.svg)](https://github.com/yusukeshib/realm/actions/workflows/ci.yml)

Sandboxed Docker environments for git repos — safe playgrounds for AI coding agents.

![demo](./docs/demo.gif)

## Why realm?

AI coding agents (Claude Code, Cursor, Copilot) are powerful — but letting them loose on your actual working tree is risky. Realm gives them a **safe, isolated sandbox** where they can go wild without consequences.

- **Your code stays safe** — only `.git` is mounted, host files are never modified
- **AI agents can experiment freely** — commit, branch, rewrite, break things — your working tree is untouched
- **Zero cleanup** — the container is destroyed on exit
- **Named sessions** — resume where you left off, run multiple experiments in parallel
- **Bring your own toolchain** — works with any Docker image

## Quick Start

```bash
curl -fsSL https://raw.githubusercontent.com/yusukeshib/realm/main/install.sh | bash
realm my-feature -c --image ubuntu:latest -- bash
# You're now in an isolated container with full git access
```

## Claude Code Integration

Realm is the ideal companion for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Run Claude Code inside a realm session and let it make risky changes, experiment with branches, and run tests — all fully isolated from your host.

```bash
realm ai-experiment -c --image node:20 -- claude
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
realm <name> [-- cmd...]                            Resume a session
realm <name> -c [options] [-- cmd...]               Create a new session
realm <name> -d                                     Delete a session
realm upgrade                                       Upgrade to latest version
```

### Create a session

```bash
# Default: alpine/git image, sh shell, current directory
realm my-feature -c

# Specify a project directory
realm my-feature -c --dir ~/projects/my-app

# Custom image with bash
realm my-feature -c --image ubuntu:latest -- bash

# Custom mount path inside container
realm my-feature -c --mount /src

# -c flag works in any position
realm -c my-feature --image ubuntu:latest -- bash
```

### Resume a session

```bash
realm my-feature
```

The container resumes with the same configuration from the original session.

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
| `-c` | Create a new session |
| `-d` | Delete the session |
| `--image <image>` | Docker image to use (default: `alpine/git`) |
| `--mount <path>` | Mount path inside the container (default: `/workspace`) |
| `--dir <path>` | Project directory (default: current directory) |

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

Realm mounts your repo's `.git` directory into a Docker container. Your host working directory is never modified.

- **`.git`-only mount** — The container gets full git functionality (commit, branch, diff) without touching your working tree
- **Session isolation** — Each session works independently inside the container
- **Host stays clean** — After container exit, realm runs `git reset` to fix the host index

| Aspect | Protection |
|--------|------------|
| Host working tree | Never modified — only `.git` is mounted |
| Git data | Container works on mounted `.git` only |
| Container | Destroyed after each exit (`--rm`) |
| Host index | Restored via `git reset` after container exit |

## License

MIT
