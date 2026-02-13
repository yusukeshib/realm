use std::path::Path;

pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Walk up from `dir` to find the nearest ancestor containing `.git`.
pub fn find_root(dir: &Path) -> Option<&Path> {
    let mut current = dir;
    loop {
        if is_repo(current) {
            return Some(current);
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_repo_true() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(is_repo(tmp.path()));
    }

    #[test]
    fn test_is_repo_git_file() {
        // Worktrees and submodules use a .git file instead of a directory
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".git"), "gitdir: /some/path").unwrap();
        assert!(is_repo(tmp.path()));
    }

    #[test]
    fn test_is_repo_false() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_repo(tmp.path()));
    }

    #[test]
    fn test_is_repo_nonexistent() {
        assert!(!is_repo(Path::new("/nonexistent/path/12345")));
    }

    #[test]
    fn test_find_root_at_root() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(find_root(tmp.path()), Some(tmp.path()));
    }

    #[test]
    fn test_find_root_from_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        let sub = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(find_root(&sub), Some(tmp.path()));
    }

    #[test]
    fn test_find_root_no_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("no_repo");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(find_root(&sub), None);
    }
}
