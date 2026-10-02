//! The optional Claude Code status mod (`contrib/claude-mod/shire-status`).
//!
//! The mod's files are compiled into the binary, so the mod on disk always
//! matches the `shire status --json` it reads. Installing writes them to
//! `~/.claude/shire-mod/shire-status/` and adds that folder to
//! `env.CLAUDE_CODE_PLUGIN_DIRS` in `~/.claude/settings.json`: Claude Code
//! reads that variable from the user's settings only, never a project's, so
//! the mod is always a per-user opt-in (`shire init --mod`), never something a
//! cloned repo's config can switch on.

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

use crate::init::{target_mode, write_with_mode};

/// The plugin's name, as its `plugin.json` declares it.
pub const MOD_NAME: &str = "shire-status";

/// The settings variable Claude Code loads extra plugin folders from.
pub const PLUGIN_DIRS_VAR: &str = "CLAUDE_CODE_PLUGIN_DIRS";

macro_rules! mod_file {
    ($rel:literal) => {
        (
            $rel,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/contrib/claude-mod/shire-status/",
                $rel
            )),
        )
    };
}

/// Every file the mod needs at run time, by its path inside the mod folder.
/// Its tests and tsconfig stay in the repo; Claude Code lays the API types
/// into `.claude-plugin/types/` by itself.
const FILES: &[(&str, &str)] = &[
    mod_file!(".claude-plugin/plugin.json"),
    mod_file!("hooks/hooks.json"),
    mod_file!("hooks/register.tsx"),
    mod_file!("hooks/format.ts"),
    mod_file!("types/index.d.ts"),
    mod_file!("README.md"),
];

/// Where the mod is installed under `claude_dir` (`~/.claude`).
///
/// Deliberately not under `~/.claude/shire/`, where the global `db_path`
/// default puts `{repo}/{worktree}/index.db`: a repository named `mod`
/// would share the directory.
pub fn mod_dir(claude_dir: &Path) -> PathBuf {
    claude_dir.join("shire-mod").join(MOD_NAME)
}

/// What [`install`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// Files written and the folder newly listed in the settings.
    Added,
    /// Files rewritten; the settings already listed the folder.
    Refreshed,
}

/// Write the mod's files and register its folder in `claude_dir/settings.json`.
/// Re-running it after an upgrade refreshes the files in place.
pub fn install(claude_dir: &Path) -> Result<Installed> {
    let dir = mod_dir(claude_dir);
    refuse_symlink(&dir)?;
    for (rel, content) in FILES {
        let path = dir.join(rel);
        let parent = path.parent().expect("mod files live in a folder");
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
        let tmp = path.with_extension("shire-tmp");
        write_with_mode(&tmp, content, 0o644)?;
        fs::rename(&tmp, &path).with_context(|| format!("Failed to write {}", path.display()))?;
    }
    let added = edit_settings(&claude_dir.join("settings.json"), |dirs| {
        if dirs.contains(&dir) {
            false
        } else {
            dirs.push(dir.clone());
            true
        }
    })?;
    Ok(if added {
        Installed::Added
    } else {
        Installed::Refreshed
    })
}

/// Rewrite the mod's files if it is installed, so `shire install` keeps an
/// installed mod in step with an upgraded binary. Never installs it.
pub fn refresh_if_installed(claude_dir: &Path) -> Result<bool> {
    if !is_installed(claude_dir) {
        return Ok(false);
    }
    install(claude_dir)?;
    Ok(true)
}

/// The mod's files are on disk (whether or not the settings list them).
pub fn is_installed(claude_dir: &Path) -> bool {
    let manifest = mod_dir(claude_dir).join(".claude-plugin/plugin.json");
    fs::symlink_metadata(&manifest).is_ok_and(|m| m.is_file())
}

/// Unregister the folder and delete it. `Ok(false)` when there was nothing
/// to remove.
pub fn uninstall(claude_dir: &Path, dry_run: bool) -> Result<bool> {
    let dir = mod_dir(claude_dir);
    let settings = claude_dir.join("settings.json");
    let listed = read_settings(&settings)?
        .as_ref()
        .is_some_and(|s| plugin_dirs(s).contains(&dir));
    let on_disk = fs::symlink_metadata(&dir).is_ok();
    if !listed && !on_disk {
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    if listed {
        edit_settings(&settings, |dirs| {
            let before = dirs.len();
            dirs.retain(|d| d != &dir);
            dirs.len() != before
        })?;
    }
    if on_disk {
        refuse_symlink(&dir)?;
        fs::remove_dir_all(&dir).with_context(|| format!("Failed to remove {}", dir.display()))?;
        // The parent holds nothing but mods shire installed.
        let _ = fs::remove_dir(dir.parent().expect("mod_dir has a parent"));
    }
    Ok(true)
}

/// `remove_dir_all` on a symlink deletes nothing but the link on unix, but
/// writing *through* one would put the mod's files wherever it points.
fn refuse_symlink(dir: &Path) -> Result<()> {
    for p in [dir, dir.parent().unwrap_or(dir)] {
        if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
            anyhow::bail!(
                "{} is a symlink; refusing to install the Claude Code mod through it",
                p.display()
            );
        }
    }
    Ok(())
}

fn read_settings(path: &Path) -> Result<Option<Map<String, Value>>> {
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    match value {
        Value::Object(map) => Ok(Some(map)),
        _ => anyhow::bail!("{} is not a JSON object", path.display()),
    }
}

