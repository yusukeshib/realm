mod config;
mod docker;
mod git;
mod session;
mod tui;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Parser)]
#[command(
    name = "box",
    about = "Sandboxed Docker environments for git repos",
    after_help = "Examples:\n  box                                         # interactive session manager\n  box my-feature                               # shortcut for `box create my-feature`\n  box create my-feature                        # create a new session\n  box create my-feature --image ubuntu -- bash # create with options\n  box resume my-feature                        # resume a session\n  box resume my-feature -d                     # resume in background\n  box stop my-feature                          # stop a running session\n  box exec my-feature -- ls -la                # run a command in a session\n  box list                                     # list all sessions\n  box list -q --running                        # names of running sessions\n  box remove my-feature                        # remove a session\n  box cd my-feature                            # print project directory\n  box path my-feature                          # print workspace path\n  box upgrade                                  # self-update"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new session
    Create(CreateArgs),
    /// Resume an existing session
    Resume(ResumeArgs),
    /// Remove a session (must be stopped first)
    Remove(RemoveArgs),
    /// Stop a running session
    Stop(StopArgs),
    /// Run a command in a running session
    Exec(ExecArgs),
    /// List sessions
    #[command(alias = "ls")]
    List(ListArgs),
    /// Print the host project directory for a session
    Cd {
        /// Session name
        name: String,
    },
    /// Print workspace path for a session
    Path {
        /// Session name
        name: String,
    },
    /// Self-update to the latest version
    Upgrade,
    /// Output shell configuration (e.g. eval "$(box config zsh)")
    Config {
        #[command(subcommand)]
        shell: ConfigShell,
    },
    /// Shortcut: `box <name>` is equivalent to `box create <name>`
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

#[derive(clap::Args, Debug)]
struct CreateArgs {
    /// Session name
    name: String,

    /// Run container in the background (detached)
    #[arg(short = 'd')]
    detach: bool,

    /// Docker image to use (default: $BOX_DEFAULT_IMAGE or alpine:latest)
    #[arg(long)]
    image: Option<String>,

    /// Extra Docker flags (e.g. -e KEY=VALUE, -v /host:/container, --network host).
    /// Overrides $BOX_DOCKER_ARGS when provided.
    #[arg(long = "docker-args", allow_hyphen_values = true)]
    docker_args: Option<String>,

    /// Disable SSH agent forwarding (enabled by default)
    #[arg(long = "no-ssh")]
    no_ssh: bool,

    /// Command to run in container (default: $BOX_DEFAULT_CMD if set)
    #[arg(last = true)]
    cmd: Vec<String>,
}

#[derive(clap::Args, Debug)]
struct ResumeArgs {
    /// Session name
    name: String,

    /// Run container in the background (detached)
    #[arg(short = 'd')]
    detach: bool,

    /// Extra Docker flags (e.g. -e KEY=VALUE, -v /host:/container, --network host).
    /// Overrides $BOX_DOCKER_ARGS when provided.
    #[arg(long = "docker-args", allow_hyphen_values = true)]
    docker_args: Option<String>,
}

#[derive(clap::Args, Debug)]
struct RemoveArgs {
    /// Session name
    name: String,
}

#[derive(clap::Args, Debug)]
struct StopArgs {
    /// Session name
    name: String,
}

#[derive(clap::Args, Debug)]
struct ExecArgs {
    /// Session name
    name: String,

    /// Command to run in the container
    #[arg(last = true, required = true)]
    cmd: Vec<String>,
}

#[derive(clap::Args, Debug)]
struct ListArgs {
    /// Show only running sessions
    #[arg(long, short)]
    running: bool,
    /// Show only stopped sessions
    #[arg(long, short)]
    stopped: bool,
    /// Only print session names
    #[arg(long, short)]
    quiet: bool,
}

