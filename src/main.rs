mod config;
mod docker;
mod git;
mod session;
mod tui;

use anyhow::{bail, Result};
use clap::Parser;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(
    name = "realm",
    about = "Sandboxed Docker environments for git repos",
    after_help = "Commands:\n  realm upgrade    Upgrade realm to the latest version"
)]
struct Cli {
    /// Session name
    name: Option<String>,

    /// Create a new session
    #[arg(short = 'c')]
    create: bool,

    /// Delete the session
    #[arg(short = 'd')]
    delete: bool,

    /// Environment variables to pass to container (e.g. -e KEY or -e KEY=VALUE)
    #[arg(short, long = "env")]
    env: Vec<String>,

    /// Docker image to use (default: $REALM_DEFAULT_IMAGE or alpine/git)
    #[arg(long)]
    image: Option<String>,

    /// Mount path inside container (default: /<dir-name>)
    #[arg(long = "mount")]
    mount_path: Option<String>,

    /// Project directory (default: current directory)
    #[arg(long = "dir")]
    dir: Option<String>,

    /// Command to run in container
    #[arg(last = true)]
    cmd: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.name.as_deref() {
        None => cmd_list(),
        Some("upgrade") => cmd_upgrade(),
        Some(_) if cli.delete => cmd_delete(cli.name.as_deref().unwrap()),
        Some(_) if cli.create => cmd_create(
            cli.name.as_deref().unwrap(),
            cli.image,
            cli.mount_path,
            cli.dir,
            cli.cmd,
            cli.env,
        ),
        Some(name) => cmd_resume(name, cli.cmd),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_list() -> Result<()> {
    let sessions = session::list()?;
    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    match tui::select_session(&sessions)? {
        Some(i) => cmd_resume(&sessions[i].name, vec![]),
        None => Ok(()),
    }
}

fn cmd_create(
    name: &str,
    image: Option<String>,
    mount_path: Option<String>,
    dir: Option<String>,
    cmd: Vec<String>,
    env: Vec<String>,
) -> Result<()> {
    session::validate_name(name)?;

    let project_dir = match dir {
        Some(d) => fs::canonicalize(&d)
            .map_err(|_| anyhow::anyhow!("Directory '{}' not found.", d))?
            .to_string_lossy()
            .to_string(),
        None => fs::canonicalize(".")
            .map_err(|_| anyhow::anyhow!("Cannot resolve current directory."))?
            .to_string_lossy()
            .to_string(),
    };

    if !git::is_repo(Path::new(&project_dir)) {
        bail!("'{}' is not a git repository.", project_dir);
    }

    if session::session_exists(name) {
        bail!(
            "Session '{}' already exists. Remove it first: realm {} -d",
            name,
            name
        );
    }

    docker::check()?;

    let cfg = config::resolve(config::RealmConfigInput {
        name: name.to_string(),
        image,
        mount_path,
        project_dir,
        command: cmd,
        env,
    });

    let sess = session::Session {
        name: cfg.name.clone(),
        project_dir: cfg.project_dir.clone(),
        image: cfg.image.clone(),
        mount_path: cfg.mount_path.clone(),
        command: cfg.command.clone(),
        env: cfg.env.clone(),
    };
    session::save(&sess)?;

    docker::remove_container(name);
    let exit_code = docker::run_container(
        name,
        &cfg.project_dir,
        &cfg.image,
        &cfg.mount_path,
        &cfg.command,
        &cfg.env,
    )?;
    git::reset_index(&cfg.project_dir);
    std::process::exit(exit_code);
}

fn cmd_resume(name: &str, cmd: Vec<String>) -> Result<()> {
    session::validate_name(name)?;

    let sess = session::load(name)?;

    if !Path::new(&sess.project_dir).is_dir() {
        bail!("Project directory '{}' no longer exists.", sess.project_dir);
    }

    docker::check()?;

    println!("Resuming session '{}'...", name);
    session::touch_resumed_at(name)?;

    let exit_code = if cmd.is_empty() && docker::container_exists(name) {
        docker::start_container(name)?
    } else {
        let final_cmd = if cmd.is_empty() {
            sess.command.clone()
        } else {
            cmd
        };
        docker::remove_container(name);
        docker::run_container(
            name,
            &sess.project_dir,
            &sess.image,
            &sess.mount_path,
            &final_cmd,
            &sess.env,
        )?
    };
    git::reset_index(&sess.project_dir);
    std::process::exit(exit_code);
}

fn cmd_upgrade() -> Result<()> {
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
        return Ok(());
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
        if msg.contains("ermission denied") {
            anyhow::anyhow!(
                "Permission denied. Try running with elevated privileges (e.g., sudo realm upgrade)."
            )
        } else {
            anyhow::anyhow!("{}", msg)
        }
    })?;

