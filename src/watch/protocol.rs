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
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "ls",
    "dir",
    "find",
    "fd",
    "tree",
    "grep",
    "rg",
    "ag",
    "ack",
    "wc",
    "diff",
    "cmp",
    "file",
    "stat",
    "echo",
    "printf",
    "true",
    "false",
    "pwd",
    "which",
    "whereis",
    "whence",
    "type",
    "env",
    "printenv",
    "set",
    "ps",
    "top",
    "htop",
    "uptime",
    "df",
    "du",
    "free",
    "date",
    "cal",
    "man",
    "help",
    "info",
    "git status",
    "git log",
    "git diff",
    "git show",
    "git branch",
    "git remote",
    "git tag",
    "git stash list",
    "git rev-parse",
    "cargo test",
    "cargo check",
    "cargo clippy",
    "cargo bench",
    "cargo doc",
    "cargo build",
    "go test",
    "go vet",
    "go build",
    "npm test",
    "npm run test",
    "npm run lint",
    "npm run build",
    "npx",
    "yarn test",
    "pnpm test",
    "python -m pytest",
    "pytest",
    "make check",
    "make test",
    "jq",
    "yq",
    "xargs",
    "curl",
    "wget",
    "http",
    "docker ps",
    "docker images",
    "docker logs",
    "kubectl get",
    "kubectl describe",
    "kubectl logs",
    "gh pr view",
    "gh issue view",
    "gh api",
    "gh run view",
];

/// Commands that wrap or chain into another, arbitrary command, or are inline
/// interpreters whose string/script argument this filter can't scan. Several of these
/// (`find`, `xargs`, `env`, `npx`) are *often* used read-only, but just as easily wrap
/// something mutating (`find . -exec rm {} \;`, `echo f | xargs rm`, `env FOO=1 mv a
/// b`, `npx <codegen>`); the interpreter ones (`python -c "open('x','w')..."`, `node -e
/// "fs.writeFileSync(...)"`) can write files directly with no shell-visible redirection
/// at all. A segment starting with one of these is never treated as read-only
/// regardless of what it's wrapping — this overrides any match against
/// READONLY_COMMANDS.
const WRAPPER_COMMANDS: &[&str] = &[
    "env",
    "xargs",
    "find",
    "npx",
    "sudo",
    "nohup",
    "eval",
    "bash -c",
    "sh -c",
    "command",
    "builtin",
    "python -c",
    "python3 -c",
    "node -e",
    "ruby -e",
    "perl -e",
];

