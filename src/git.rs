use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// Name of the main repository (directory basename of the main working tree).
    pub repo_name: String,
    /// Worktree identifier: "main" for the primary working tree, or the directory
    /// basename for linked worktrees.
    pub worktree_name: String,
    /// Absolute path to the main repository root (for resolving seed DB paths).
    pub main_root: Option<PathBuf>,
}

impl WorktreeInfo {
    pub fn is_linked(&self) -> bool {
        self.main_root.is_some()
    }
}

/// Detect worktree identity by inspecting the `.git` entry at `repo_root`.
///
/// - If `.git` is a directory → this is the main working tree.
/// - If `.git` is a file → it contains `gitdir: <path>` pointing to
///   `.git/worktrees/<name>` in the main repo, making this a linked worktree.
/// - If neither exists → fallback to directory name with worktree_name="main".
pub fn worktree_info(repo_root: &Path) -> WorktreeInfo {
    let dot_git = repo_root.join(".git");

    if dot_git.is_dir() {
        // Main working tree
        return WorktreeInfo {
            repo_name: dir_name(repo_root),
            worktree_name: "main".into(),
            main_root: None,
        };
    }

    if dot_git.is_file() {
        if let Some(info) = parse_linked_worktree(&dot_git, repo_root) {
            return info;
        }
    }

    // Not a git repo or unrecognizable structure
    WorktreeInfo {
        repo_name: dir_name(repo_root),
        worktree_name: "main".into(),
        main_root: None,
    }
}

/// Parse a `.git` file to extract linked worktree information.
///
/// A linked worktree's `.git` file contains a line like:
///   gitdir: /path/to/main-repo/.git/worktrees/<worktree-name>
fn parse_linked_worktree(dot_git_file: &Path, repo_root: &Path) -> Option<WorktreeInfo> {
    let content = std::fs::read_to_string(dot_git_file).ok()?;
    let gitdir_line = content.trim();
    let gitdir_path = gitdir_line.strip_prefix("gitdir: ")?;

    // Resolve relative gitdir paths against repo_root
    let gitdir = if Path::new(gitdir_path).is_absolute() {
        PathBuf::from(gitdir_path)
    } else {
        repo_root.join(gitdir_path)
    };

    let gitdir = gitdir.canonicalize().ok()?;

    // Expected structure: <main-repo>/.git/worktrees/<name>
    // Walk up: gitdir parent = "worktrees", grandparent = ".git", great-grandparent = main repo
    let worktrees_dir = gitdir.parent()?;
    if worktrees_dir.file_name()?.to_str()? != "worktrees" {
        return None;
    }
    let git_dir = worktrees_dir.parent()?;
    if git_dir.file_name()?.to_str()? != ".git" {
        return None;
    }
    let main_repo_root = git_dir.parent()?;

    // Use Git's stable worktree ID (the directory name under .git/worktrees/<id>)
    // rather than the checkout directory basename, which could collide or be "main".
    let worktree_id = gitdir.file_name()?.to_str()?.to_owned();

    Some(WorktreeInfo {
        repo_name: dir_name(main_repo_root),
        worktree_name: worktree_id,
        main_root: Some(main_repo_root.to_path_buf()),
    })
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_main_working_tree() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("my-repo");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();

        let info = worktree_info(&repo);
        assert_eq!(info.repo_name, "my-repo");
        assert_eq!(info.worktree_name, "main");
        assert!(!info.is_linked());
        assert!(info.main_root.is_none());
    }

    #[test]
    fn test_linked_worktree() {
        let dir = TempDir::new().unwrap();

        // Set up main repo with .git/worktrees/feat-branch/
        let main_repo = dir.path().join("my-repo");
        let git_dir = main_repo.join(".git");
        let wt_git_dir = git_dir.join("worktrees").join("feat-branch");
        std::fs::create_dir_all(&wt_git_dir).unwrap();

        // Set up linked worktree with .git file
        let wt_dir = dir.path().join("feat-branch");
        std::fs::create_dir(&wt_dir).unwrap();
        std::fs::write(
            wt_dir.join(".git"),
            format!("gitdir: {}", wt_git_dir.display()),
        )
        .unwrap();

        let info = worktree_info(&wt_dir);
        assert_eq!(info.repo_name, "my-repo");
        assert_eq!(info.worktree_name, "feat-branch");
        assert!(info.is_linked());
        // Compare canonicalized paths to handle macOS /var -> /private/var symlink
        let expected = main_repo.canonicalize().unwrap();
        let actual = info.main_root.unwrap().canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_linked_worktree_relative_gitdir() {
        let dir = TempDir::new().unwrap();

        // Main repo at dir/main-repo
        let main_repo = dir.path().join("main-repo");
        let git_dir = main_repo.join(".git");
        let wt_git_dir = git_dir.join("worktrees").join("my-wt");
        std::fs::create_dir_all(&wt_git_dir).unwrap();

        // Linked worktree as sibling: dir/my-wt
        // Relative path from my-wt to main-repo/.git/worktrees/my-wt is:
        //   ../main-repo/.git/worktrees/my-wt
        let wt_dir = dir.path().join("my-wt");
        std::fs::create_dir(&wt_dir).unwrap();
        std::fs::write(
            wt_dir.join(".git"),
            "gitdir: ../main-repo/.git/worktrees/my-wt",
        )
        .unwrap();

        let info = worktree_info(&wt_dir);
        assert_eq!(info.repo_name, "main-repo");
        assert_eq!(info.worktree_name, "my-wt");
        assert!(info.is_linked());
    }

    #[test]
    fn test_non_git_directory() {
        let dir = TempDir::new().unwrap();
        let plain = dir.path().join("plain-dir");
        std::fs::create_dir(&plain).unwrap();

        let info = worktree_info(&plain);
        assert_eq!(info.repo_name, "plain-dir");
        assert_eq!(info.worktree_name, "main");
        assert!(!info.is_linked());
    }

    #[test]
    fn test_malformed_git_file() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("bad-repo");
        std::fs::create_dir(&repo).unwrap();
        std::fs::write(repo.join(".git"), "not a valid gitdir line").unwrap();

        let info = worktree_info(&repo);
        assert_eq!(info.repo_name, "bad-repo");
        assert_eq!(info.worktree_name, "main");
    }

    #[test]
    fn test_identity_is_stable() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("stable-repo");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();

        let i1 = worktree_info(&repo);
        let i2 = worktree_info(&repo);
        assert_eq!(i1, i2);
    }

    #[test]
    fn test_root_path_fallback() {
        let info = super::dir_name(Path::new("/"));
        assert_eq!(info, "unknown");
    }
}
