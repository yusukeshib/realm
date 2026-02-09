use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::fs;
use std::path::PathBuf;

use crate::config;

#[derive(Debug, Clone)]
pub struct Session {
    pub name: String,
    pub project_dir: String,
    pub image: String,
    pub mount_path: String,
    pub command: Vec<String>,
    pub env: Vec<String>,
    pub ssh: bool,
}

impl From<config::RealmConfig> for Session {
    fn from(cfg: config::RealmConfig) -> Self {
        Session {
            name: cfg.name,
            project_dir: cfg.project_dir,
            image: cfg.image,
            mount_path: cfg.mount_path,
            command: cfg.command,
            env: cfg.env,
            ssh: cfg.ssh,
        }
    }
}

pub struct SessionSummary {
    pub name: String,
    pub project_dir: String,
    pub image: String,
    pub created_at: String,
    pub running: bool,
}

pub fn sessions_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(config::home_dir()?)
        .join(".realm")
        .join("sessions"))
}

const RESERVED_NAMES: &[&str] = &["upgrade"];

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Session name is required.");
    }
    if RESERVED_NAMES.contains(&name) {
        bail!(
            "'{}' is a reserved name and cannot be used as a session name.",
            name
        );
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!(
            "Invalid session name '{}'. Use only letters, digits, hyphens, and underscores.",
            name
        );
    }
    Ok(())
}

pub fn session_exists(name: &str) -> bool {
    match sessions_dir() {
        Ok(dir) => dir.join(name).is_dir(),
        Err(e) => {
            eprintln!(
                "Failed to determine sessions directory while checking if session '{}' exists: {}",
                name, e
            );
            false
        }
    }
}

pub fn save(session: &Session) -> Result<()> {
    let dir = sessions_dir()?.join(&session.name);
    fs::create_dir_all(&dir).context("Failed to create session directory")?;

    fs::write(dir.join("project_dir"), &session.project_dir)?;
    fs::write(dir.join("image"), &session.image)?;
    fs::write(dir.join("mount_path"), &session.mount_path)?;
    fs::write(
        dir.join("created_at"),
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    )?;
    if !session.command.is_empty() {
        let content: Vec<&str> = session.command.iter().map(|s| s.as_str()).collect();
        fs::write(dir.join("command"), content.join("\0"))?;
    } else {
        let _ = fs::remove_file(dir.join("command"));
    }
    if !session.env.is_empty() {
        let content: Vec<&str> = session.env.iter().map(|s| s.as_str()).collect();
        fs::write(dir.join("env"), content.join("\0"))?;
    } else {
        let _ = fs::remove_file(dir.join("env"));
    }
    if session.ssh {
        fs::write(dir.join("ssh"), "true")?;
    } else {
        let _ = fs::remove_file(dir.join("ssh"));
    }

    Ok(())
}