    println!("Upgraded from {} to {}.", current_version, latest_version);
    Ok(())
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

    use std::io::Write;
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

fn cmd_delete(name: &str) -> Result<()> {
    session::validate_name(name)?;

    if !session::session_exists(name) {
        bail!("Session '{}' not found.", name);
    }

    docker::remove_container(name);
    session::remove_dir(name)?;
    println!("Session '{}' removed.", name);
    Ok(())
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
        assert!(!cli.create);
        assert!(!cli.delete);
    }

    #[test]
    fn test_name_only_resumes() {
        let cli = parse(&["my-session"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(!cli.create);
        assert!(!cli.delete);
    }

    #[test]
    fn test_create_flag_before_name() {
        let cli = parse(&["-c", "my-session"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.create);
    }

    #[test]
    fn test_create_flag_after_name() {
        let cli = parse(&["my-session", "-c"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.create);
    }

    #[test]
    fn test_delete_flag_before_name() {
        let cli = parse(&["-d", "my-session"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.delete);
    }

    #[test]
    fn test_delete_flag_after_name() {
        let cli = parse(&["my-session", "-d"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.delete);
    }

    #[test]
    fn test_create_with_image() {
        let cli = parse(&["my-session", "-c", "--image", "ubuntu:latest"]);
        assert!(cli.create);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert_eq!(cli.image.as_deref(), Some("ubuntu:latest"));
    }

    #[test]
    fn test_create_with_mount() {
        let cli = parse(&["-c", "my-session", "--mount", "/src"]);
        assert!(cli.create);
        assert_eq!(cli.mount_path.as_deref(), Some("/src"));
    }

    #[test]
    fn test_create_with_dir() {
        let cli = parse(&["-c", "my-session", "--dir", "/tmp/project"]);
        assert!(cli.create);
        assert_eq!(cli.dir.as_deref(), Some("/tmp/project"));
    }

    #[test]
    fn test_with_command() {
        let cli = parse(&["my-session", "--", "bash", "-c", "echo hi"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert_eq!(cli.cmd, vec!["bash", "-c", "echo hi"]);
    }

    #[test]
    fn test_create_with_command() {
        let cli = parse(&["-c", "my-session", "--", "bash"]);
        assert!(cli.create);
        assert_eq!(cli.cmd, vec!["bash"]);
    }

    #[test]
    fn test_create_all_options() {
        let cli = parse(&[
            "-c",
            "full-session",
            "--image",
            "python:3.11",
            "--mount",
            "/app",
            "--dir",
            "/tmp/project",
            "--",
            "python",
            "main.py",
        ]);
        assert!(cli.create);
        assert_eq!(cli.name.as_deref(), Some("full-session"));
        assert_eq!(cli.image.as_deref(), Some("python:3.11"));
        assert_eq!(cli.mount_path.as_deref(), Some("/app"));
        assert_eq!(cli.dir.as_deref(), Some("/tmp/project"));
        assert_eq!(cli.cmd, vec!["python", "main.py"]);
    }

    #[test]
    fn test_empty_command_after_separator() {
        let cli = parse(&["my-session", "--"]);
        assert_eq!(cli.name.as_deref(), Some("my-session"));
        assert!(cli.cmd.is_empty());
    }

    #[test]
    fn test_create_flag_at_end_with_options() {
        let cli = parse(&["test-session", "--image", "ubuntu:latest", "-c"]);
        assert!(cli.create);
        assert_eq!(cli.name.as_deref(), Some("test-session"));
        assert_eq!(cli.image.as_deref(), Some("ubuntu:latest"));
    }

    #[test]
    fn test_env_single() {
        let cli = parse(&["my-session", "-c", "-e", "FOO=bar"]);
        assert!(cli.create);
        assert_eq!(cli.env, vec!["FOO=bar"]);
    }

    #[test]
    fn test_env_multiple() {
        let cli = parse(&["my-session", "-c", "-e", "FOO", "-e", "BAR=baz"]);
        assert!(cli.create);
        assert_eq!(cli.env, vec!["FOO", "BAR=baz"]);
    }

    #[test]
    fn test_env_long_flag() {
        let cli = parse(&["my-session", "-c", "--env", "KEY=val"]);
        assert_eq!(cli.env, vec!["KEY=val"]);
    }

    #[test]
    fn test_env_empty_by_default() {
        let cli = parse(&["my-session", "-c"]);
        assert!(cli.env.is_empty());
    }
}
