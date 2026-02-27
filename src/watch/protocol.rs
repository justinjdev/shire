use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Message sent over UDS to signal a rebuild.
#[derive(Debug, Serialize, Deserialize)]
pub struct RebuildMessage {
    #[serde(default)]
    pub files: Vec<PathBuf>,
}

/// Claude Code hook JSON received on stdin for PostToolUse events.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub tool_name: Option<String>,
    pub tool_input: ToolInput,
    /// Working directory of the Claude Code session (repo root).
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct ToolInput {
    pub file_path: Option<PathBuf>,
    /// NotebookEdit uses notebook_path instead of file_path
    pub notebook_path: Option<PathBuf>,
    /// Bash tool command string
    pub command: Option<String>,
}

/// Bash commands known to be read-only. If every segment of a piped/chained
/// command starts with one of these, we skip the rebuild.
const READONLY_COMMANDS: &[&str] = &[
    "cat", "head", "tail", "less", "more",
    "ls", "dir", "find", "fd", "tree",
    "grep", "rg", "ag", "ack",
    "wc", "diff", "cmp", "file", "stat",
    "echo", "printf", "true", "false",
    "pwd", "which", "whereis", "whence", "type", "command",
    "env", "printenv", "set",
    "ps", "top", "htop", "uptime", "df", "du", "free",
    "date", "cal",
    "man", "help", "info",
    "git status", "git log", "git diff", "git show", "git branch",
    "git remote", "git tag", "git stash list", "git rev-parse",
    "cargo test", "cargo check", "cargo clippy", "cargo bench", "cargo doc",
    "cargo build",
    "go test", "go vet", "go build",
    "npm test", "npm run test", "npm run lint", "npm run build",
    "npx", "yarn test", "pnpm test",
    "python -c", "python -m pytest", "pytest", "node -e",
    "make check", "make test",
    "jq", "yq", "xargs",
    "curl", "wget", "http",
    "docker ps", "docker images", "docker logs",
    "kubectl get", "kubectl describe", "kubectl logs",
    "gh pr view", "gh issue view", "gh api", "gh run view",
];

impl HookInput {
    /// Parse Claude Code hook JSON from stdin.
    /// Returns None if parsing fails (non-fatal — caller falls back to empty file list).
    pub fn from_stdin() -> Option<Self> {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).ok()?;
        serde_json::from_str(&buf).ok()
    }

    /// Whether this hook event should trigger a rebuild signal.
    /// For Bash: returns false only for commands known to be read-only.
    /// Unknown commands default to triggering a rebuild (safe default).
    pub fn should_rebuild(&self) -> bool {
        if self.tool_name.as_deref() != Some("Bash") {
            return true;
        }

        let cmd = match self.tool_input.command.as_deref() {
            Some(c) => c,
            None => return true,
        };

        // Check every segment of piped/chained commands.
        // If ALL are read-only, skip. If ANY is unknown, rebuild.
        !cmd.split(&['|', ';'][..])
            .map(|s| s.trim().trim_start_matches('('))
            // split on && (crude: split on & then skip empty segments)
            .flat_map(|s| s.split("&&").map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .all(|segment| {
                READONLY_COMMANDS.iter().any(|ro| segment.starts_with(ro))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(tool_name: &str, command: Option<&str>) -> HookInput {
        HookInput {
            tool_name: Some(tool_name.into()),
            tool_input: ToolInput {
                file_path: None,
                notebook_path: None,
                command: command.map(|s| s.into()),
            },
            cwd: None,
        }
    }

    #[test]
    fn test_edit_always_rebuilds() {
        assert!(hook("Edit", None).should_rebuild());
        assert!(hook("Write", None).should_rebuild());
    }

    #[test]
    fn test_bash_readonly_skips() {
        assert!(!hook("Bash", Some("ls -la")).should_rebuild());
        assert!(!hook("Bash", Some("cat foo.txt")).should_rebuild());
        assert!(!hook("Bash", Some("git status")).should_rebuild());
        assert!(!hook("Bash", Some("git log --oneline")).should_rebuild());
        assert!(!hook("Bash", Some("grep -r TODO src/")).should_rebuild());
        assert!(!hook("Bash", Some("cargo test")).should_rebuild());
        assert!(!hook("Bash", Some("npm test")).should_rebuild());
        assert!(!hook("Bash", Some("echo hello")).should_rebuild());
        assert!(!hook("Bash", Some("cargo build")).should_rebuild());
    }

    #[test]
    fn test_bash_known_mutating_rebuilds() {
        assert!(hook("Bash", Some("mv foo bar")).should_rebuild());
        assert!(hook("Bash", Some("cp -r src dest")).should_rebuild());
        assert!(hook("Bash", Some("rm -rf node_modules")).should_rebuild());
        assert!(hook("Bash", Some("sed -i 's/foo/bar/' file.txt")).should_rebuild());
        assert!(hook("Bash", Some("npm install lodash")).should_rebuild());
    }

    #[test]
    fn test_bash_unknown_commands_rebuild() {
        // Unknown commands default to rebuild (safe)
        assert!(hook("Bash", Some("protoc --go_out=. foo.proto")).should_rebuild());
        assert!(hook("Bash", Some("buf generate")).should_rebuild());
        assert!(hook("Bash", Some("sqlc generate")).should_rebuild());
        assert!(hook("Bash", Some("make")).should_rebuild());
        assert!(hook("Bash", Some("./scripts/codegen.sh")).should_rebuild());
    }

    #[test]
    fn test_bash_piped_readonly_skips() {
        assert!(!hook("Bash", Some("cat foo | grep bar")).should_rebuild());
        assert!(!hook("Bash", Some("git log | head -5")).should_rebuild());
    }

    #[test]
    fn test_bash_piped_with_unknown_rebuilds() {
        assert!(hook("Bash", Some("cat foo | ./process.sh")).should_rebuild());
        assert!(hook("Bash", Some("echo hi && mv a b")).should_rebuild());
    }

    #[test]
    fn test_bash_no_command_rebuilds() {
        assert!(hook("Bash", None).should_rebuild());
    }
}