#[derive(Subcommand, Debug)]
enum ConfigShell {
    /// Output Zsh completions
    Zsh,
    /// Output Bash completions
    Bash,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::Create(args)) => {
            let docker_args = args
                .docker_args
                .or_else(|| std::env::var("BOX_DOCKER_ARGS").ok())
                .unwrap_or_default();
            let cmd = if args.cmd.is_empty() {
                None
            } else {
                Some(args.cmd)
            };
            cmd_create(
                &args.name,
                args.image,
                &docker_args,
                cmd,
                !args.no_ssh,
                args.detach,
            )
        }
        Some(Commands::Resume(args)) => {
            let docker_args = args
                .docker_args
                .or_else(|| std::env::var("BOX_DOCKER_ARGS").ok())
                .unwrap_or_default();
            cmd_resume(&args.name, &docker_args, args.detach)
        }
        Some(Commands::Remove(args)) => cmd_remove(&args.name),
        Some(Commands::Stop(args)) => cmd_stop(&args.name),
        Some(Commands::Exec(args)) => cmd_exec(&args.name, &args.cmd),
        Some(Commands::List(args)) => cmd_list_sessions(&args),
        Some(Commands::Cd { name }) => cmd_cd(&name),
        Some(Commands::Path { name }) => cmd_path(&name),
        Some(Commands::Upgrade) => cmd_upgrade(),
        Some(Commands::Config { shell }) => match shell {
            ConfigShell::Zsh => cmd_config_zsh(),
            ConfigShell::Bash => cmd_config_bash(),
        },
        Some(Commands::External(args)) => {
            let name = args[0].to_string_lossy().to_string();
            let docker_args = std::env::var("BOX_DOCKER_ARGS").unwrap_or_default();
            if session::session_exists(&name).unwrap_or(false) {
                cmd_resume(&name, &docker_args, false)
            } else {
                let cmd: Vec<String> = args[1..]
                    .iter()
                    .skip_while(|a| *a != "--")
                    .skip(1)
                    .map(|a| a.to_string_lossy().to_string())
                    .collect();
                let cmd = if cmd.is_empty() { None } else { Some(cmd) };
                cmd_create(&name, None, &docker_args, cmd, true, false)
            }
        }
        None => cmd_list(),
    };

    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn output_cd_path(path: &str) {
    if let Ok(cd_file) = std::env::var("BOX_CD_FILE") {
        let _ = fs::write(cd_file, path);
    } else {
        println!("{}", path);
    }
}

fn cmd_list() -> Result<i32> {
    let mut sessions = session::list()?;

    docker::check()?;
    let running = docker::running_sessions();
    for s in &mut sessions {
        s.running = running.contains(&s.name);
    }

    let delete_fn = |name: &str| -> Result<()> {
        docker::remove_container(name);
        docker::remove_workspace(name);
        session::remove_dir(name)?;
        Ok(())
    };

    let docker_args = std::env::var("BOX_DOCKER_ARGS").unwrap_or_default();

    match tui::session_manager(&sessions, delete_fn)? {
        tui::TuiAction::Resume(name) => cmd_resume(&name, &docker_args, false),
        tui::TuiAction::New {
            name,
            image,
            command,
        } => cmd_create(&name, image, &docker_args, command, true, false),
        tui::TuiAction::Cd(name) => cmd_cd(&name),
        tui::TuiAction::Quit => Ok(0),
    }
}

fn cmd_list_sessions(args: &ListArgs) -> Result<i32> {
    let mut sessions = session::list()?;

    docker::check()?;
    let running = docker::running_sessions();
    for s in &mut sessions {
        s.running = running.contains(&s.name);
    }

    if args.running {
        sessions.retain(|s| s.running);
    }
    if args.stopped {
        sessions.retain(|s| !s.running);
    }

    if args.quiet {
        for s in &sessions {
            println!("{}", s.name);
        }
        return Ok(0);
    }

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(0);
    }

    let home = config::home_dir().unwrap_or_default();

    // Compute column widths
    let name_w = sessions
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(0)
        .max(4);
    let status_w = 7; // "running" or "stopped"
    let image_w = sessions
        .iter()
        .map(|s| s.image.len())
        .max()
        .unwrap_or(0)
        .max(5);

    let shorten_home = |p: &str| -> String {
        if !home.is_empty() {
            if let Some(rest) = p.strip_prefix(&home) {
                return format!("~{}", rest);
            }
        }
        p.to_string()
    };

    let project_w = sessions
        .iter()
        .map(|s| shorten_home(&s.project_dir).len())
        .max()
        .unwrap_or(0)
        .max(7);
    let command_w = sessions
        .iter()
        .map(|s| s.command.len())
        .max()
        .unwrap_or(0)
        .max(7);

    println!(
        "{:<name_w$}  {:<status_w$}  {:<image_w$}  {:<project_w$}  {:<command_w$}  CREATED",
        "NAME", "STATUS", "IMAGE", "PROJECT", "COMMAND",
    );

    for s in &sessions {
        let status = if s.running { "running" } else { "stopped" };
        let project = shorten_home(&s.project_dir);
        println!(
            "{:<name_w$}  {:<status_w$}  {:<image_w$}  {:<project_w$}  {:<command_w$}  {}",
            s.name, status, s.image, project, s.command, s.created_at,
        );
    }

    Ok(0)
}

