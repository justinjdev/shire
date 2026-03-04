use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIdentity {
    pub repo_name: String,
    pub worktree_name: String,
}

/// Determine the repository name and worktree identifier for a given path.
///
/// Uses `git rev-parse` to detect whether the path is a main working tree or
/// a linked worktree, and resolves the main repo name in either case.
///
/// Falls back to `{ dir_name, "main" }` when git is unavailable or the path
/// is not inside a git repository.
pub fn repo_identity(path: &Path) -> RepoIdentity {
    match detect_git_identity(path) {
        Some(identity) => identity,
        None => fallback_identity(path),
    }
}

fn detect_git_identity(path: &Path) -> Option<RepoIdentity> {
    // Get the worktree root (toplevel of whichever worktree we're in)
    let toplevel = match Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                return None; // Not a git repo — expected
            }
            output
        }
        Err(e) => {
            eprintln!("Warning: could not run git: {e}");
            return None;
        }
    };
    let toplevel_str = match std::str::from_utf8(&toplevel.stdout) {
        Ok(s) => s.trim(),
        Err(e) => {
            eprintln!("Warning: git rev-parse --show-toplevel returned non-UTF-8 output: {e}");
            return None;
        }
    };
    let toplevel_path = Path::new(toplevel_str);

    // Get the common git dir (shared .git directory for all worktrees)
    let common_dir = match Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(path)
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                eprintln!("Warning: git rev-parse --git-common-dir failed");
                return None;
            }
            output
        }
        Err(e) => {
            eprintln!("Warning: could not run git: {e}");
            return None;
        }
    };
    let common_dir_raw = match std::str::from_utf8(&common_dir.stdout) {
        Ok(s) => s.trim(),
        Err(e) => {
            eprintln!("Warning: git rev-parse --git-common-dir returned non-UTF-8 output: {e}");
            return None;
        }
    };

    // Resolve the common dir relative to the current directory (where git ran)
    let common_dir_path = if Path::new(common_dir_raw).is_absolute() {
        std::path::PathBuf::from(common_dir_raw)
    } else {
        path.join(common_dir_raw)
    };
    let common_dir_path = match common_dir_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Warning: could not canonicalize git common dir '{}': {e}",
                common_dir_path.display()
            );
            return None;
        }
    };

    // The main repo root is the parent of the common .git dir
    let main_repo_root = common_dir_path.parent()?;
    let repo_name = main_repo_root
        .file_name()?
        .to_string_lossy()
        .into_owned();

    // Determine worktree name: if toplevel == main repo root, it's "main";
    // otherwise use the basename of the worktree directory
    let toplevel_canon = match toplevel_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Warning: could not canonicalize worktree path '{}': {e}",
                toplevel_path.display()
            );
            return None;
        }
    };
    let main_canon = match main_repo_root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Warning: could not canonicalize main repo path '{}': {e}",
                main_repo_root.display()
            );
            return None;
        }
    };

    let worktree_name = if toplevel_canon == main_canon {
        "main".to_string()
    } else {
        toplevel_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "main".to_string())
    };

    Some(RepoIdentity {
        repo_name,
        worktree_name,
    })
}

fn fallback_identity(path: &Path) -> RepoIdentity {
    let repo_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());
    RepoIdentity {
        repo_name,
        worktree_name: "main".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git_init(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init failed");
        // Need at least one commit for worktrees to work
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .expect("git commit failed");
    }

    #[test]
    fn test_main_working_tree() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("my-repo");
        std::fs::create_dir(&repo).unwrap();
        git_init(&repo);

        let identity = repo_identity(&repo);
        assert_eq!(identity.repo_name, "my-repo");
        assert_eq!(identity.worktree_name, "main");
    }

    #[test]
    fn test_linked_worktree() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("my-repo");
        std::fs::create_dir(&repo).unwrap();
        git_init(&repo);

        // Create a linked worktree
        let wt_path = dir.path().join("my-worktree");
        let output = Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "feat-branch"])
            .current_dir(&repo)
            .output()
            .expect("git worktree add failed");
        assert!(output.status.success(), "git worktree add failed: {}", String::from_utf8_lossy(&output.stderr));

        let identity = repo_identity(&wt_path);
        assert_eq!(identity.repo_name, "my-repo");
        assert_eq!(identity.worktree_name, "my-worktree");
    }

    #[test]
    fn test_non_git_directory() {
        let dir = TempDir::new().unwrap();
        let non_git = dir.path().join("plain-dir");
        std::fs::create_dir(&non_git).unwrap();

        let identity = repo_identity(&non_git);
        assert_eq!(identity.repo_name, "plain-dir");
        assert_eq!(identity.worktree_name, "main");
    }

    #[test]
    fn test_identity_is_stable() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("stable-repo");
        std::fs::create_dir(&repo).unwrap();
        git_init(&repo);

        let id1 = repo_identity(&repo);
        let id2 = repo_identity(&repo);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_fallback_identity_root_path() {
        let identity = super::fallback_identity(Path::new("/"));
        assert_eq!(identity.repo_name, "unknown");
        assert_eq!(identity.worktree_name, "main");
    }
}
