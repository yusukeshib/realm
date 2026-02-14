use anyhow::{bail, Result};
use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::config;

/// Create a workspace directory on the host for the session.
/// On first run, clones the project repo via `git clone --local`.
/// Returns the host path. The directory is writable by the owner and group so container users with the appropriate group can write.
pub fn ensure_workspace(home: &str, name: &str, project_dir: &str) -> Result<String> {
    let dir_path = Path::new(home).join(".box").join("workspaces").join(name);
    let dir = dir_path.to_string_lossy().to_string();
    let git_dir = dir_path.join(".git");

    if !Path::new(&git_dir).exists() {
        eprintln!("\x1b[2mrunning clone command:\x1b[0m");
        eprintln!("git clone --local {} {}", project_dir, dir);
        let status = Command::new("git")
            .args(["clone", "--local", project_dir, &dir])
            .status()?;
        if !status.success() {
            bail!("git clone --local failed");
        }

        // git clone --local sets origin to the host path, which won't exist
        // inside the container. Re-point origin to the real remote URL.
        if let Ok(output) = Command::new("git")
            .args(["-C", project_dir, "remote", "get-url", "origin"])
            .output()
        {
            if output.status.success() {
                let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !url.is_empty() {
                    eprintln!("\x1b[2mrunning remote update:\x1b[0m");
                    eprintln!("git remote set-url origin {}", url);
                    let _ = Command::new("git")
                        .args(["-C", &dir, "remote", "set-url", "origin", &url])
                        .status();
                }
            }
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
    if let Ok(home) = config::home_dir() {
        let dir = Path::new(&home).join(".box").join("workspaces").join(name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

pub fn check() -> Result<()> {
    let docker_exists = Command::new("docker")
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
///
/// On macOS, both Docker Desktop and OrbStack expose a magic VM-internal socket
/// at `/run/host-services/ssh-auth.sock` that forwards to the host SSH agent.
/// Mounting the raw host socket (e.g. 1Password's) does NOT work because Unix
/// sockets cannot cross the VM boundary.
///
/// On Linux, the host socket from `SSH_AUTH_SOCK` is used directly.
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

/// Fix SSH agent socket permissions for non-root container users.
///
/// OrbStack sets restrictive permissions (0660) on the forwarded SSH agent socket,
/// which prevents non-root container users from accessing it. This runs a one-shot
/// container as root to make the socket world-accessible. Silently ignored if it fails.
fn fix_ssh_socket_permissions(image: &str) {
    let mount = format!("{p}:{p}", p = SSH_CONTAINER_PATH);
    let _ = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--user",
            "root",
            "--entrypoint",
            "chmod",
            "-v",
            &mount,
            image,
            "666",
            SSH_CONTAINER_PATH,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Restore terminal state after an interactive Docker session.
/// Writes show-cursor and attribute-reset escape sequences. Best-effort; errors ignored.
fn restore_terminal() {
    let _ = std::io::stdout().write_all(b"\x1b[?25h\x1b[0m");
    let _ = std::io::stdout().flush();
}

pub struct DockerRunConfig<'a> {
    pub name: &'a str,
    pub project_dir: &'a str,
    pub image: &'a str,
    pub mount_path: &'a str,
    pub cmd: &'a [String],
    pub env: &'a [String],
    pub home: &'a str,
    pub docker_args: Option<&'a str>,
    pub ssh: bool,
    pub detach: bool,
}

/// Build the docker run argument list without executing. Used by run_container and tests.
pub fn build_run_args(cfg: &DockerRunConfig) -> Result<Vec<String>> {
    let workspace_dir = Path::new(cfg.home)
        .join(".box")
        .join("workspaces")
        .join(cfg.name);
    let workspace_dir = workspace_dir.to_string_lossy();
    let interactive_flag = if cfg.detach { "-d" } else { "-it" };
    let mut args: Vec<String> = vec![
        "run".into(),
        interactive_flag.into(),
        "--name".into(),
        format!("box-{}", cfg.name),
        "--hostname".into(),
        format!("box-{}", cfg.name),
        "-v".into(),
        format!("{}:{}", workspace_dir, cfg.mount_path),
        "-w".into(),
        cfg.mount_path.into(),
    ];

    // Mount host ~/.gitconfig so git user.name/user.email etc. are available
    let gitconfig = Path::new(cfg.home).join(".gitconfig");
    if gitconfig.exists() {
        args.push("-v".into());
        args.push(format!("{}:/etc/gitconfig:ro", gitconfig.display()));
    }

    if cfg.ssh {
        let (host_path, container_path) = ssh_agent_paths()?;
        args.push("-v".into());
        args.push(format!("{}:{}", host_path, container_path));
        args.push("-e".into());
        args.push(format!("SSH_AUTH_SOCK={}", container_path));
    }

    if let Some(extra) = cfg.docker_args {
        if !extra.is_empty() {
            match shell_words::split(extra) {
                Ok(extra_args) => args.extend(extra_args),
                Err(e) => {
                    bail!("Failed to parse docker args: {}", e);
                }
            }
        }
    }

    for entry in cfg.env {
        args.push("-e".into());
        args.push(entry.clone());
    }

    args.push(cfg.image.into());

    if !cfg.cmd.is_empty() {
        args.extend(cfg.cmd.iter().cloned());
    }

    Ok(args)
}

pub fn run_container(cfg: &DockerRunConfig) -> Result<i32> {
    ensure_workspace(cfg.home, cfg.name, cfg.project_dir)?;

    if cfg.ssh && std::cfg!(target_os = "macos") {
        fix_ssh_socket_permissions(cfg.image);
    }

    let args = build_run_args(cfg)?;
    eprintln!("\x1b[2mrunning container:\x1b[0m");
    eprintln!("docker {}", shell_words::join(&args));

    if cfg.detach {
        let output = Command::new("docker").args(&args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("docker run failed: {}", stderr.trim());
        }
        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        println!("{}", container_id);
        println!("Run `box {}` to attach.", cfg.name);
        Ok(0)
    } else {
        let status = Command::new("docker")
            .args(&args)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()?;
        restore_terminal();
        Ok(status.code().unwrap_or(1))
    }
}

pub fn container_exists(name: &str) -> bool {
    Command::new("docker")
        .args(["container", "inspect", &format!("box-{}", name)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn container_is_running(name: &str) -> bool {
    let output = Command::new("docker")
        .args([
            "container",
            "inspect",
            "-f",
            "{{.State.Running}}",
            &format!("box-{}", name),
        ])
        .stderr(std::process::Stdio::null())
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim() == "true",
        _ => false,
    }
}

/// Return the set of session names whose containers are currently running.
pub fn running_sessions() -> std::collections::HashSet<String> {
    let output = Command::new("docker")
        .args(["ps", "--filter", "name=box-", "--format", "{{.Names}}"])
        .stderr(std::process::Stdio::null())
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix("box-"))
            .map(|s| s.to_string())
            .collect(),
        _ => std::collections::HashSet::new(),
    }
}

pub fn start_container(name: &str) -> Result<i32> {
    // Start container in background first, then attach separately.
    // This avoids the PTY size race condition that `docker start -ai` has,
    // where the terminal inside may not receive the correct dimensions.
    let status = Command::new("docker")
        .args(["start", &format!("box-{}", name)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()?;

    if !status.success() {
        return Ok(status.code().unwrap_or(1));
    }

    attach_container(name)
}

pub fn attach_container(name: &str) -> Result<i32> {
    let status = Command::new("docker")
        .args(["attach", &format!("box-{}", name)])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()?;
    restore_terminal();

    Ok(status.code().unwrap_or(1))
}

pub fn start_container_detached(name: &str) -> Result<i32> {
    let status = Command::new("docker")
        .args(["start", &format!("box-{}", name)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()?;

    if status.success() {
        println!("Container box-{} started in background.", name);
        println!("Run `box {}` to attach.", name);
        Ok(0)
    } else {
        Ok(status.code().unwrap_or(1))
    }
}

pub fn stop_container(name: &str) -> Result<i32> {
    let status = Command::new("docker")
        .args(["stop", &format!("box-{}", name)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()?;

    if status.success() {
        println!("Session '{}' stopped.", name);
        Ok(0)
    } else {
        Ok(status.code().unwrap_or(1))
    }
}

pub fn remove_container(name: &str) {
    let _ = Command::new("docker")
        .args(["rm", "-f", &format!("box-{}", name)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config<'a>() -> DockerRunConfig<'a> {
        DockerRunConfig {
            name: "sess",
            project_dir: "/tmp/project",
            image: "alpine:latest",
            mount_path: "/workspace",
            cmd: &[],
            env: &[],
            home: "/home/user",
            docker_args: None,
            ssh: false,
            detach: false,
        }
    }

    #[test]
    fn test_build_run_args_basic() {
        let args = build_run_args(&DockerRunConfig {
            name: "test-session",
            ..default_config()
        })
        .unwrap();

        assert_eq!(args[0], "run");
        assert_eq!(args[1], "-it");
        assert_eq!(args[2], "--name");
        assert_eq!(args[3], "box-test-session");
        assert_eq!(args[4], "--hostname");
        assert_eq!(args[5], "box-test-session");
        assert_eq!(args[6], "-v");
        assert_eq!(
            args[7],
            "/home/user/.box/workspaces/test-session:/workspace"
        );
        assert_eq!(args[8], "-w");
        assert_eq!(args[9], "/workspace");
        // image
        assert_eq!(args[10], "alpine:latest");
        assert_eq!(args.len(), 11);
    }

    #[test]
    fn test_build_run_args_with_command() {
        let cmd = vec!["bash".to_string()];
        let args = build_run_args(&DockerRunConfig {
            image: "ubuntu:latest",
            cmd: &cmd,
            ..default_config()
        })
        .unwrap();

        // Command follows image directly
        let image_pos = args.iter().position(|a| a == "ubuntu:latest").unwrap();
        assert_eq!(args[image_pos + 1], "bash");
        assert_eq!(args.len(), image_pos + 2);
    }

    #[test]
    fn test_build_run_args_with_multi_command() {
        let cmd = vec!["python".to_string(), "-m".to_string(), "pytest".to_string()];
        let args = build_run_args(&DockerRunConfig {
            image: "python:3.11",
            mount_path: "/app",
            cmd: &cmd,
            ..default_config()
        })
        .unwrap();

        let image_pos = args.iter().position(|a| a == "python:3.11").unwrap();
        assert_eq!(args[image_pos + 1], "python");
        assert_eq!(args[image_pos + 2], "-m");
        assert_eq!(args[image_pos + 3], "pytest");
        assert_eq!(args.len(), image_pos + 4);
    }

    #[test]
    fn test_build_run_args_with_docker_args() {
        let args = build_run_args(&DockerRunConfig {
            docker_args: Some("--network host -v /data:/data:ro"),
            ..default_config()
        })
        .unwrap();

        assert!(args.contains(&"--network".to_string()));
        assert!(args.contains(&"host".to_string()));
        assert!(args.contains(&"/data:/data:ro".to_string()));
    }

    #[test]
    fn test_build_run_args_docker_args_with_quotes() {
        let args = build_run_args(&DockerRunConfig {
            docker_args: Some("-e 'FOO=hello world'"),
            ..default_config()
        })
        .unwrap();

        assert!(args.contains(&"-e".to_string()));
        assert!(args.contains(&"FOO=hello world".to_string()));
    }

    #[test]
    fn test_build_run_args_empty_docker_args() {
        let args = build_run_args(&DockerRunConfig {
            docker_args: Some(""),
            ..default_config()
        })
        .unwrap();

        // Should still work, just no extra args
        assert!(args.contains(&"alpine:latest".to_string()));
    }

    #[test]
    fn test_build_run_args_invalid_docker_args() {
        let result = build_run_args(&DockerRunConfig {
            docker_args: Some("--flag 'unclosed quote"),
            ..default_config()
        });

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse docker args"));
    }

    #[test]
    fn test_build_run_args_custom_mount_path() {
        let args = build_run_args(&DockerRunConfig {
            mount_path: "/src",
            ..default_config()
        })
        .unwrap();

        assert!(args.contains(&"/home/user/.box/workspaces/sess:/src".to_string()));
        assert!(args.contains(&"/src".to_string()));
    }

    #[test]
    fn test_build_run_args_hostname() {
        let args = build_run_args(&DockerRunConfig {
            name: "my-session",
            ..default_config()
        })
        .unwrap();

        assert!(args.contains(&"box-my-session".to_string()));
    }

    #[test]
    fn test_build_run_args_no_rm_flag() {
        let args = build_run_args(&default_config()).unwrap();

        assert!(!args.contains(&"--rm".to_string()));
    }

    #[test]
    fn test_build_run_args_has_name() {
        let args = build_run_args(&DockerRunConfig {
            name: "my-session",
            ..default_config()
        })
        .unwrap();

        let name_pos = args.iter().position(|a| a == "--name").unwrap();
        assert_eq!(args[name_pos + 1], "box-my-session");
    }

    #[test]
    fn test_build_run_args_with_env() {
        let env = vec!["FOO=bar".to_string(), "BAZ".to_string()];
        let args = build_run_args(&DockerRunConfig {
            env: &env,
            ..default_config()
        })
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
        let args = build_run_args(&default_config()).unwrap();

        // No -e flags should be present
        assert!(!args.iter().any(|a| a == "-e"));
    }

    #[test]
    fn test_build_run_args_detached() {
        let args = build_run_args(&DockerRunConfig {
            detach: true,
            ..default_config()
        })
        .unwrap();

        assert_eq!(args[0], "run");
        assert_eq!(args[1], "-d");
        assert!(!args.contains(&"-it".to_string()));
    }

    #[test]
    fn test_build_run_args_detached_with_command() {
        let cmd = vec!["sleep".to_string(), "60".to_string()];
        let args = build_run_args(&DockerRunConfig {
            detach: true,
            cmd: &cmd,
            ..default_config()
        })
        .unwrap();

        assert_eq!(args[1], "-d");
        let image_pos = args.iter().position(|a| a == "alpine:latest").unwrap();
        assert_eq!(args[image_pos + 1], "sleep");
        assert_eq!(args[image_pos + 2], "60");
    }

    #[test]
    fn test_build_run_args_with_ssh() {
        unsafe { std::env::set_var("SSH_AUTH_SOCK", "/tmp/fake-ssh-agent.sock") };
        let args = build_run_args(&DockerRunConfig {
            ssh: true,
            ..default_config()
        })
        .unwrap();

        // Should have volume mount for the SSH socket
        let (host_path, container_path) = ssh_agent_paths().unwrap();
        let vol_mount = format!("{}:{}", host_path, container_path);
        let vol_pos = args
            .iter()
            .position(|a| a.contains("ssh-auth.sock"))
            .expect("SSH socket volume mount not found");
        assert_eq!(args[vol_pos - 1], "-v");
        assert_eq!(args[vol_pos], vol_mount);

        // Should have SSH_AUTH_SOCK env var
        let env_pos = args
            .iter()
            .position(|a| a.starts_with("SSH_AUTH_SOCK="))
            .expect("SSH_AUTH_SOCK env var not found");
        assert_eq!(args[env_pos - 1], "-e");
        assert_eq!(
            args[env_pos],
            format!("SSH_AUTH_SOCK={}", SSH_CONTAINER_PATH)
        );
    }
}