fn cmd_create(
    name: &str,
    image: Option<String>,
    docker_args: &str,
    cmd: Option<Vec<String>>,
    ssh: bool,
    detach: bool,
) -> Result<i32> {
    session::validate_name(name)?;

    if session::session_exists(name)? {
        bail!(
            "Session '{}' already exists. Use `box resume {}` to resume it.",
            name,
            name
        );
    }

    let cwd =
        fs::canonicalize(".").map_err(|_| anyhow::anyhow!("Cannot resolve current directory."))?;

    let project_dir = git::find_root(&cwd)
        .ok_or_else(|| anyhow::anyhow!("'{}' is not inside a git repository.", cwd.display()))?
        .to_string_lossy()
        .to_string();

    docker::check()?;

    let cfg = config::resolve(config::BoxConfigInput {
        name: name.to_string(),
        image,
        mount_path: None,
        project_dir,
        command: cmd,
        env: vec![],
        ssh,
    })?;

    eprintln!("\x1b[2msession:\x1b[0m {}", cfg.name);
    eprintln!("\x1b[2mimage:\x1b[0m {}", cfg.image);
    eprintln!("\x1b[2mmount:\x1b[0m {}", cfg.mount_path);
    if cfg.ssh {
        eprintln!("\x1b[2mssh:\x1b[0m true");
    }
    if !cfg.command.is_empty() {
        eprintln!("\x1b[2mcommand:\x1b[0m {}", shell_words::join(&cfg.command));
    }
    if !docker_args.is_empty() {
        eprintln!("\x1b[2mdocker args:\x1b[0m {}", docker_args);
    }
    eprintln!();

    let sess = session::Session::from(cfg);
    session::save(&sess)?;

    let home = config::home_dir()?;
    let docker_args_opt = if docker_args.is_empty() {
        None
    } else {
        Some(docker_args)
    };

    docker::remove_container(name);
    docker::run_container(&docker::DockerRunConfig {
        name,
        project_dir: &sess.project_dir,
        image: &sess.image,
        mount_path: &sess.mount_path,
        cmd: &sess.command,
        env: &sess.env,
        home: &home,
        docker_args: docker_args_opt,
        ssh: sess.ssh,
        detach,
    })
}

fn cmd_resume(name: &str, docker_args: &str, detach: bool) -> Result<i32> {
    session::validate_name(name)?;

    let sess = session::load(name)?;

    if !Path::new(&sess.project_dir).is_dir() {
        bail!("Project directory '{}' no longer exists.", sess.project_dir);
    }

    docker::check()?;

    if docker::container_is_running(name) {
        if detach {
            println!("Session '{}' is already running.", name);
            return Ok(0);
        }
        return docker::attach_container(name);
    }

    println!("Resuming session '{}'...", name);
    session::touch_resumed_at(name)?;

    if docker::container_exists(name) {
        if detach {
            docker::start_container_detached(name)
        } else {
            docker::start_container(name)
        }
    } else {
        let home = config::home_dir()?;
        let docker_args_opt = if docker_args.is_empty() {
            None
        } else {
            Some(docker_args)
        };

        docker::remove_container(name);
        docker::run_container(&docker::DockerRunConfig {
            name,
            project_dir: &sess.project_dir,
            image: &sess.image,
            mount_path: &sess.mount_path,
            cmd: &sess.command,
            env: &sess.env,
            home: &home,
            docker_args: docker_args_opt,
            ssh: sess.ssh,
            detach,
        })
    }
}