/// The folders `env.CLAUDE_CODE_PLUGIN_DIRS` lists, split on the platform's
/// path-list separator the way Claude Code splits it.
fn plugin_dirs(settings: &Map<String, Value>) -> Vec<PathBuf> {
    settings
        .get("env")
        .and_then(|e| e.get(PLUGIN_DIRS_VAR))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| std::env::split_paths(s).collect())
        .unwrap_or_default()
}

/// Apply `change` to the listed plugin folders and write the settings back
/// atomically when it reports a change. Every other key is kept as it was.
fn edit_settings(path: &Path, change: impl FnOnce(&mut Vec<PathBuf>) -> bool) -> Result<bool> {
    let mut settings = read_settings(path)?.unwrap_or_default();
    let mut dirs = plugin_dirs(&settings);
    if !change(&mut dirs) {
        return Ok(false);
    }

    let env = settings.entry("env").or_insert_with(|| json!({}));
    let Some(env) = env.as_object_mut() else {
        anyhow::bail!(
            "{} has 'env' as a non-object; fix it by hand and re-run",
            path.display()
        );
    };
    if dirs.is_empty() {
        env.remove(PLUGIN_DIRS_VAR);
        if env.is_empty() {
            settings.remove("env");
        }
    } else {
        let joined = std::env::join_paths(&dirs)
            .context("a plugin folder path contains the path-list separator")?;
        let joined = joined
            .into_string()
            .map_err(|_| anyhow::anyhow!("a plugin folder path is not valid UTF-8"))?;
        env.insert(PLUGIN_DIRS_VAR.into(), Value::String(joined));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let output = serde_json::to_string_pretty(&Value::Object(settings))
        .context("Failed to serialize settings")?;
    let tmp = path.with_extension("json.tmp");
    write_with_mode(&tmp, &format!("{output}\n"), target_mode(path, 0o600))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("Failed to write {}", path.display()));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(claude: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(claude.join("settings.json")).unwrap()).unwrap()
    }

    #[test]
    fn embedded_manifest_names_the_mod() {
        let (_, manifest) = FILES[0];
        let v: Value = serde_json::from_str(manifest).unwrap();
        assert_eq!(v["name"], MOD_NAME);
        assert!(FILES.iter().any(|(rel, _)| *rel == "hooks/register.tsx"));
    }

    #[test]
    fn install_writes_the_files_and_keeps_other_settings() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude = tmp.path();
        fs::write(
            claude.join("settings.json"),
            r#"{"hooks": {"PostToolUse": []}, "env": {"FOO": "1", "CLAUDE_CODE_PLUGIN_DIRS": "/other/mod"}}"#,
        )
        .unwrap();

        assert_eq!(install(claude).unwrap(), Installed::Added);

        let dir = mod_dir(claude);
        for (rel, content) in FILES {
            assert_eq!(&fs::read_to_string(dir.join(rel)).unwrap(), content);
        }
        let s = settings(claude);
        assert_eq!(s["env"]["FOO"], "1");
        assert!(s["hooks"]["PostToolUse"].is_array());
        let listed: Vec<PathBuf> =
            std::env::split_paths(s["env"][PLUGIN_DIRS_VAR].as_str().unwrap()).collect();
        assert_eq!(listed, vec![PathBuf::from("/other/mod"), dir]);
    }

    #[test]
    fn install_twice_refreshes_without_listing_twice() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude = tmp.path();
        install(claude).unwrap();
        fs::write(mod_dir(claude).join("hooks/register.tsx"), "stale").unwrap();

        assert_eq!(install(claude).unwrap(), Installed::Refreshed);
        assert_ne!(
            fs::read_to_string(mod_dir(claude).join("hooks/register.tsx")).unwrap(),
            "stale"
        );
        let s = settings(claude);
        let var = s["env"][PLUGIN_DIRS_VAR].as_str().unwrap();
        assert_eq!(std::env::split_paths(var).count(), 1);
    }

    #[test]
    fn refresh_only_touches_an_installed_mod() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude = tmp.path();
        assert!(!refresh_if_installed(claude).unwrap());
        assert!(!claude.join("settings.json").exists());
        install(claude).unwrap();
        assert!(refresh_if_installed(claude).unwrap());
    }

    #[test]
    fn uninstall_removes_only_what_install_added() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude = tmp.path();
        fs::write(
            claude.join("settings.json"),
            r#"{"env": {"CLAUDE_CODE_PLUGIN_DIRS": "/other/mod"}}"#,
        )
        .unwrap();
        install(claude).unwrap();

        assert!(uninstall(claude, true).unwrap(), "dry run reports it");
        assert!(mod_dir(claude).exists(), "dry run changes nothing");

        assert!(uninstall(claude, false).unwrap());
        assert!(!mod_dir(claude).exists());
        assert!(!claude.join("shire-mod").exists());
        assert_eq!(settings(claude)["env"][PLUGIN_DIRS_VAR], "/other/mod");

        assert!(!uninstall(claude, false).unwrap(), "nothing left to remove");
    }

    #[test]
    fn uninstall_drops_an_env_block_it_emptied() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude = tmp.path();
        install(claude).unwrap();
        uninstall(claude, false).unwrap();
        assert!(settings(claude).get("env").is_none());
    }

    #[test]
    fn a_non_object_env_is_refused_not_clobbered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude = tmp.path();
        fs::write(claude.join("settings.json"), r#"{"env": "nope"}"#).unwrap();
        assert!(install(claude).is_err());
        assert_eq!(settings(claude)["env"], "nope");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_mod_folder_is_refused() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude = tmp.path().join("claude");
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir_all(claude.join("shire-mod")).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, mod_dir(&claude)).unwrap();
        assert!(install(&claude).is_err());
        assert!(fs::read_dir(&elsewhere).unwrap().next().is_none());
    }
}