pub fn load(name: &str) -> Result<Session> {
    let dir = sessions_dir()?.join(name);
    if !dir.is_dir() {
        bail!("Session '{}' not found.", name);
    }

    let project_dir_path = dir.join("project_dir");
    if !project_dir_path.exists() {
        bail!("Session '{}' is missing project directory metadata.", name);
    }
    let project_dir = fs::read_to_string(&project_dir_path)?.trim().to_string();

    let image = fs::read_to_string(dir.join("image"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| config::DEFAULT_IMAGE.to_string());

    let mount_path = fs::read_to_string(dir.join("mount_path"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| config::derive_mount_path(&project_dir));

    let command = fs::read_to_string(dir.join("command"))
        .map(|s| {
            s.split('\0')
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default();

    let env = fs::read_to_string(dir.join("env"))
        .map(|s| {
            s.split('\0')
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default();

    let ssh = dir.join("ssh").exists();

    Ok(Session {
        name: name.to_string(),
        project_dir,
        image,
        mount_path,
        command,
        env,
        ssh,
    })
}

pub fn list() -> Result<Vec<SessionSummary>> {
    let dir = sessions_dir()?;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let session_path = entry.path();

        let project_dir = fs::read_to_string(session_path.join("project_dir"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let image = fs::read_to_string(session_path.join("image"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let created_at = fs::read_to_string(session_path.join("created_at"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        sessions.push(SessionSummary {
            name,
            project_dir,
            image,
            created_at,
            running: false,
        });
    }

    Ok(sessions)
}

pub fn remove_dir(name: &str) -> Result<()> {
    let dir = sessions_dir()?.join(name);
    fs::remove_dir_all(&dir).context(format!("Failed to remove session directory for '{}'", name))
}

pub fn touch_resumed_at(name: &str) -> Result<()> {
    let dir = sessions_dir()?.join(name);
    fs::write(
        dir.join("resumed_at"),
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize tests that mutate HOME env var
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_home<F: FnOnce(&std::path::Path)>(f: F) {
        let _lock = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        f(tmp.path());
        match old_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_validate_name_valid() {
        assert!(validate_name("my-session").is_ok());
        assert!(validate_name("test_123").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("ABC").is_ok());
        assert!(validate_name("hello-world_99").is_ok());
    }

    #[test]
    fn test_validate_name_empty() {
        let err = validate_name("").unwrap_err();
        assert_eq!(err.to_string(), "Session name is required.");
    }

    #[test]
    fn test_validate_name_reserved() {
        let err = validate_name("upgrade").unwrap_err();
        assert!(err.to_string().contains("reserved name"));
    }

    #[test]
    fn test_validate_name_invalid_chars() {
        let err = validate_name("bad name").unwrap_err();
        assert!(err.to_string().contains("Invalid session name"));
        assert!(err.to_string().contains("bad name"));

        let err = validate_name("bad/name").unwrap_err();
        assert!(err.to_string().contains("Invalid session name"));

        let err = validate_name("bad.name").unwrap_err();
        assert!(err.to_string().contains("Invalid session name"));

        let err = validate_name("bad@name").unwrap_err();
        assert!(err.to_string().contains("Invalid session name"));
    }

    #[test]
    fn test_sessions_dir() {
        with_temp_home(|tmp| {
            let dir = sessions_dir().unwrap();
            assert_eq!(dir, tmp.join(".realm").join("sessions"));
        });
    }

    #[test]
    fn test_save_and_load_basic() {
        with_temp_home(|_| {
            let sess = Session {
                name: "test-session".to_string(),
                project_dir: "/tmp/myproject".to_string(),
                image: "ubuntu:latest".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec![],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();

            let loaded = load("test-session").unwrap();
            assert_eq!(loaded.name, "test-session");
            assert_eq!(loaded.project_dir, "/tmp/myproject");
            assert_eq!(loaded.image, "ubuntu:latest");
            assert_eq!(loaded.mount_path, "/workspace");
            assert!(loaded.command.is_empty());
        });
    }

    #[test]
    fn test_save_and_load_with_command() {
        with_temp_home(|_| {
            let sess = Session {
                name: "full-session".to_string(),
                project_dir: "/tmp/project".to_string(),
                image: "realm-full:latest".to_string(),
                mount_path: "/src".to_string(),
                command: vec![
                    "bash".to_string(),
                    "-c".to_string(),
                    "echo hello".to_string(),
                ],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();

            let loaded = load("full-session").unwrap();
            assert_eq!(loaded.command, vec!["bash", "-c", "echo hello"]);
        });
    }

    #[test]
    fn test_save_creates_metadata_files() {
        with_temp_home(|_| {
            let sess = Session {
                name: "meta-test".to_string(),
                project_dir: "/tmp/p".to_string(),
                image: "alpine:latest".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec![],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();

            let dir = sessions_dir().unwrap().join("meta-test");
            assert!(dir.join("project_dir").exists());
            assert!(dir.join("image").exists());
            assert!(dir.join("mount_path").exists());
            assert!(dir.join("created_at").exists());
            assert!(!dir.join("command").exists());

            let created = fs::read_to_string(dir.join("created_at")).unwrap();
            assert!(created.ends_with("UTC"));
        });
    }

    #[test]
    fn test_load_nonexistent() {
        with_temp_home(|_| {
            let err = load("nonexistent").unwrap_err();
            assert_eq!(err.to_string(), "Session 'nonexistent' not found.");
        });
    }

    #[test]
    fn test_load_missing_project_dir() {
        with_temp_home(|_| {
            let dir = sessions_dir().unwrap().join("broken");
            fs::create_dir_all(&dir).unwrap();
            // Don't write project_dir file

            let err = load("broken").unwrap_err();
            assert!(err
                .to_string()
                .contains("missing project directory metadata"));
        });
    }

    #[test]
    fn test_load_defaults_when_optional_files_missing() {
        with_temp_home(|_| {
            let dir = sessions_dir().unwrap().join("minimal");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("project_dir"), "/tmp/project").unwrap();
            // Don't write image or mount_path

            let loaded = load("minimal").unwrap();
            assert_eq!(loaded.image, config::DEFAULT_IMAGE);
            assert_eq!(loaded.mount_path, config::derive_mount_path("/tmp/project"));
        });
    }

    #[test]
    fn test_session_exists() {
        with_temp_home(|_| {
            assert!(!session_exists("nope"));

            let sess = Session {
                name: "exists-test".to_string(),
                project_dir: "/tmp/p".to_string(),
                image: "alpine:latest".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec![],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();
            assert!(session_exists("exists-test"));
        });
    }

    #[test]
    fn test_list_empty() {
        with_temp_home(|_| {
            let sessions = list().unwrap();
            assert!(sessions.is_empty());
        });
    }

    #[test]
    fn test_list_multiple_sessions() {
        with_temp_home(|_| {
            for name in &["alpha", "beta", "gamma"] {
                let sess = Session {
                    name: name.to_string(),
                    project_dir: format!("/tmp/{}", name),
                    image: "alpine:latest".to_string(),
                    mount_path: "/workspace".to_string(),
                    command: vec![],
                    env: vec![],
                    ssh: false,
                };
                save(&sess).unwrap();
            }

            let sessions = list().unwrap();
            assert_eq!(sessions.len(), 3);
            // Should be sorted alphabetically
            assert_eq!(sessions[0].name, "alpha");
            assert_eq!(sessions[1].name, "beta");
            assert_eq!(sessions[2].name, "gamma");
        });
    }

    #[test]
    fn test_list_reads_metadata() {
        with_temp_home(|_| {
            let sess = Session {
                name: "list-meta".to_string(),
                project_dir: "/home/user/project".to_string(),
                image: "ubuntu:22.04".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec![],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();

            let sessions = list().unwrap();
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].project_dir, "/home/user/project");
            assert_eq!(sessions[0].image, "ubuntu:22.04");
            assert!(!sessions[0].created_at.is_empty());
        });
    }

    #[test]
    fn test_remove_dir() {
        with_temp_home(|_| {
            let sess = Session {
                name: "to-remove".to_string(),
                project_dir: "/tmp/p".to_string(),
                image: "alpine:latest".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec![],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();
            assert!(session_exists("to-remove"));

            remove_dir("to-remove").unwrap();
            assert!(!session_exists("to-remove"));
        });
    }

    #[test]
    fn test_remove_dir_nonexistent() {
        with_temp_home(|_| {
            let err = remove_dir("nonexistent").unwrap_err();
            assert!(err.to_string().contains("Failed to remove"));
        });
    }

    #[test]
    fn test_touch_resumed_at() {
        with_temp_home(|_| {
            let sess = Session {
                name: "resume-test".to_string(),
                project_dir: "/tmp/p".to_string(),
                image: "alpine:latest".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec![],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();

            touch_resumed_at("resume-test").unwrap();

            let dir = sessions_dir().unwrap().join("resume-test");
            let content = fs::read_to_string(dir.join("resumed_at")).unwrap();
            assert!(content.ends_with("UTC"));
        });
    }

    #[test]
    fn test_save_trims_whitespace_on_load() {
        with_temp_home(|_| {
            let dir = sessions_dir().unwrap().join("trim-test");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("project_dir"), "  /tmp/project  \n").unwrap();
            fs::write(dir.join("image"), " ubuntu:latest \n").unwrap();
            fs::write(dir.join("mount_path"), " /src \n").unwrap();

            let loaded = load("trim-test").unwrap();
            assert_eq!(loaded.project_dir, "/tmp/project");
            assert_eq!(loaded.image, "ubuntu:latest");
            assert_eq!(loaded.mount_path, "/src");
        });
    }

    #[test]
    fn test_command_save_format() {
        with_temp_home(|_| {
            let sess = Session {
                name: "cmd-format".to_string(),
                project_dir: "/tmp/p".to_string(),
                image: "alpine:latest".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec!["bash".to_string(), "-c".to_string(), "echo hi".to_string()],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();

            let dir = sessions_dir().unwrap().join("cmd-format");
            let raw = fs::read_to_string(dir.join("command")).unwrap();
            assert_eq!(raw, "bash\0-c\0echo hi");
        });
    }

    #[test]
    fn test_save_and_load_with_env() {
        with_temp_home(|_| {
            let sess = Session {
                name: "env-test".to_string(),
                project_dir: "/tmp/project".to_string(),
                image: "alpine:latest".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec![],
                env: vec!["FOO=bar".to_string(), "BAZ".to_string()],
                ssh: false,
            };
            save(&sess).unwrap();

            let loaded = load("env-test").unwrap();
            assert_eq!(loaded.env, vec!["FOO=bar", "BAZ"]);

            let dir = sessions_dir().unwrap().join("env-test");
            let raw = fs::read_to_string(dir.join("env")).unwrap();
            assert_eq!(raw, "FOO=bar\0BAZ");
        });
    }

    #[test]
    fn test_save_and_load_empty_env() {
        with_temp_home(|_| {
            let sess = Session {
                name: "no-env".to_string(),
                project_dir: "/tmp/project".to_string(),
                image: "alpine:latest".to_string(),
                mount_path: "/workspace".to_string(),
                command: vec![],
                env: vec![],
                ssh: false,
            };
            save(&sess).unwrap();

            let dir = sessions_dir().unwrap().join("no-env");
            assert!(!dir.join("env").exists());

            let loaded = load("no-env").unwrap();
            assert!(loaded.env.is_empty());
        });
    }
}
