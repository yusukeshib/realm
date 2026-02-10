mod config;
mod docker;
mod git;
mod session;
mod tui;

use anyhow::{bail, Result};
use clap::Parser;
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Parser)]
#[command(
    name = "realm",
    about = "Sandboxed Docker environments for git repos",
    after_help = "Examples:\n  realm                                    # interactive session manager\n  realm my-feature --image ubuntu:latest -- bash\n  realm my-feature\n  realm my-feature -d -- claude -p \"do something\"\n  realm upgrade\n\nSessions are automatically created if they don't exist."
)]
struct Cli {
    /// Session name
    name: Option<String>,

    /// Run container in the background (detached)
    #[arg(short = 'd')]
    detach: bool,

    /// Docker image to use (default: $REALM_DEFAULT_IMAGE or alpine:latest)
    #[arg(long)]
    image: Option<String>,

    /// Extra Docker flags (e.g. -e KEY=VALUE, -v /host:/container, --network host).
    /// Overrides $REALM_DOCKER_ARGS when provided.
    #[arg(long = "docker-args", allow_hyphen_values = true)]
    docker_args: Option<String>,

    /// Disable SSH agent forwarding (enabled by default)
    #[arg(long = "no-ssh")]
    no_ssh: bool,

    /// Command to run in container
    #[arg(last = true)]
    cmd: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    let docker_args = cli
        .docker_args
        .or_else(|| std::env::var("REALM_DOCKER_ARGS").ok())
        .unwrap_or_default();

    let result = match cli.name.as_deref() {
        None if cli.detach => {
            eprintln!("Error: Session name required for -d.");
            std::process::exit(1);
        }
        None => cmd_list(),
        Some("upgrade") if cli.detach => {
            eprintln!("Error: -d cannot be used with upgrade.");
            std::process::exit(1);
        }
        Some("upgrade") => cmd_upgrade(),
        Some(name) => cmd_create_or_resume(
            name,
            cli.image,
            docker_args,
            cli.cmd,
            !cli.no_ssh,
            cli.detach,
        ),
    };

    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
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

    match tui::session_manager(&sessions, delete_fn)? {
        tui::TuiAction::Resume(name) => cmd_resume(&name, "", vec![], true, false),
        tui::TuiAction::New { name, image } => cmd_create(&name, image, "", vec![], true, false),
        tui::TuiAction::Quit => Ok(0),
    }
}

fn cmd_create(
    name: &str,
    image: Option<String>,
    docker_args: &str,
    cmd: Vec<String>,
    ssh: bool,
    detach: bool,
) -> Result<i32> {
    session::validate_name(name)?;

    let project_dir = fs::canonicalize(".")
        .map_err(|_| anyhow::anyhow!("Cannot resolve current directory."))?
        .to_string_lossy()
        .to_string();

    if !git::is_repo(Path::new(&project_dir)) {
        bail!("'{}' is not a git repository.", project_dir);
    }

    docker::check()?;

    let cfg = config::resolve(config::RealmConfigInput {
        name: name.to_string(),
        image,
        mount_path: None,
        project_dir,
        command: cmd,
        env: vec![],
        ssh,
    });

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

fn cmd_create_or_resume(
    name: &str,
    image: Option<String>,
    docker_args: String,
    cmd: Vec<String>,
    ssh: bool,
    detach: bool,
) -> Result<i32> {
    // Check if session exists
    if session::session_exists(name) {
        // Session exists - resume it
        return cmd_resume(name, &docker_args, cmd, ssh, detach);
    }

    // Session doesn't exist - create it
    cmd_create(name, image, &docker_args, cmd, ssh, detach)
}

fn cmd_resume(
    name: &str,
    docker_args: &str,
    cmd: Vec<String>,
    ssh: bool,
    detach: bool,
) -> Result<i32> {
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

    if cmd.is_empty() && docker::container_exists(name) {
        if detach {
            docker::start_container_detached(name)
        } else {
            docker::start_container(name)
        }
    } else {
        let final_cmd = if cmd.is_empty() {
            sess.command.clone()
        } else {
            cmd
        };
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
            cmd: &final_cmd,
            env: &sess.env,
            home: &home,
            docker_args: docker_args_opt,
            ssh,
            detach,
        })
    }
}

fn cmd_upgrade() -> Result<i32> {
    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: {}", current_version);

    println!("Checking for updates...");
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("yusukeshib")
        .repo_name("realm")
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
        "https://github.com/yusukeshib/realm/releases/download/v{}/{}",
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
                "Permission denied. Try running with elevated privileges (e.g., sudo realm upgrade)."
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
    Ok(format!("realm-{}-{}", arch, os_name))
}

fn upgrade_download(url: &str) -> Result<std::path::PathBuf> {
    let tmp_path = std::env::temp_dir().join(format!("realm-update-{}", std::process::id()));
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
        let mut full_args = vec!["realm"];
        full_args.extend_from_slice(args);
        Cli::try_parse_from(full_args).unwrap()
    }

    #[test]
    fn test_no_args_lists() {
        let cli = parse(&[]);
        assert!(cli.name.is_none());
        assert!(!cli.detach);
    }

    #[test]
    fn test_name_only_resumes() {
        let cli = parse(&["my-session"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(!cli.detach);
    }

    #[test]
    fn test_detach_flag_before_name() {
        let cli = parse(&["-d", "my-session"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.detach);
    }

    #[test]
    fn test_detach_flag_after_name() {
        let cli = parse(&["my-session", "-d"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.detach);
    }

    #[test]
    fn test_detach_with_command() {
        let cli = parse(&["my-session", "-d", "--", "sleep", "60"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.detach);
        assert_eq!(cli.cmd, vec!["sleep", "60"]);
    }

    #[test]
    fn test_detach_without_name() {
        let cli = parse(&["-d"]);
        assert!(cli.name.is_none());
        assert!(cli.detach);
    }

    #[test]
    fn test_create_with_image() {
        let cli = parse(&["my-session", "--image", "ubuntu:latest"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert_eq!(cli.image.as_deref(), Some("ubuntu:latest"));
    }

    #[test]
    fn test_with_command() {
        let cli = parse(&["my-session", "--", "bash", "-c", "echo hi"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert_eq!(cli.cmd, vec!["bash", "-c", "echo hi"]);
    }

    #[test]
    fn test_create_with_command() {
        let cli = parse(&["my-session", "--", "bash"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert_eq!(cli.cmd, vec!["bash"]);
    }

    #[test]
    fn test_create_all_options() {
        let cli = parse(&[
            "full-session",
            "--image",
            "python:3.11",
            "--docker-args",
            "-e FOO=bar --network host",
            "--",
            "python",
            "main.py",
        ]);
        assert_eq!(cli.name.as_deref(), Some("full-session"));
        assert_eq!(cli.image.as_deref(), Some("python:3.11"));
        assert_eq!(
            cli.docker_args.as_deref(),
            Some("-e FOO=bar --network host")
        );
        assert_eq!(cli.cmd, vec!["python", "main.py"]);
    }

    #[test]
    fn test_docker_args() {
        let cli = parse(&["my-session", "--docker-args", "-e KEY=val -v /a:/b"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert_eq!(cli.docker_args.as_deref(), Some("-e KEY=val -v /a:/b"));
    }

    #[test]
    fn test_empty_command_after_separator() {
        let cli = parse(&["my-session", "--"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.cmd.is_empty());
    }
}
