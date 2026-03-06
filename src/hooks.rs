use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const HOOK_BEGIN: &str = "# BEGIN shire";
const HOOK_END: &str = "# END shire";

/// Detect the git hooks directory for a repository.
/// Priority: core.hooksPath > .githooks/ in repo root > .git/hooks/
pub fn detect_hooks_dir(repo_root: &Path) -> Result<PathBuf> {
    // 1. Check core.hooksPath
    if let Ok(output) = Command::new("git")
        .args(["config", "core.hooksPath"])
        .current_dir(repo_root)
        .output()
    {
        if output.status.success() {
            let hooks_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !hooks_path.is_empty() {
                let p = Path::new(&hooks_path);
                let resolved = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    repo_root.join(p)
                };
                if resolved.exists() && !resolved.is_dir() {
                    anyhow::bail!(
                        "core.hooksPath '{}' exists but is not a directory",
                        resolved.display()
                    );
                }
                return Ok(resolved);
            }
        }
    }

    // 2. Check .githooks/ convention
    let githooks = repo_root.join(".githooks");
    if githooks.is_dir() {
        return Ok(githooks);
    }

    // 3. Fall back to .git/hooks/
    let git_hooks = repo_root.join(".git").join("hooks");
    Ok(git_hooks)
}

/// Install or update the shire post-checkout hook in the given hooks directory.
pub fn install_post_checkout_hook(hooks_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(hooks_dir)
        .with_context(|| format!("Failed to create hooks directory '{}'", hooks_dir.display()))?;

    let hook_path = hooks_dir.join("post-checkout");
    let shire_block = format!(
        "{HOOK_BEGIN}\n\
         # Seed and build index for new worktrees\n\
         if ! shire worktree 2>/dev/null; then\n\
             : # Not a worktree or shire not installed — silently skip\n\
         fi\n\
         {HOOK_END}"
    );

    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path)
            .with_context(|| format!("Failed to read '{}'", hook_path.display()))?;
        if existing.contains(HOOK_BEGIN) {
            // Replace existing shire block
            let before = existing.split(HOOK_BEGIN).next().unwrap_or("");
            let after = existing
                .split(HOOK_END)
                .nth(1)
                .unwrap_or("");
            let updated = format!("{before}{shire_block}{after}");
            std::fs::write(&hook_path, updated)
                .with_context(|| format!("Failed to write '{}'", hook_path.display()))?;
        } else {
            // Append
            let updated = format!("{existing}\n{shire_block}\n");
            std::fs::write(&hook_path, updated)
                .with_context(|| format!("Failed to write '{}'", hook_path.display()))?;
        }
    } else {
        let content = format!("#!/bin/sh\n{shire_block}\n");
        std::fs::write(&hook_path, &content)
            .with_context(|| format!("Failed to write '{}'", hook_path.display()))?;
    }

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn git_init(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init failed");
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
    fn test_detect_hooks_dir_default() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_init(&repo);

        let hooks = detect_hooks_dir(&repo).unwrap();
        assert!(hooks.ends_with(".git/hooks"));
    }

    #[test]
    fn test_detect_hooks_dir_githooks_convention() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_init(&repo);
        std::fs::create_dir(repo.join(".githooks")).unwrap();

        let hooks = detect_hooks_dir(&repo).unwrap();
        assert!(hooks.ends_with(".githooks"));
    }

    #[test]
    fn test_detect_hooks_dir_core_hookspath() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_init(&repo);
        let custom = dir.path().join("custom-hooks");
        std::fs::create_dir(&custom).unwrap();
        Command::new("git")
            .args(["config", "core.hooksPath", custom.to_str().unwrap()])
            .current_dir(&repo)
            .output()
            .unwrap();

        let hooks = detect_hooks_dir(&repo).unwrap();
        assert_eq!(hooks, custom);
    }

    #[test]
    fn test_install_post_checkout_hook_new() {
        let dir = TempDir::new().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir(&hooks_dir).unwrap();

        install_post_checkout_hook(&hooks_dir).unwrap();

        let hook_path = hooks_dir.join("post-checkout");
        assert!(hook_path.exists());
        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains(HOOK_BEGIN));
        assert!(content.contains(HOOK_END));
        assert!(content.contains("shire worktree"));
    }

    #[test]
    fn test_install_post_checkout_hook_append() {
        let dir = TempDir::new().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir(&hooks_dir).unwrap();
        let hook_path = hooks_dir.join("post-checkout");
        std::fs::write(&hook_path, "#!/bin/sh\necho 'existing hook'\n").unwrap();

        install_post_checkout_hook(&hooks_dir).unwrap();

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("existing hook"));
        assert!(content.contains(HOOK_BEGIN));
        assert!(content.contains("shire worktree"));
    }

    #[test]
    fn test_install_post_checkout_hook_idempotent() {
        let dir = TempDir::new().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir(&hooks_dir).unwrap();

        install_post_checkout_hook(&hooks_dir).unwrap();
        install_post_checkout_hook(&hooks_dir).unwrap();

        let content = std::fs::read_to_string(hooks_dir.join("post-checkout")).unwrap();
        let count = content.matches(HOOK_BEGIN).count();
        assert_eq!(count, 1, "Hook block should appear exactly once");
    }
}