fn cmd_remove(name: &str) -> Result<i32> {
    session::validate_name(name)?;

    if !session::session_exists(name)? {
        bail!("Session '{}' not found.", name);
    }

    docker::check()?;

    if docker::container_is_running(name) {
        bail!(
            "Session '{}' is still running. Stop it first with `box stop {}`.",
            name,
            name
        );
    }

    docker::remove_container(name);
    docker::remove_workspace(name);
    session::remove_dir(name)?;

    println!("Session '{}' removed.", name);
    Ok(0)
}

fn cmd_stop(name: &str) -> Result<i32> {
    session::validate_name(name)?;

    if !session::session_exists(name)? {
        bail!("Session '{}' not found.", name);
    }

    docker::check()?;

    if !docker::container_is_running(name) {
        bail!("Session '{}' is not running.", name);
    }

    docker::stop_container(name)
}

fn cmd_exec(name: &str, cmd: &[String]) -> Result<i32> {
    session::validate_name(name)?;

    if !session::session_exists(name)? {
        bail!("Session '{}' not found.", name);
    }

    docker::check()?;

    if !docker::container_is_running(name) {
        bail!("Session '{}' is not running.", name);
    }

    docker::exec_container(name, cmd)
}

fn cmd_cd(name: &str) -> Result<i32> {
    session::validate_name(name)?;
    if !session::session_exists(name)? {
        bail!("Session '{}' not found.", name);
    }
    let home = config::home_dir()?;
    let path = Path::new(&home).join(".box").join("workspaces").join(name);
    output_cd_path(&path.to_string_lossy());
    Ok(0)
}

fn cmd_path(name: &str) -> Result<i32> {
    session::validate_name(name)?;
    if !session::session_exists(name)? {
        bail!("Session '{}' not found.", name);
    }
    let home = config::home_dir()?;
    let path = Path::new(&home).join(".box").join("workspaces").join(name);
    println!("{}", path.display());
    Ok(0)
}

