pub const DEFAULT_IMAGE: &str = "alpine/git";

#[derive(Debug, Clone, PartialEq)]
pub struct RealmConfig {
    pub name: String,
    pub project_dir: String,
    pub image: String,
    pub mount_path: String,
    pub command: Vec<String>,
}

pub struct RealmConfigInput {
    pub name: String,
    pub image: Option<String>,
    pub mount_path: Option<String>,
    pub project_dir: String,
    pub command: Vec<String>,
}

pub fn resolve(input: RealmConfigInput) -> RealmConfig {
    let mount_path = input
        .mount_path
        .unwrap_or_else(|| derive_mount_path(&input.project_dir));
    let image = input.image.unwrap_or_else(|| {
        std::env::var("REALM_DEFAULT_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.to_string())
    });

    RealmConfig {
        name: input.name,
        project_dir: input.project_dir,
        image,
        mount_path,
        command: input.command,
    }
}

pub fn derive_mount_path(project_dir: &str) -> String {
    let trimmed = project_dir.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/workspace".to_string();
    }
    match trimmed.rsplit('/').next() {
        Some(name) if !name.is_empty() => format!("/{}", name),
        _ => "/workspace".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_mount_path_normal() {
        assert_eq!(derive_mount_path("/home/user/realm"), "/realm");
    }

    #[test]
    fn test_derive_mount_path_nested() {
        assert_eq!(derive_mount_path("/home/user/projects/myapp"), "/myapp");
    }

    #[test]
    fn test_derive_mount_path_root_fallback() {
        assert_eq!(derive_mount_path("/"), "/workspace");
    }

    #[test]
    fn test_derive_mount_path_trailing_slash() {
        assert_eq!(derive_mount_path("/home/user/realm/"), "/realm");
    }

    #[test]
    fn test_derive_mount_path_single_component() {
        assert_eq!(derive_mount_path("/myproject"), "/myproject");
    }

    #[test]
    fn test_resolve_defaults() {
        let config = resolve(RealmConfigInput {
            name: "test".to_string(),
            image: None,

            mount_path: None,
            project_dir: "/home/user/myproject".to_string(),
            command: vec![],
        });

        assert_eq!(
            config,
            RealmConfig {
                name: "test".to_string(),
                project_dir: "/home/user/myproject".to_string(),
                image: DEFAULT_IMAGE.to_string(),
                mount_path: "/myproject".to_string(),

                command: vec![],
            }
        );
    }

    #[test]
    fn test_resolve_mount_override() {
        let config = resolve(RealmConfigInput {
            name: "test".to_string(),
            image: None,

            mount_path: Some("/custom".to_string()),
            project_dir: "/home/user/myproject".to_string(),
            command: vec![],
        });

        assert_eq!(config.mount_path, "/custom");
    }

    #[test]
    fn test_resolve_image_override() {
        let config = resolve(RealmConfigInput {
            name: "test".to_string(),
            image: Some("ubuntu:latest".to_string()),

            mount_path: None,
            project_dir: "/home/user/myproject".to_string(),
            command: vec![],
        });

        assert_eq!(config.image, "ubuntu:latest");
    }

    #[test]
    fn test_resolve_env_default_image() {
        std::env::set_var("REALM_DEFAULT_IMAGE", "ubuntu:latest");
        let config = resolve(RealmConfigInput {
            name: "test".to_string(),
            image: None,
            mount_path: None,
            project_dir: "/home/user/myproject".to_string(),
            command: vec![],
        });
        assert_eq!(config.image, "ubuntu:latest");
        std::env::remove_var("REALM_DEFAULT_IMAGE");
    }

    #[test]
    fn test_resolve_image_flag_overrides_env() {
        std::env::set_var("REALM_DEFAULT_IMAGE", "ubuntu:latest");
        let config = resolve(RealmConfigInput {
            name: "test".to_string(),
            image: Some("python:3.11".to_string()),
            mount_path: None,
            project_dir: "/home/user/myproject".to_string(),
            command: vec![],
        });
        assert_eq!(config.image, "python:3.11");
        std::env::remove_var("REALM_DEFAULT_IMAGE");
    }

    #[test]
    fn test_resolve_full() {
        let config = resolve(RealmConfigInput {
            name: "full".to_string(),
            image: Some("python:3.11".to_string()),
            mount_path: Some("/app".to_string()),
            project_dir: "/home/user/project".to_string(),
            command: vec!["python".to_string(), "main.py".to_string()],
        });

        assert_eq!(
            config,
            RealmConfig {
                name: "full".to_string(),
                project_dir: "/home/user/project".to_string(),
                image: "python:3.11".to_string(),
                mount_path: "/app".to_string(),
                command: vec!["python".to_string(), "main.py".to_string()],
            }
        );
    }
}