/// Whether a single command segment could have a side effect that a prefix match
/// against READONLY_COMMANDS can't see: shell output redirection (creates/overwrites a
/// file) or command substitution (runs an arbitrary embedded command, which could
/// itself mutate — `echo $(protoc --go_out=. x.proto)` looks read-only by prefix alone).
fn segment_has_hidden_side_effect(segment: &str) -> bool {
    // Covers `>` and `>>`. Deliberately not narrowed to exclude fd-duplication forms
    // like `2>&1` — this filter is meant to fail toward triggering a rebuild, so an
    // occasional harmless extra rebuild from stderr redirection is an acceptable
    // trade-off for never missing a real file write.
    if segment.contains('>') || segment.contains("$(") || segment.contains('`') {
        return true;
    }
    // curl/wget are read-only listed for the common "fetch and print/pipe" case, but
    // -o/-O (curl) and -O (wget) write the response to a file in the tree.
    if segment.starts_with("curl") || segment.starts_with("wget") {
        return segment
            .split_whitespace()
            .any(|tok| tok == "-O" || tok.starts_with("-o") || tok.starts_with("--output"));
    }
    false
}

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
    /// Unknown commands default to triggering a rebuild (safe default), and so does
    /// anything this filter can't fully account for (redirections, command
    /// substitution, wrapper commands) — see `segment_has_hidden_side_effect` and
    /// `WRAPPER_COMMANDS`.
    pub fn should_rebuild(&self) -> bool {
        if self.tool_name.as_deref() != Some("Bash") {
            return true;
        }

        let cmd = match self.tool_input.command.as_deref() {
            Some(c) => c,
            None => return true,
        };

        // Check every segment of piped/chained/sequenced/backgrounded commands.
        // If ALL are read-only, skip. If ANY is unknown (or looks risky), rebuild.
        // A single char class covers '|' and '||' together (splitting on '|' breaks
        // '||' into two segments plus a filtered-out empty one), and likewise '&' and
        // '&&'; '\n' and '\r' cover multi-line Bash strings (CRLF, LF, or a bare CR —
        // classic Mac line endings — all split the same way).
        !cmd.split(['\n', '\r', '|', '&', ';'])
            .map(|s| s.trim().trim_start_matches('('))
            .filter(|s| !s.is_empty())
            .all(|segment| {
                !segment_has_hidden_side_effect(segment)
                    && !WRAPPER_COMMANDS.iter().any(|w| segment.starts_with(w))
                    && READONLY_COMMANDS.iter().any(|ro| segment.starts_with(ro))
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

    #[test]
    fn test_bash_output_redirection_rebuilds() {
        // A redirection after a "read-only" leading command still writes a file.
        for cmd in [
            "echo generated > out.rs",
            "cat template > gen.rs",
            "printf 'x' >> a.rs",
            "cat a.rs 2>&1 > out.log",
        ] {
            assert!(
                hook("Bash", Some(cmd)).should_rebuild(),
                "expected rebuild for: {cmd}"
            );
        }
    }

    #[test]
    fn test_bash_wrapper_commands_rebuild() {
        for cmd in [
            r#"find . -name "*.tmp" -exec rm {} \;"#,
            "echo src/gen.rs | xargs rm",
            "env FOO=bar mv a.rs b.rs",
            "npx tsc --outDir gen",
            "sudo rm -rf build",
            "nohup ./codegen.sh &",
            r#"eval "$CMD""#,
            r#"bash -c "mv a.rs b.rs""#,
            r#"sh -c "mv a.rs b.rs""#,
        ] {
            assert!(
                hook("Bash", Some(cmd)).should_rebuild(),
                "expected rebuild for: {cmd}"
            );
        }
    }

    #[test]
    fn test_bash_newline_separated_commands_rebuild() {
        assert!(hook("Bash", Some("cat a.rs\nmv b.rs c.rs")).should_rebuild());
        // The mutating line first should also be caught (order shouldn't matter).
        assert!(hook("Bash", Some("mv b.rs c.rs\ncat a.rs")).should_rebuild());
    }

    #[test]
    fn test_bash_or_chain_with_mutating_segment_rebuilds() {
        assert!(hook("Bash", Some("false || rm -rf build")).should_rebuild());
    }

    #[test]
    fn test_bash_or_chain_all_readonly_skips() {
        assert!(!hook("Bash", Some("git status || true")).should_rebuild());
    }

    #[test]
    fn test_bash_command_substitution_rebuilds() {
        assert!(hook("Bash", Some("echo $(protoc --go_out=. x.proto)")).should_rebuild());
        assert!(hook("Bash", Some("echo `codegen`")).should_rebuild());
    }

    #[test]
    fn test_bash_wrapper_commands_still_flagged_when_piped() {
        assert!(hook("Bash", Some("git log | xargs -I{} echo {}")).should_rebuild());
    }

    #[test]
    fn test_bash_inline_interpreters_rebuild() {
        for cmd in [
            r#"python -c "open('x.rs','w').write('x')""#,
            r#"python3 -c "open('x.rs','w').write('x')""#,
            r#"node -e "require('fs').writeFileSync('x.rs','x')""#,
            r#"ruby -e "File.write('x.rs','x')""#,
            r#"perl -e "open(F,'>x.rs')""#,
        ] {
            assert!(
                hook("Bash", Some(cmd)).should_rebuild(),
                "expected rebuild for: {cmd}"
            );
        }
        // python -m pytest / pytest themselves remain read-only.
        assert!(!hook("Bash", Some("python -m pytest")).should_rebuild());
        assert!(!hook("Bash", Some("pytest")).should_rebuild());
    }

    #[test]
    fn test_bash_downloader_output_flags_rebuild() {
        for cmd in [
            "curl -o src/x.ts https://example.com/x.ts",
            "curl -O https://example.com/x.ts",
            "curl https://example.com/x.ts -o src/x.ts",
            "wget -O x.ts https://example.com/x.ts",
        ] {
            assert!(
                hook("Bash", Some(cmd)).should_rebuild(),
                "expected rebuild for: {cmd}"
            );
        }
        // Plain fetch-and-print/pipe usage stays read-only.
        assert!(!hook("Bash", Some("curl https://example.com/status")).should_rebuild());
        assert!(!hook("Bash", Some("wget -q -O- https://example.com | grep ok")).should_rebuild());
    }

    #[test]
    fn test_bash_command_and_builtin_wrappers_rebuild() {
        assert!(hook("Bash", Some("command rm x.rs")).should_rebuild());
        assert!(hook("Bash", Some("builtin rm x.rs")).should_rebuild());
    }

    #[test]
    fn test_bash_single_ampersand_background_chain_rebuilds() {
        assert!(hook("Bash", Some("echo hi & mv a.rs b.rs")).should_rebuild());
    }

    #[test]
    fn test_bash_carriage_return_separated_commands_rebuild() {
        assert!(hook("Bash", Some("cat a.rs\rmv b.rs c.rs")).should_rebuild());
    }

    #[test]
    fn test_bash_crlf_separated_readonly_still_skips() {
        // A CRLF-joined pair of genuinely read-only commands should still skip.
        assert!(!hook("Bash", Some("git status\r\ncat a.rs")).should_rebuild());
    }
}
