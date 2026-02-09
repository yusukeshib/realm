# realm

[日本語](README.ja.md)

[![Crates.io](https://img.shields.io/crates/v/realm-cli)](https://crates.io/crates/realm-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/yusukeshib/realm/actions/workflows/ci.yml/badge.svg)](https://github.com/yusukeshib/realm/actions/workflows/ci.yml)

Sandboxed Docker environments for git repos — safe playgrounds for AI coding agents.

![demo](./demo.gif)

## Why realm?

AI coding agents (Claude Code, Cursor, Copilot) are powerful — but letting them loose on your actual working tree is risky. Realm gives them a **safe, isolated sandbox** where they can go wild without consequences.

- **Your code stays safe** — an independent clone is used, host files are never modified
- **AI agents can experiment freely** — commit, branch, rewrite, break things — your working tree is untouched
- **Persistent sessions** — exit and resume where you left off, files are preserved
- **Named sessions** — run multiple experiments in parallel
- **Bring your own toolchain** — works with any Docker image

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

## Quick Start

```bash
curl -fsSL https://raw.githubusercontent.com/yusukeshib/realm/main/install.sh | bash
realm my-feature --image ubuntu:latest -- bash
# You're now in an isolated container with full git access
```

Realm must be run inside a git repository — it clones the current repo into the container.

For a zero-flags workflow, see [Custom Image Setup](#custom-image-setup) below.

## Custom Image Setup

The recommended way to use realm: build your image once, set a couple of env vars, and never pass flags again.

**1. Create a Dockerfile with your toolchain**

Include whatever tools your workflow needs (languages, runtimes, CLI tools, etc.).

**2. Build the image**

```bash
docker build -t mydev .
```

**3. Run interactively to configure auth/state**

Some tools (like Claude Code) need interactive authentication that can't be baked into a Dockerfile. Run the image, complete the setup, then save the result:

```bash
docker run -it mydev
# Inside the container: run `claude` to authenticate, install extras, etc.
# When done:
exit
```

```bash
# Find the container ID and commit the configured state
docker commit <container_id> mydev:ready
```

**4. Set environment variables**

Add these to your `.zshrc` or `.bashrc`:

```bash
export REALM_DEFAULT_IMAGE=mydev:ready        # your custom image
export REALM_DOCKER_ARGS="--network host"     # any extra Docker flags you always want
```

**5. Done — just use realm**

With those env vars set, every session uses your custom image with zero flags:

```bash
# That's it. From now on:
realm feature-1
realm bugfix-auth
realm experiment-v2
# Each gets an isolated sandbox with your full toolchain.
```

## Usage

```bash
realm                                               List all sessions (TUI)
realm <name> [options] [-- cmd...]                  Create or resume a session
realm <name> -d [-- cmd...]                         Run detached (background)
realm <name>                                        Attach to running session
realm <name> --delete                               Delete a session
realm upgrade                                       Upgrade to latest version
```

### Create or resume a session

```bash
# Default: alpine:latest image, sh shell, current directory
realm my-feature

# Custom image with bash (only used when creating)
realm my-feature --image ubuntu:latest -- bash

# Extra Docker flags (env vars, volumes, network, etc.)
realm my-feature --docker-args "-e KEY=VALUE -v /host:/container --network host"

# If session exists, it resumes with original configuration
# If session doesn't exist, it creates a new one
realm my-feature
```

Sessions are automatically created if they don't exist. If a session already exists, create-time options like `--image` are ignored when resuming. Runtime options like `--docker-args` and `--no-ssh` apply on every run.

### Detach mode

```bash
# Run in background
realm my-feature -d -- claude -p "do something"

# Attach to a running session
realm my-feature

# Detach without stopping: Ctrl+P, Ctrl+Q
```

### List sessions

```bash
realm
```

```
NAME                 PROJECT                        IMAGE                CREATED
----                 -------                        -----                -------
my-feature           /Users/you/projects/app        alpine:latest           2026-02-07 12:00:00 UTC
test                 /Users/you/projects/other      ubuntu:latest        2026-02-07 12:30:00 UTC
```

### Delete a session

```bash
realm my-feature --delete
```

## Options

| Option | Description |
|--------|-------------|
| `-d` | Run container in the background (detached) |
| `--delete` | Delete the session |
| `--image <image>` | Docker image to use (default: `alpine:latest`) - only used when creating |
| `--docker-args <args>` | Extra Docker flags (e.g. `-e KEY=VALUE`, `-v /host:/container`). Overrides `$REALM_DOCKER_ARGS` |
| `--no-ssh` | Disable SSH agent forwarding (enabled by default) |

## Environment Variables

These let you configure defaults so you can skip CLI flags entirely. Set them in your `.zshrc` or `.bashrc` and every `realm <name>` invocation uses them automatically.

| Variable | Description |
|----------|-------------|
| `REALM_DEFAULT_IMAGE` | Default Docker image for new sessions (default: `alpine:latest`) |
| `REALM_DOCKER_ARGS` | Default extra Docker flags, used when `--docker-args` is not provided |

```bash
# Set default Docker flags for all sessions
export REALM_DOCKER_ARGS="--network host -v /data:/data:ro"
realm my-session

# Override with --docker-args for a specific session
realm my-session --docker-args "-e DEBUG=1"
```

## How It Works

On first run, `git clone --local` creates an independent copy of your repo in the workspace directory. The container gets a fully self-contained git repo — no special mounts or entrypoint scripts needed. Your host working directory is never modified.

- **Independent clone** — Each session gets its own complete git repo via `git clone --local`
- **Persistent workspace** — Files survive `exit` and `realm <name>` resume; cleaned up on `realm <name> --delete`
- **Any image, any user** — Works with root and non-root container images

| Aspect | Protection |
|--------|------------|
| Host working tree | Never modified — workspace is an independent clone |
| Workspace | Bind-mounted from `~/.realm/workspaces/<name>/`, persists across stop/start |
| Session cleanup | `realm <name> --delete` removes container, workspace, and session data |

## Design Decisions

### Why `git clone --local`?

Several git isolation strategies exist — here's why the alternatives fall short:

| Strategy | Problem |
|----------|---------|
| **Bind-mount the host repo** | No isolation at all; the agent modifies your actual files |
| **git worktree** | Shares the `.git` directory with the host; checkout, reset, and rebase can affect host branches and refs |
| **Bare-git mount** | Still shares state; branch creates/deletes in the container affect the host |
| **Branch-only isolation** | Nothing stops the agent from checking out other branches or running destructive git commands on shared refs |
| **Full copy (`cp -r`)** | Truly isolated but slow for large repos |
| **Docker sandbox (`--sandbox`)** | Opaque — no control over image, no persistence, no SSH forwarding, no custom Docker args (see [below](#why-plain-docker-not---sandbox)) |

`git clone --local` wins because it's:

- **Fully independent** — the clone has its own `.git`; nothing in the container can touch the host repo
- **Fast** — hardlinks file objects on the same filesystem instead of copying
- **Complete** — full history, all branches, standard git repo
- **Simple** — no wrapper scripts or special entrypoints needed

### Why plain Docker (not `--sandbox`)?

Claude Code's built-in `--sandbox` mode wraps Docker, but trades flexibility for convenience:

- **Opaque** — you don't control the base image, installed packages, or container setup
- **Not flexible** — can't bring your own toolchain, runtime, or dev environment
- **No persistence** — no exit-and-resume; each run starts fresh
- **No SSH forwarding** — git push/pull with SSH keys doesn't work out of the box
- **No custom Docker args** — can't configure network, extra volumes, or env vars
- **Single-agent** — tied to Claude Code; realm works with any agent or manual use

Plain Docker gives full control while realm handles the isolation and lifecycle.

## SSH Agent Forwarding

**The problem**: Docker containers can't normally access your host SSH keys. On macOS it's even harder — Docker runs in a VM, so Unix sockets can't cross the VM boundary.

**What realm does**: Automatically forwards the host's SSH agent into the container. `git push`, `git pull`, and `ssh` all work with your existing keys — no key copying needed.

**How it works per platform**:

- **macOS** (Docker Desktop / OrbStack): Mounts the VM-bridged socket at `/run/host-services/ssh-auth.sock`
- **Linux**: Mounts `$SSH_AUTH_SOCK` directly into the container

Realm also re-points the cloned repo's `origin` remote to the real URL (not the local clone path), so `git push origin` works out of the box.

```bash
realm my-feature --image ubuntu:latest -- bash

# Inside the container
ssh-add -l          # should list your keys
git push origin main

# To disable SSH forwarding
realm my-feature --no-ssh -- bash
```

## Security Note

The `--docker-args` flag and `REALM_DOCKER_ARGS` environment variable pass arguments directly to `docker run`. This means flags like `--privileged`, `--pid=host`, or `-v /:/host` can weaken or bypass container sandboxing. Only use trusted values and be careful when sourcing `REALM_DOCKER_ARGS` from shared or automated environments.

## Claude Code Integration

Realm is the ideal companion for [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Run Claude Code inside a realm session and let it make risky changes, experiment with branches, and run tests — all fully isolated from your host.

```bash
realm ai-experiment --image node:20 -- claude
```

Run in the background with detach mode:

```bash
realm ai-experiment -d --image node:20 -- claude -p "refactor the auth module"
```

Everything the agent does stays inside the container. When you're done, delete the session and it's gone.

## License

MIT
