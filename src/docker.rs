use anyhow::{bail, Result};
use std::path::Path;
use std::process::Command;

/// Create a workspace directory on the host for the session.
/// On first run, clones the project repo via `git clone --local`.
/// Returns the host path. The directory is world-writable so any container user can write.
pub fn ensure_workspace(home: &str, name: &str, project_dir: &str) -> Result<String> {
    let dir = format!("{}/.realm/workspaces/{}", home, name);
    let git_dir = format!("{}/.git", dir);

    if !Path::new(&git_dir).exists() {
        let status = Command::new("git")
            .args(["clone", "--local", project_dir, &dir])
            .status()?;
        if !status.success() {
            bail!("git clone --local failed");
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dir)?.permissions();
        perms.set_mode(0o777);
        std::fs::set_permissions(&dir, perms)?;
    }

    Ok(dir)
}

/// Remove the workspace directory for a session.
pub fn remove_workspace(name: &str) {
    let home = std::env::var("HOME").unwrap_or_default();
    let dir = format!("{}/.realm/workspaces/{}", home, name);
    let _ = std::fs::remove_dir_all(&dir);
}

pub fn check() -> Result<()> {
    let has_docker = Command::new("command")
        .args(["-v", "docker"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    // "command -v" may not work outside a shell, so try "docker version" instead
    let docker_exists = has_docker.map(|s| s.success()).unwrap_or(false)
        || Command::new("docker")
            .arg("version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

    if !docker_exists {
        bail!("docker is not installed. See https://docs.docker.com/get-docker/");
    }

    let info = Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !info.success() {
        bail!("Docker daemon is not running. Please start Docker.");
    }

    Ok(())
}

const SSH_CONTAINER_PATH: &str = "/run/host-services/ssh-auth.sock";

/// Return (host_path, container_path) for SSH agent forwarding.
fn ssh_agent_paths() -> Result<(String, String)> {
    if cfg!(target_os = "macos") {
        Ok((
            "/run/host-services/ssh-auth.sock".to_string(),
            SSH_CONTAINER_PATH.to_string(),
        ))
    } else {
        let host = std::env::var("SSH_AUTH_SOCK").map_err(|_| {
            anyhow::anyhow!("SSH_AUTH_SOCK is not set. Cannot forward SSH agent on Linux.")
        })?;
        Ok((host, SSH_CONTAINER_PATH.to_string()))
    }
}

/// Build the docker run argument list without executing. Used by run_container and tests.
#[allow(clippy::too_many_arguments)]
pub fn build_run_args(
    name: &str,
    image: &str,
    mount_path: &str,
    cmd: &[String],
    env: &[String],
    home: &str,
    gitconfig_exists: bool,
    docker_args_env: Option<&str>,
    ssh: bool,
) -> Result<Vec<String>> {
    let workspace_dir = format!("{}/.realm/workspaces/{}", home, name);
    let mut args: Vec<String> = vec![
        "run".into(),
        "-it".into(),
        "--name".into(),
        format!("realm-{}", name),
        "--hostname".into(),
        format!("realm-{}", name),
        "-v".into(),
        format!("{}:{}", workspace_dir, mount_path),
        "-w".into(),
        mount_path.into(),
    ];

    if gitconfig_exists {
        let gitconfig = format!("{}/.gitconfig", home);
        args.push("-v".into());
        args.push(format!("{}:/root/.gitconfig:ro", gitconfig));
    }

    if ssh {
        let (host_path, container_path) = ssh_agent_paths()?;
        args.push("-v".into());
        args.push(format!("{}:{}", host_path, container_path));
        args.push("-e".into());
        args.push(format!("SSH_AUTH_SOCK={}", container_path));
    }

    if let Some(extra) = docker_args_env {
        if !extra.is_empty() {
            match shell_words::split(extra) {
                Ok(extra_args) => args.extend(extra_args),
                Err(e) => {
                    bail!("Failed to parse REALM_DOCKER_ARGS: {}", e);
                }
            }
        }
    }

    for entry in env {
        args.push("-e".into());
        args.push(entry.clone());
    }

    args.push(image.into());

    if !cmd.is_empty() {
        args.extend(cmd.iter().cloned());
    }

    Ok(args)
}

pub fn run_container(
    name: &str,
    project_dir: &str,
    image: &str,
    mount_path: &str,
    cmd: &[String],
    env: &[String],
    ssh: bool,
) -> Result<i32> {
    let home = std::env::var("HOME").unwrap_or_default();
    let gitconfig = format!("{}/.gitconfig", home);
    let gitconfig_exists = Path::new(&gitconfig).exists();
    let docker_args_env = std::env::var("REALM_DOCKER_ARGS").ok();

    ensure_workspace(&home, name, project_dir)?;

    let args = build_run_args(
        name,
        image,
        mount_path,
        cmd,
        env,
        &home,
        gitconfig_exists,
        docker_args_env.as_deref(),
        ssh,
    )?;

    let status = Command::new("docker")
        .args(&args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()?;

    Ok(status.code().unwrap_or(1))
}

pub fn container_exists(name: &str) -> bool {
    Command::new("docker")
        .args(["container", "inspect", &format!("realm-{}", name)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn start_container(name: &str) -> Result<i32> {
    let status = Command::new("docker")
        .args(["start", "-ai", &format!("realm-{}", name)])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()?;

    Ok(status.code().unwrap_or(1))
}

pub fn remove_container(name: &str) {
    let _ = Command::new("docker")
        .args(["rm", "-f", &format!("realm-{}", name)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_run_args_basic() {
        let args = build_run_args(
            "test-session",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        assert_eq!(args[0], "run");
        assert_eq!(args[1], "-it");
        assert_eq!(args[2], "--name");
        assert_eq!(args[3], "realm-test-session");
        assert_eq!(args[4], "--hostname");
        assert_eq!(args[5], "realm-test-session");
        assert_eq!(args[6], "-v");
        assert_eq!(
            args[7],
            "/home/user/.realm/workspaces/test-session:/workspace"
        );
        assert_eq!(args[8], "-w");
        assert_eq!(args[9], "/workspace");
        // image
        assert_eq!(args[10], "alpine/git");
        assert_eq!(args.len(), 11);
    }

    #[test]
    fn test_build_run_args_with_command() {
        let cmd = vec!["bash".to_string()];
        let args = build_run_args(
            "sess",
            "ubuntu:latest",
            "/workspace",
            &cmd,
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        // Command follows image directly
        let image_pos = args.iter().position(|a| a == "ubuntu:latest").unwrap();
        assert_eq!(args[image_pos + 1], "bash");
        assert_eq!(args.len(), image_pos + 2);
    }

    #[test]
    fn test_build_run_args_with_multi_command() {
        let cmd = vec!["python".to_string(), "-m".to_string(), "pytest".to_string()];
        let args = build_run_args(
            "sess",
            "python:3.11",
            "/app",
            &cmd,
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        let image_pos = args.iter().position(|a| a == "python:3.11").unwrap();
        assert_eq!(args[image_pos + 1], "python");
        assert_eq!(args[image_pos + 2], "-m");
        assert_eq!(args[image_pos + 3], "pytest");
        assert_eq!(args.len(), image_pos + 4);
    }

    #[test]
    fn test_build_run_args_with_gitconfig() {
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            true,
            None,
            false,
        )
        .unwrap();

        assert!(args.contains(&"-v".to_string()));
        assert!(args.contains(&"/home/user/.gitconfig:/root/.gitconfig:ro".to_string()));
    }

    #[test]
    fn test_build_run_args_without_gitconfig() {
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        assert!(!args.contains(&"/home/user/.gitconfig:/root/.gitconfig:ro".to_string()));
    }

    #[test]
    fn test_build_run_args_with_docker_args_env() {
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            Some("--network host -v /data:/data:ro"),
            false,
        )
        .unwrap();

        assert!(args.contains(&"--network".to_string()));
        assert!(args.contains(&"host".to_string()));
        assert!(args.contains(&"/data:/data:ro".to_string()));
    }

    #[test]
    fn test_build_run_args_docker_args_with_quotes() {
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            Some("-e 'FOO=hello world'"),
            false,
        )
        .unwrap();

        assert!(args.contains(&"-e".to_string()));
        assert!(args.contains(&"FOO=hello world".to_string()));
    }

    #[test]
    fn test_build_run_args_empty_docker_args() {
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            Some(""),
            false,
        )
        .unwrap();

        // Should still work, just no extra args
        assert!(args.contains(&"alpine/git".to_string()));
    }

    #[test]
    fn test_build_run_args_invalid_docker_args() {
        let result = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            Some("--flag 'unclosed quote"),
            false,
        );

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse REALM_DOCKER_ARGS"));
    }

    #[test]
    fn test_build_run_args_custom_mount_path() {
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/src",
            &[],
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        assert!(args.contains(&"/home/user/.realm/workspaces/sess:/src".to_string()));
        assert!(args.contains(&"/src".to_string()));
    }

    #[test]
    fn test_build_run_args_hostname() {
        let args = build_run_args(
            "my-session",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        assert!(args.contains(&"realm-my-session".to_string()));
    }

    #[test]
    fn test_build_run_args_no_rm_flag() {
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        assert!(!args.contains(&"--rm".to_string()));
    }

    #[test]
    fn test_build_run_args_has_name() {
        let args = build_run_args(
            "my-session",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        let name_pos = args.iter().position(|a| a == "--name").unwrap();
        assert_eq!(args[name_pos + 1], "realm-my-session");
    }

    #[test]
    fn test_build_run_args_with_env() {
        let env = vec!["FOO=bar".to_string(), "BAZ".to_string()];
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &env,
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        // env flags should appear before the image
        let e_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "-e")
            .map(|(i, _)| i)
            .collect();
        assert!(e_positions.len() >= 2);
        let last_two = &e_positions[e_positions.len() - 2..];
        assert_eq!(args[last_two[0] + 1], "FOO=bar");
        assert_eq!(args[last_two[1] + 1], "BAZ");
    }

    #[test]
    fn test_build_run_args_empty_env() {
        let args = build_run_args(
            "sess",
            "alpine/git",
            "/workspace",
            &[],
            &[],
            "/home/user",
            false,
            None,
            false,
        )
        .unwrap();

        // No -e flags should be present
        assert!(!args.iter().any(|a| a == "-e"));
    }
}