fn cmd_config_zsh() -> Result<i32> {
    print!(
        r#"__box_sessions() {{
    local -a sessions
    if [[ -d "$HOME/.box/sessions" ]]; then
        for s in "$HOME/.box/sessions"/*(N:t); do
            local desc=""
            if [[ -f "$HOME/.box/sessions/$s/project_dir" ]]; then
                desc=$(< "$HOME/.box/sessions/$s/project_dir")
                desc=${{desc/#$HOME/\~}}
            fi
            sessions+=("$s:[$desc]")
        done
    fi
    if (( ${{#sessions}} )); then
        _describe 'session' sessions
    fi
}}

_box() {{
    local curcontext="$curcontext" state line
    typeset -A opt_args

    _arguments -C \
        '1: :->subcmd' \
        '*:: :->args'

    case $state in
        subcmd)
            __box_sessions
            ;;
        args)
            case $words[1] in
                create)
                    _arguments \
                        '-d[Run container in the background]' \
                        '--image=[Docker image to use]:image' \
                        '--docker-args=[Extra Docker flags]:args' \
                        '--no-ssh[Disable SSH agent forwarding]' \
                        '1:session name:' \
                        '*:command:'
                    ;;
                resume)
                    _arguments \
                        '-d[Run container in the background]' \
                        '--docker-args=[Extra Docker flags]:args' \
                        '1:session name:__box_sessions'
                    ;;
                exec)
                    _arguments \
                        '1:session name:__box_sessions' \
                        '*:command:'
                    ;;
                list|ls)
                    _arguments \
                        '--running[Show only running sessions]' \
                        '-r[Show only running sessions]' \
                        '--stopped[Show only stopped sessions]' \
                        '-s[Show only stopped sessions]' \
                        '--quiet[Only print session names]' \
                        '-q[Only print session names]'
                    ;;
                remove|stop|path|cd)
                    if (( CURRENT == 2 )); then
                        __box_sessions
                    fi
                    ;;
                config)
                    if (( CURRENT == 2 )); then
                        local -a shells
                        shells=('zsh:Zsh completion script' 'bash:Bash completion script')
                        _describe 'shell' shells
                    fi
                    ;;
            esac
            ;;
    esac
}}
compdef _box box

box() {{
    local __box_cd_file
    __box_cd_file=$(mktemp "/tmp/.box-cd.XXXXXX")
    BOX_CD_FILE="$__box_cd_file" command box "$@"
    local __box_exit=$?
    if [[ -s "$__box_cd_file" ]]; then
        local __box_dir
        __box_dir=$(<"$__box_cd_file")
        cd "$__box_dir"
    fi
    rm -f "$__box_cd_file"
    return $__box_exit
}}
"#
    );
    Ok(0)
}

fn cmd_config_bash() -> Result<i32> {
    print!(
        r#"_box() {{
    local cur prev words cword
    _init_completion || return

    local subcommands="create resume remove stop exec list cd path upgrade config"
    local session_cmds="resume remove stop exec cd path"

    if [[ $cword -eq 1 ]]; then
        local sessions=""
        if [[ -d "$HOME/.box/sessions" ]]; then
            sessions=$(command ls "$HOME/.box/sessions" 2>/dev/null)
        fi
        COMPREPLY=($(compgen -W "$sessions" -- "$cur"))
        return
    fi

    local subcmd="${{words[1]}}"

    case "$subcmd" in
        create)
            case "$cur" in
                -*)
                    COMPREPLY=($(compgen -W "-d --image --docker-args --no-ssh" -- "$cur"))
                    ;;
            esac
            ;;
        resume)
            case "$cur" in
                -*)
                    COMPREPLY=($(compgen -W "-d --docker-args" -- "$cur"))
                    ;;
                *)
                    if [[ $cword -eq 2 ]]; then
                        local sessions=""
                        if [[ -d "$HOME/.box/sessions" ]]; then
                            sessions=$(command ls "$HOME/.box/sessions" 2>/dev/null)
                        fi
                        COMPREPLY=($(compgen -W "$sessions" -- "$cur"))
                    fi
                    ;;
            esac
            ;;
        exec)
            if [[ $cword -eq 2 ]]; then
                local sessions=""
                if [[ -d "$HOME/.box/sessions" ]]; then
                    sessions=$(command ls "$HOME/.box/sessions" 2>/dev/null)
                fi
                COMPREPLY=($(compgen -W "$sessions" -- "$cur"))
            fi
            ;;
        list|ls)
            case "$cur" in
                -*)
                    COMPREPLY=($(compgen -W "--running -r --stopped -s --quiet -q" -- "$cur"))
                    ;;
            esac
            ;;
        remove|stop|path|cd)
            if [[ $cword -eq 2 ]]; then
                local sessions=""
                if [[ -d "$HOME/.box/sessions" ]]; then
                    sessions=$(command ls "$HOME/.box/sessions" 2>/dev/null)
                fi
                COMPREPLY=($(compgen -W "$sessions" -- "$cur"))
            fi
            ;;
        config)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=($(compgen -W "zsh bash" -- "$cur"))
            fi
            ;;
    esac
}}
complete -F _box box

box() {{
    local __box_cd_file
    __box_cd_file=$(mktemp "/tmp/.box-cd.XXXXXX")
    BOX_CD_FILE="$__box_cd_file" command box "$@"
    local __box_exit=$?
    if [[ -s "$__box_cd_file" ]]; then
        local __box_dir
        __box_dir=$(<"$__box_cd_file")
        cd "$__box_dir"
    fi
    rm -f "$__box_cd_file"
    return $__box_exit
}}
"#
    );
    Ok(0)
}

fn cmd_upgrade() -> Result<i32> {
    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: {}", current_version);

    println!("Checking for updates...");
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("yusukeshib")
        .repo_name("box")
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build release list: {}", e))?
        .fetch()
        .map_err(|e| anyhow::anyhow!("Failed to fetch releases: {}", e))?;

    let latest = releases
        .first()
        .ok_or_else(|| anyhow::anyhow!("No releases found"))?;
    let latest_version = latest.version.trim_start_matches('v');

    println!("Latest version: {}", latest_version);

    if current_version == latest_version {
        println!("Already at latest version.");
        return Ok(0);
    }

    let asset_name = upgrade_asset_name()?;
    println!("Looking for asset: {}", asset_name);

    let asset_exists = latest.assets.iter().any(|a| a.name == asset_name);
    if !asset_exists {
        bail!(
            "Asset '{}' not found for this platform. Available assets: {}",
            asset_name,
            latest
                .assets
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let download_url = format!(
        "https://github.com/yusukeshib/box/releases/download/v{}/{}",
        latest_version, asset_name
    );

    println!("Downloading new version...");
    let tmp_path = upgrade_download(&download_url)?;
    let _guard = UpgradeTempGuard(tmp_path.clone());

    println!("Installing update...");
    self_update::self_replace::self_replace(&tmp_path).map_err(|e| {
        let msg = e.to_string();
        if msg.to_lowercase().contains("permission denied") {
            anyhow::anyhow!(
                "Permission denied. Try running with elevated privileges (e.g., sudo box upgrade)."
            )
        } else {
            anyhow::anyhow!("{}", msg)
        }
    })?;

    println!("Upgraded from {} to {}.", current_version, latest_version);
    Ok(0)
}

/// RAII guard that removes the temp file on drop.
struct UpgradeTempGuard(std::path::PathBuf);

impl Drop for UpgradeTempGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn upgrade_asset_name() -> Result<String> {
    let arch = std::env::consts::ARCH;
    let os_name = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        other => bail!("Unsupported platform: {}", other),
    };
    Ok(format!("box-{}-{}", arch, os_name))
}

fn upgrade_download(url: &str) -> Result<std::path::PathBuf> {
    let tmp_path = std::env::temp_dir().join(format!("box-update-{}", std::process::id()));
    let mut tmp_file = fs::File::create(&tmp_path)?;

    self_update::Download::from_url(url)
        .download_to(&mut tmp_file)
        .map_err(|e| anyhow::anyhow!("Download failed: {}", e))?;

    tmp_file.flush()?;
    drop(tmp_file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tmp_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp_path, perms)?;
    }

    Ok(tmp_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        let mut full_args = vec!["box"];
        full_args.extend_from_slice(args);
        Cli::try_parse_from(full_args).unwrap()
    }

    fn try_parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut full_args = vec!["box"];
        full_args.extend_from_slice(args);
        Cli::try_parse_from(full_args)
    }

    // -- No args = TUI --

    #[test]
    fn test_no_args_launches_tui() {
        let cli = parse(&[]);
        assert!(cli.command.is_none());
    }

    // -- create subcommand --

    #[test]
    fn test_create_name_only() {
        let cli = parse(&["create", "my-session"]);
        match cli.command {
            Some(Commands::Create(args)) => {
                assert_eq!(args.name, "my-session");
                assert!(!args.detach);
                assert!(args.image.is_none());
                assert!(args.docker_args.is_none());
                assert!(!args.no_ssh);
                assert!(args.cmd.is_empty());
            }
            other => panic!("expected Create, got {:?}", other),
        }
    }

    #[test]
    fn test_create_with_all_options() {
        let cli = parse(&[
            "create",
            "full-session",
            "-d",
            "--image",
            "python:3.11",
            "--docker-args",
            "-e FOO=bar --network host",
            "--no-ssh",
            "--",
            "python",
            "main.py",
        ]);
        match cli.command {
            Some(Commands::Create(args)) => {
                assert_eq!(args.name, "full-session");
                assert!(args.detach);
                assert_eq!(args.image.as_deref(), Some("python:3.11"));
                assert_eq!(
                    args.docker_args.as_deref(),
                    Some("-e FOO=bar --network host")
                );
                assert!(args.no_ssh);
                assert_eq!(args.cmd, vec!["python", "main.py"]);
            }
            other => panic!("expected Create, got {:?}", other),
        }
    }

    #[test]
    fn test_create_with_image() {
        let cli = parse(&["create", "my-session", "--image", "ubuntu:latest"]);
        match cli.command {
            Some(Commands::Create(args)) => {
                assert_eq!(args.name, "my-session");
                assert_eq!(args.image.as_deref(), Some("ubuntu:latest"));
            }
            other => panic!("expected Create, got {:?}", other),
        }
    }

    #[test]
    fn test_create_with_command() {
        let cli = parse(&["create", "my-session", "--", "bash", "-c", "echo hi"]);
        match cli.command {
            Some(Commands::Create(args)) => {
                assert_eq!(args.name, "my-session");
                assert_eq!(args.cmd, vec!["bash", "-c", "echo hi"]);
            }
            other => panic!("expected Create, got {:?}", other),
        }
    }

    #[test]
    fn test_create_detach() {
        let cli = parse(&["create", "my-session", "-d"]);
        match cli.command {
            Some(Commands::Create(args)) => {
                assert_eq!(args.name, "my-session");
                assert!(args.detach);
            }
            other => panic!("expected Create, got {:?}", other),
        }
    }

    #[test]
    fn test_create_requires_name() {
        let result = try_parse(&["create"]);
        assert!(result.is_err());
    }

    // -- resume subcommand --

    #[test]
    fn test_resume_name_only() {
        let cli = parse(&["resume", "my-session"]);
        match cli.command {
            Some(Commands::Resume(args)) => {
                assert_eq!(args.name, "my-session");
                assert!(!args.detach);
                assert!(args.docker_args.is_none());
            }
            other => panic!("expected Resume, got {:?}", other),
        }
    }

    #[test]
    fn test_resume_detach() {
        let cli = parse(&["resume", "my-session", "-d"]);
        match cli.command {
            Some(Commands::Resume(args)) => {
                assert_eq!(args.name, "my-session");
                assert!(args.detach);
            }
            other => panic!("expected Resume, got {:?}", other),
        }
    }

    #[test]
    fn test_resume_with_docker_args() {
        let cli = parse(&["resume", "my-session", "--docker-args", "-e KEY=val"]);
        match cli.command {
            Some(Commands::Resume(args)) => {
                assert_eq!(args.name, "my-session");
                assert_eq!(args.docker_args.as_deref(), Some("-e KEY=val"));
            }
            other => panic!("expected Resume, got {:?}", other),
        }
    }

    #[test]
    fn test_resume_requires_name() {
        let result = try_parse(&["resume"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_resume_rejects_image() {
        let result = try_parse(&["resume", "my-session", "--image", "ubuntu"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_resume_rejects_no_ssh() {
        let result = try_parse(&["resume", "my-session", "--no-ssh"]);
        assert!(result.is_err());
    }

    // -- remove subcommand --

    #[test]
    fn test_remove_parses() {
        let cli = parse(&["remove", "my-session"]);
        match cli.command {
            Some(Commands::Remove(args)) => {
                assert_eq!(args.name, "my-session");
            }
            other => panic!("expected Remove, got {:?}", other),
        }
    }

    #[test]
    fn test_remove_requires_name() {
        let result = try_parse(&["remove"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_rejects_flags() {
        let result = try_parse(&["remove", "my-session", "-d"]);
        assert!(result.is_err());
    }

    // -- stop subcommand --

    #[test]
    fn test_stop_parses() {
        let cli = parse(&["stop", "my-session"]);
        match cli.command {
            Some(Commands::Stop(args)) => {
                assert_eq!(args.name, "my-session");
            }
            other => panic!("expected Stop, got {:?}", other),
        }
    }

    #[test]
    fn test_stop_requires_name() {
        let result = try_parse(&["stop"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_stop_rejects_flags() {
        let result = try_parse(&["stop", "my-session", "-d"]);
        assert!(result.is_err());
    }

    // -- exec subcommand --

    #[test]
    fn test_exec_parses() {
        let cli = parse(&["exec", "my-session", "--", "ls", "-la"]);
        match cli.command {
            Some(Commands::Exec(args)) => {
                assert_eq!(args.name, "my-session");
                assert_eq!(args.cmd, vec!["ls", "-la"]);
            }
            other => panic!("expected Exec, got {:?}", other),
        }
    }

    #[test]
    fn test_exec_requires_name() {
        let result = try_parse(&["exec"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_exec_requires_command() {
        let result = try_parse(&["exec", "my-session"]);
        assert!(result.is_err());
    }

    // -- path subcommand --

    #[test]
    fn test_path_subcommand_parses() {
        let cli = parse(&["path", "my-session"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Path { ref name }) if name == "my-session"
        ));
    }

    #[test]
    fn test_path_requires_name() {
        let result = try_parse(&["path"]);
        assert!(result.is_err());
    }

    // -- cd subcommand --

    #[test]
    fn test_cd_subcommand_parses() {
        let cli = parse(&["cd", "my-session"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Cd { ref name }) if name == "my-session"
        ));
    }

    #[test]
    fn test_cd_requires_name() {
        let result = try_parse(&["cd"]);
        assert!(result.is_err());
    }

    // -- upgrade subcommand --

    #[test]
    fn test_upgrade_subcommand_parses() {
        let cli = parse(&["upgrade"]);
        assert!(matches!(cli.command, Some(Commands::Upgrade)));
    }

    #[test]
    fn test_upgrade_rejects_flags() {
        let result = try_parse(&["upgrade", "-d"]);
        assert!(result.is_err());
    }

    // -- config subcommand --

    #[test]
    fn test_config_zsh_subcommand_parses() {
        let cli = parse(&["config", "zsh"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Config {
                shell: ConfigShell::Zsh
            })
        ));
    }

    #[test]
    fn test_config_bash_subcommand_parses() {
        let cli = parse(&["config", "bash"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Config {
                shell: ConfigShell::Bash
            })
        ));
    }

    #[test]
    fn test_config_requires_shell() {
        let result = try_parse(&["config"]);
        assert!(result.is_err());
    }

    // -- list subcommand --

    #[test]
    fn test_list_no_flags() {
        let cli = parse(&["list"]);
        match cli.command {
            Some(Commands::List(args)) => {
                assert!(!args.running);
                assert!(!args.stopped);
                assert!(!args.quiet);
            }
            other => panic!("expected List, got {:?}", other),
        }
    }

    #[test]
    fn test_list_running_flag() {
        let cli = parse(&["list", "--running"]);
        match cli.command {
            Some(Commands::List(args)) => {
                assert!(args.running);
                assert!(!args.stopped);
            }
            other => panic!("expected List, got {:?}", other),
        }
    }

    #[test]
    fn test_list_stopped_flag() {
        let cli = parse(&["list", "--stopped"]);
        match cli.command {
            Some(Commands::List(args)) => {
                assert!(!args.running);
                assert!(args.stopped);
            }
            other => panic!("expected List, got {:?}", other),
        }
    }

    #[test]
    fn test_list_quiet_flag() {
        let cli = parse(&["list", "-q"]);
        match cli.command {
            Some(Commands::List(args)) => {
                assert!(args.quiet);
            }
            other => panic!("expected List, got {:?}", other),
        }
    }

    #[test]
    fn test_list_combined_flags() {
        let cli = parse(&["list", "-q", "--running"]);
        match cli.command {
            Some(Commands::List(args)) => {
                assert!(args.quiet);
                assert!(args.running);
                assert!(!args.stopped);
            }
            other => panic!("expected List, got {:?}", other),
        }
    }

    #[test]
    fn test_list_short_flags() {
        let cli = parse(&["list", "-r", "-s", "-q"]);
        match cli.command {
            Some(Commands::List(args)) => {
                assert!(args.running);
                assert!(args.stopped);
                assert!(args.quiet);
            }
            other => panic!("expected List, got {:?}", other),
        }
    }

    #[test]
    fn test_list_alias_ls() {
        let cli = parse(&["ls"]);
        match cli.command {
            Some(Commands::List(args)) => {
                assert!(!args.running);
                assert!(!args.stopped);
                assert!(!args.quiet);
            }
            other => panic!("expected List, got {:?}", other),
        }
    }

    #[test]
    fn test_list_rejects_positional_args() {
        let result = try_parse(&["list", "my-session"]);
        assert!(result.is_err());
    }

    // -- bare name shortcut (external subcommand) --

    #[test]
    fn test_bare_name_parsed_as_external() {
        let cli = parse(&["my-session"]);
        match cli.command {
            Some(Commands::External(args)) => {
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], "my-session");
            }
            other => panic!("expected External, got {:?}", other),
        }
    }
}
