# Shire Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust CLI + MCP server that indexes monorepo package graphs in SQLite so Claude Code can query codebase structure without burning context on exploration.

**Architecture:** Single Rust binary with `build` and `serve` subcommands. `build` walks the repo for manifest files (package.json, go.mod, Cargo.toml, pyproject.toml), parses them, and populates a SQLite database with package metadata and dependency edges. `serve` opens the database read-only and exposes it via MCP tools over stdio transport using the `rmcp` crate.

**Tech Stack:** Rust, rusqlite (bundled + FTS5), rmcp 0.3 (MCP server, stdio), clap (CLI), serde/serde_json/toml (parsing), walkdir + ignore (directory traversal), schemars 1.0 (JSON schema for MCP tool params), tokio (async runtime for MCP)

---

### Task 1: Project Scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`

**Step 1: Initialize the cargo project**

Run: `cargo init --name shire /Users/justin/git/new-idea`

**Step 2: Write Cargo.toml with all dependencies**

Replace `Cargo.toml` contents:

```toml
[package]
name = "shire"
version = "0.1.0"
edition = "2024"
description = "Search, Hierarchy, Index, Repo Explorer - monorepo package index and MCP server"

[dependencies]
clap = { version = "4", features = ["derive"] }
rusqlite = { version = "0.36", features = ["bundled"] }
rmcp = { version = "0.3", features = ["server", "transport-io"] }
schemars = "1.0"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
walkdir = "2"
ignore = "0.4"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
chrono = "0.4"
```

**Step 3: Write minimal main.rs with clap subcommands**

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "shire", about = "Monorepo package index and MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan the repository and build the package index
    Build {
        /// Root directory of the repository (defaults to current directory)
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Start the MCP server over stdio
    Serve {
        /// Path to the index database (defaults to .shire/index.db in repo root)
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Build { root } => {
            println!("Building index for: {}", root.display());
            Ok(())
        }
        Commands::Serve { db } => {
            println!("Serving from: {:?}", db);
            Ok(())
        }
    }
}
```

**Step 4: Verify it compiles and runs**

Run: `cargo build`
Expected: Compiles with no errors (warnings OK for now)

Run: `cargo run -- build`
Expected: `Building index for: .`

Run: `cargo run -- serve`
Expected: `Serving from: None`

**Step 5: Commit**

```bash
git add Cargo.toml src/main.rs
git commit -m "feat: project scaffold with clap CLI"
```

---

### Task 2: Database Schema

**Files:**
- Create: `src/db/mod.rs`
- Create: `src/db/queries.rs`
- Modify: `src/main.rs` (add `mod db;`)

**Step 1: Write a test for schema creation**

In `src/db/mod.rs`:

```rust
pub mod queries;

use anyhow::Result;
use rusqlite::Connection;

pub fn open_or_create(path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    create_schema(&conn)?;
    Ok(conn)
}

pub fn open_readonly(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(conn)
}

fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS packages (
            name        TEXT PRIMARY KEY,
            path        TEXT NOT NULL UNIQUE,
            kind        TEXT NOT NULL,
            version     TEXT,
            description TEXT,
            metadata    TEXT
        );

        CREATE TABLE IF NOT EXISTS dependencies (
            package     TEXT NOT NULL REFERENCES packages(name),
            dependency  TEXT NOT NULL,
            dep_kind    TEXT NOT NULL DEFAULT 'runtime',
            version_req TEXT,
            is_internal INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (package, dependency, dep_kind)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS packages_fts USING fts5(
            name, description, path,
            content='packages',
            content_rowid='rowid'
        );

        -- Triggers to keep FTS in sync
        CREATE TRIGGER IF NOT EXISTS packages_ai AFTER INSERT ON packages BEGIN
            INSERT INTO packages_fts(rowid, name, description, path)
            VALUES (new.rowid, new.name, new.description, new.path);
        END;

        CREATE TRIGGER IF NOT EXISTS packages_ad AFTER DELETE ON packages BEGIN
            INSERT INTO packages_fts(packages_fts, rowid, name, description, path)
            VALUES ('delete', old.rowid, old.name, old.description, old.path);
        END;

        CREATE TRIGGER IF NOT EXISTS packages_au AFTER UPDATE ON packages BEGIN
            INSERT INTO packages_fts(packages_fts, rowid, name, description, path)
            VALUES ('delete', old.rowid, old.name, old.description, old.path);
            INSERT INTO packages_fts(rowid, name, description, path)
            VALUES (new.rowid, new.name, new.description, new.path);
        END;

        CREATE TABLE IF NOT EXISTS shire_meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );
        ",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_schema_creates_tables() {
        let conn = in_memory_db();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(tables.contains(&"packages".to_string()));
        assert!(tables.contains(&"dependencies".to_string()));
        assert!(tables.contains(&"shire_meta".to_string()));
    }

    #[test]
    fn test_insert_and_fts_search() {
        let conn = in_memory_db();
        conn.execute(
            "INSERT INTO packages (name, path, kind, description) VALUES (?1, ?2, ?3, ?4)",
            ("auth-service", "services/auth", "npm", "Authentication and authorization"),
        )
        .unwrap();

        let results: Vec<String> = conn
            .prepare("SELECT name FROM packages_fts WHERE packages_fts MATCH ?1")
            .unwrap()
            .query_map(["auth"], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(results, vec!["auth-service"]);
    }

    #[test]
    fn test_schema_is_idempotent() {
        let conn = in_memory_db();
        // Running create_schema again should not error
        create_schema(&conn).unwrap();
    }
}
```

**Step 2: Create empty queries module**

In `src/db/queries.rs`:

```rust
// Query functions will be added as MCP tools are implemented.
```

**Step 3: Add module to main.rs**

Add `mod db;` near the top of `src/main.rs`:

```rust
mod db;
```

**Step 4: Run tests**

Run: `cargo test db::tests`
Expected: 3 tests pass

**Step 5: Commit**

```bash
git add src/db/ src/main.rs
git commit -m "feat: database schema with packages, dependencies, FTS5"
```

---

### Task 3: Manifest Parser Trait + npm Parser

**Files:**
- Create: `src/index/mod.rs`
- Create: `src/index/manifest.rs`
- Create: `src/index/npm.rs`
- Modify: `src/main.rs` (add `mod index;`)

**Step 1: Define the ManifestParser trait and data types**

In `src/index/manifest.rs`:

```rust
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub version: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub dependencies: Vec<DepInfo>,
}

#[derive(Debug, Clone)]
pub struct DepInfo {
    pub name: String,
    pub version_req: Option<String>,
    pub dep_kind: DepKind,
}

#[derive(Debug, Clone, Copy)]
pub enum DepKind {
    Runtime,
    Dev,
    Peer,
    Build,
}

impl DepKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DepKind::Runtime => "runtime",
            DepKind::Dev => "dev",
            DepKind::Peer => "peer",
            DepKind::Build => "build",
        }
    }
}

pub trait ManifestParser {
    /// The filename this parser handles (e.g. "package.json")
    fn filename(&self) -> &'static str;

    /// Parse a manifest file and return package info.
    /// `manifest_path` is the full path to the manifest file.
    /// `relative_dir` is the directory containing the manifest, relative to repo root.
    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo>;
}
```

**Step 2: Write tests for the npm parser**

In `src/index/npm.rs`:

```rust
use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::path::Path;

pub struct NpmParser;

impl ManifestParser for NpmParser {
    fn filename(&self) -> &'static str {
        "package.json"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let json: serde_json::Value = serde_json::from_str(&content)?;

        let name = json["name"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let version = json["version"].as_str().map(|s| s.to_string());
        let description = json["description"].as_str().map(|s| s.to_string());

        let mut dependencies = Vec::new();

        if let Some(deps) = json["dependencies"].as_object() {
            for (name, ver) in deps {
                dependencies.push(DepInfo {
                    name: name.clone(),
                    version_req: ver.as_str().map(|s| s.to_string()),
                    dep_kind: DepKind::Runtime,
                });
            }
        }

        if let Some(deps) = json["devDependencies"].as_object() {
            for (name, ver) in deps {
                dependencies.push(DepInfo {
                    name: name.clone(),
                    version_req: ver.as_str().map(|s| s.to_string()),
                    dep_kind: DepKind::Dev,
                });
            }
        }

        if let Some(deps) = json["peerDependencies"].as_object() {
            for (name, ver) in deps {
                dependencies.push(DepInfo {
                    name: name.clone(),
                    version_req: ver.as_str().map(|s| s.to_string()),
                    dep_kind: DepKind::Peer,
                });
            }
        }

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "npm",
            version,
            description,
            metadata: None,
            dependencies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("package.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_basic_package_json() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
                "name": "@scope/auth-service",
                "version": "1.2.3",
                "description": "Auth service",
                "dependencies": {
                    "express": "^4.18.0",
                    "jsonwebtoken": "^9.0.0"
                },
                "devDependencies": {
                    "jest": "^29.0.0"
                }
            }"#,
        );

        let parser = NpmParser;
        let info = parser.parse(&path, "services/auth").unwrap();

        assert_eq!(info.name, "@scope/auth-service");
        assert_eq!(info.version.as_deref(), Some("1.2.3"));
        assert_eq!(info.description.as_deref(), Some("Auth service"));
        assert_eq!(info.kind, "npm");
        assert_eq!(info.path, "services/auth");
        assert_eq!(info.dependencies.len(), 3);

        let runtime_deps: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Runtime))
            .map(|d| d.name.as_str())
            .collect();
        assert!(runtime_deps.contains(&"express"));
        assert!(runtime_deps.contains(&"jsonwebtoken"));

        let dev_deps: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Dev))
            .map(|d| d.name.as_str())
            .collect();
        assert!(dev_deps.contains(&"jest"));
    }

    #[test]
    fn test_parse_minimal_package_json() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(dir.path(), r#"{"name": "minimal"}"#);

        let parser = NpmParser;
        let info = parser.parse(&path, "packages/minimal").unwrap();

        assert_eq!(info.name, "minimal");
        assert_eq!(info.version, None);
        assert_eq!(info.description, None);
        assert!(info.dependencies.is_empty());
    }

    #[test]
    fn test_parse_no_name_falls_back_to_dir() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(dir.path(), r#"{"version": "1.0.0"}"#);

        let parser = NpmParser;
        let info = parser.parse(&path, "packages/unnamed").unwrap();

        assert_eq!(info.name, "packages-unnamed");
    }
}
```

**Step 3: Create the index module**

In `src/index/mod.rs`:

```rust
pub mod manifest;
pub mod npm;
```

Add to `src/main.rs`:

```rust
mod index;
```

**Step 4: Add tempfile dev-dependency**

Add to `Cargo.toml` under `[dependencies]`:

```toml
[dev-dependencies]
tempfile = "3"
```

**Step 5: Run tests**

Run: `cargo test index::npm::tests`
Expected: 3 tests pass

**Step 6: Commit**

```bash
git add src/index/ src/main.rs Cargo.toml
git commit -m "feat: manifest parser trait and npm parser"
```

---

### Task 4: Go, Cargo, and Python Parsers

**Files:**
- Create: `src/index/go.rs`
- Create: `src/index/cargo.rs`
- Create: `src/index/python.rs`
- Modify: `src/index/mod.rs` (add modules)

**Step 1: Write the Go parser with tests**

In `src/index/go.rs`:

```rust
use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::path::Path;

pub struct GoParser;

impl ManifestParser for GoParser {
    fn filename(&self) -> &'static str {
        "go.mod"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let mut module_name = String::new();
        let mut go_version = None;
        let mut dependencies = Vec::new();
        let mut in_require = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if let Some(rest) = trimmed.strip_prefix("module ") {
                module_name = rest.trim().to_string();
            } else if let Some(rest) = trimmed.strip_prefix("go ") {
                go_version = Some(rest.trim().to_string());
            } else if trimmed == "require (" {
                in_require = true;
            } else if trimmed == ")" {
                in_require = false;
            } else if in_require && !trimmed.is_empty() && !trimmed.starts_with("//") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    dependencies.push(DepInfo {
                        name: parts[0].to_string(),
                        version_req: Some(parts[1].to_string()),
                        dep_kind: DepKind::Runtime,
                    });
                }
            } else if let Some(rest) = trimmed.strip_prefix("require ") {
                // Single-line require
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 2 {
                    dependencies.push(DepInfo {
                        name: parts[0].to_string(),
                        version_req: Some(parts[1].to_string()),
                        dep_kind: DepKind::Runtime,
                    });
                }
            }
        }

        if module_name.is_empty() {
            module_name = relative_dir.replace('/', "-");
        }

        // Use the last segment of the module path as the short name
        let short_name = module_name
            .rsplit('/')
            .next()
            .unwrap_or(&module_name)
            .to_string();

        Ok(PackageInfo {
            name: short_name,
            path: relative_dir.to_string(),
            kind: "go",
            version: go_version,
            description: Some(module_name),
            metadata: None,
            dependencies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_parse_go_mod() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("go.mod");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(
            b"module github.com/company/api-gateway\n\
              \n\
              go 1.22\n\
              \n\
              require (\n\
              \tgithub.com/gin-gonic/gin v1.9.1\n\
              \tgithub.com/lib/pq v1.10.9\n\
              )\n",
        )
        .unwrap();

        let parser = GoParser;
        let info = parser.parse(&path, "services/gateway").unwrap();

        assert_eq!(info.name, "api-gateway");
        assert_eq!(info.kind, "go");
        assert_eq!(info.version.as_deref(), Some("1.22"));
        assert_eq!(info.dependencies.len(), 2);
        assert_eq!(info.dependencies[0].name, "github.com/gin-gonic/gin");
    }
}
```

**Step 2: Write the Cargo parser with tests**

In `src/index/cargo.rs`:

```rust
use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::path::Path;

pub struct CargoParser;

impl ManifestParser for CargoParser {
    fn filename(&self) -> &'static str {
        "Cargo.toml"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let parsed: toml::Value = content.parse()?;

        let package = parsed.get("package");

        let name = package
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let version = package
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let description = package
            .and_then(|p| p.get("description"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut dependencies = Vec::new();

        fn extract_deps(table: &toml::Value, kind: DepKind) -> Vec<DepInfo> {
            let mut deps = Vec::new();
            if let Some(obj) = table.as_table() {
                for (name, val) in obj {
                    let version_req = match val {
                        toml::Value::String(s) => Some(s.clone()),
                        toml::Value::Table(t) => {
                            t.get("version").and_then(|v| v.as_str()).map(|s| s.to_string())
                        }
                        _ => None,
                    };
                    deps.push(DepInfo {
                        name: name.clone(),
                        version_req,
                        dep_kind: kind,
                    });
                }
            }
            deps
        }

        if let Some(deps) = parsed.get("dependencies") {
            dependencies.extend(extract_deps(deps, DepKind::Runtime));
        }
        if let Some(deps) = parsed.get("dev-dependencies") {
            dependencies.extend(extract_deps(deps, DepKind::Dev));
        }
        if let Some(deps) = parsed.get("build-dependencies") {
            dependencies.extend(extract_deps(deps, DepKind::Build));
        }

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "cargo",
            version,
            description,
            metadata: None,
            dependencies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_parse_cargo_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Cargo.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(
            br#"[package]
name = "my-service"
version = "0.1.0"
description = "A service"

[dependencies]
serde = { version = "1", features = ["derive"] }
tokio = "1"

[dev-dependencies]
tempfile = "3"
"#,
        )
        .unwrap();

        let parser = CargoParser;
        let info = parser.parse(&path, "services/my-service").unwrap();

        assert_eq!(info.name, "my-service");
        assert_eq!(info.kind, "cargo");
        assert_eq!(info.version.as_deref(), Some("0.1.0"));
        assert_eq!(info.description.as_deref(), Some("A service"));

        let runtime: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Runtime))
            .map(|d| d.name.as_str())
            .collect();
        assert!(runtime.contains(&"serde"));
        assert!(runtime.contains(&"tokio"));

        let dev: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Dev))
            .map(|d| d.name.as_str())
            .collect();
        assert!(dev.contains(&"tempfile"));
    }
}
```

**Step 3: Write the Python parser with tests**

In `src/index/python.rs`:

```rust
use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::path::Path;

pub struct PythonParser;

impl ManifestParser for PythonParser {
    fn filename(&self) -> &'static str {
        "pyproject.toml"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let parsed: toml::Value = content.parse()?;

        let project = parsed.get("project");

        let name = project
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let version = project
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let description = project
            .and_then(|p| p.get("description"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut dependencies = Vec::new();

        // PEP 621 dependencies array
        if let Some(deps) = project.and_then(|p| p.get("dependencies")).and_then(|v| v.as_array())
        {
            for dep in deps {
                if let Some(dep_str) = dep.as_str() {
                    // Parse "package>=1.0" style strings
                    let (name, ver) = parse_pep508(dep_str);
                    dependencies.push(DepInfo {
                        name,
                        version_req: ver,
                        dep_kind: DepKind::Runtime,
                    });
                }
            }
        }

        // Optional dependencies (extras) as dev deps
        if let Some(optional) = project
            .and_then(|p| p.get("optional-dependencies"))
            .and_then(|v| v.as_table())
        {
            for (_group, deps) in optional {
                if let Some(arr) = deps.as_array() {
                    for dep in arr {
                        if let Some(dep_str) = dep.as_str() {
                            let (name, ver) = parse_pep508(dep_str);
                            dependencies.push(DepInfo {
                                name,
                                version_req: ver,
                                dep_kind: DepKind::Dev,
                            });
                        }
                    }
                }
            }
        }

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "python",
            version,
            description,
            metadata: None,
            dependencies,
        })
    }
}

/// Parse a PEP 508 dependency string like "requests>=2.28.0" into (name, version_req)
fn parse_pep508(s: &str) -> (String, Option<String>) {
    let s = s.trim();
    // Split on first version specifier character
    if let Some(pos) = s.find(|c: char| matches!(c, '>' | '<' | '=' | '!' | '~' | '[')) {
        let name = s[..pos].trim().to_string();
        let rest = s[pos..].trim();
        // Strip extras like [security]
        if rest.starts_with('[') {
            if let Some(end) = rest.find(']') {
                let ver = rest[end + 1..].trim();
                if ver.is_empty() {
                    return (name, None);
                }
                return (name, Some(ver.to_string()));
            }
        }
        (name, Some(rest.to_string()))
    } else {
        (s.to_string(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_parse_pyproject_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pyproject.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(
            br#"[project]
name = "ml-pipeline"
version = "0.3.0"
description = "ML training pipeline"
dependencies = [
    "torch>=2.0",
    "numpy",
    "pandas>=1.5.0",
]

[project.optional-dependencies]
dev = ["pytest>=7.0", "ruff"]
"#,
        )
        .unwrap();

        let parser = PythonParser;
        let info = parser.parse(&path, "services/ml").unwrap();

        assert_eq!(info.name, "ml-pipeline");
        assert_eq!(info.kind, "python");
        assert_eq!(info.version.as_deref(), Some("0.3.0"));

        let runtime: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Runtime))
            .map(|d| d.name.as_str())
            .collect();
        assert_eq!(runtime.len(), 3);
        assert!(runtime.contains(&"torch"));
        assert!(runtime.contains(&"numpy"));
    }

    #[test]
    fn test_pep508_parsing() {
        assert_eq!(parse_pep508("requests>=2.28"), ("requests".into(), Some(">=2.28".into())));
        assert_eq!(parse_pep508("numpy"), ("numpy".into(), None));
        assert_eq!(parse_pep508("torch>=2.0"), ("torch".into(), Some(">=2.0".into())));
    }
}
```

**Step 4: Update index/mod.rs**

```rust
pub mod manifest;
pub mod npm;
pub mod go;
pub mod cargo;
pub mod python;
```

**Step 5: Run all parser tests**

Run: `cargo test index`
Expected: All 7 parser tests pass (3 npm + 1 go + 1 cargo + 2 python)

**Step 6: Commit**

```bash
git add src/index/
git commit -m "feat: go, cargo, and python manifest parsers"
```

---

### Task 5: Config Module

**Files:**
- Create: `src/config.rs`
- Create: `shire.toml.example`
- Modify: `src/main.rs` (add `mod config;`)

**Step 1: Write the config parser with tests**

In `src/config.rs`:

```rust
use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub packages: Vec<PackageOverride>,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default = "default_manifests")]
    pub manifests: Vec<String>,
    #[serde(default = "default_exclude")]
    pub exclude: Vec<String>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            manifests: default_manifests(),
            exclude: default_exclude(),
        }
    }
}

fn default_manifests() -> Vec<String> {
    vec![
        "package.json".into(),
        "go.mod".into(),
        "Cargo.toml".into(),
        "pyproject.toml".into(),
    ]
}

fn default_exclude() -> Vec<String> {
    vec![
        "node_modules".into(),
        "vendor".into(),
        "dist".into(),
        ".build".into(),
        "target".into(),
        "third_party".into(),
    ]
}

#[derive(Debug, Deserialize)]
pub struct PackageOverride {
    pub name: String,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}

pub fn load_config(repo_root: &Path) -> Result<Config> {
    let config_path = repo_root.join("shire.toml");
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.discovery.manifests.len(), 4);
        assert!(config.discovery.exclude.contains(&"node_modules".to_string()));
        assert!(config.packages.is_empty());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[discovery]
manifests = ["package.json", "go.mod"]
exclude = ["vendor", "dist"]

[[packages]]
name = "legacy-auth"
description = "Deprecated auth service"
tags = ["deprecated"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.discovery.manifests.len(), 2);
        assert_eq!(config.packages.len(), 1);
        assert_eq!(config.packages[0].name, "legacy-auth");
    }

    #[test]
    fn test_load_missing_config_returns_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = load_config(dir.path()).unwrap();
        assert_eq!(config.discovery.manifests.len(), 4);
    }
}
```

**Step 2: Create shire.toml.example**

```toml
# Shire configuration (optional - defaults work for most repos)

[discovery]
# Which manifest files to look for
manifests = ["package.json", "go.mod", "Cargo.toml", "pyproject.toml"]

# Directories to skip (in addition to .gitignore patterns)
exclude = ["node_modules", "vendor", "dist", ".build", "target", "third_party"]

# Override package metadata not in manifests
# [[packages]]
# name = "legacy-auth"
# description = "Legacy auth service - deprecated, use auth-service instead"
# tags = ["deprecated"]
```

**Step 3: Add module to main.rs**

Add `mod config;` to `src/main.rs`.

**Step 4: Run tests**

Run: `cargo test config::tests`
Expected: 3 tests pass

**Step 5: Commit**

```bash
git add src/config.rs shire.toml.example src/main.rs
git commit -m "feat: shire.toml config parsing with defaults"
```

---

### Task 6: Index Orchestrator

**Files:**
- Modify: `src/index/mod.rs` (add orchestrator logic)
- Modify: `src/main.rs` (wire up `build` command)

**Step 1: Write the orchestrator that walks the repo and populates the database**

Replace `src/index/mod.rs`:

```rust
pub mod cargo;
pub mod go;
pub mod manifest;
pub mod npm;
pub mod python;

use crate::config::Config;
use crate::db;
use anyhow::Result;
use ignore::WalkBuilder;
use manifest::ManifestParser;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

pub fn build_index(repo_root: &Path, config: &Config) -> Result<()> {
    let db_dir = repo_root.join(".shire");
    let db_path = db_dir.join("index.db");
    let conn = db::open_or_create(&db_path)?;

    // Clear existing data for a clean rebuild
    conn.execute_batch(
        "DELETE FROM dependencies; DELETE FROM packages; DELETE FROM packages_fts;",
    )?;

    let parsers: Vec<Box<dyn ManifestParser>> = vec![
        Box::new(npm::NpmParser),
        Box::new(go::GoParser),
        Box::new(cargo::CargoParser),
        Box::new(python::PythonParser),
    ];

    let manifest_filenames: HashSet<&str> = parsers.iter().map(|p| p.filename()).collect();
    let enabled: HashSet<&str> = config
        .discovery
        .manifests
        .iter()
        .map(|s| s.as_str())
        .collect();

    let exclude_set: HashSet<&str> = config.discovery.exclude.iter().map(|s| s.as_str()).collect();

    let mut packages = Vec::new();

    let walker = WalkBuilder::new(repo_root)
        .hidden(true) // skip hidden dirs
        .filter_entry(move |entry| {
            if let Some(name) = entry.file_name().to_str() {
                if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                    return !exclude_set.contains(name);
                }
            }
            true
        })
        .build();

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().map_or(false, |ft| ft.is_file()) {
            continue;
        }

        let filename = match entry.file_name().to_str() {
            Some(f) => f,
            None => continue,
        };

        if !manifest_filenames.contains(filename) || !enabled.contains(filename) {
            continue;
        }

        // Skip the shire project's own Cargo.toml
        let file_path = entry.path();
        let relative_dir = file_path
            .parent()
            .and_then(|p| p.strip_prefix(repo_root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        for parser in &parsers {
            if parser.filename() == filename {
                match parser.parse(file_path, &relative_dir) {
                    Ok(info) => packages.push(info),
                    Err(e) => {
                        eprintln!("Warning: failed to parse {}: {}", file_path.display(), e);
                    }
                }
                break;
            }
        }
    }

    // Collect all package names for is_internal resolution
    let all_names: HashSet<String> = packages.iter().map(|p| p.name.clone()).collect();

    // Insert packages and dependencies
    let mut pkg_stmt = conn.prepare(
        "INSERT OR REPLACE INTO packages (name, path, kind, version, description, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    let mut dep_stmt = conn.prepare(
        "INSERT OR REPLACE INTO dependencies (package, dependency, dep_kind, version_req, is_internal)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;

    for pkg in &packages {
        pkg_stmt.execute((
            &pkg.name,
            &pkg.path,
            pkg.kind,
            &pkg.version,
            &pkg.description,
            &pkg.metadata.as_ref().map(|m| m.to_string()),
        ))?;

        for dep in &pkg.dependencies {
            let is_internal = all_names.contains(&dep.name);
            dep_stmt.execute((
                &pkg.name,
                &dep.name,
                dep.dep_kind.as_str(),
                &dep.version_req,
                is_internal as i32,
            ))?;
        }
    }

    // Apply config overrides
    for override_pkg in &config.packages {
        if let Some(desc) = &override_pkg.description {
            conn.execute(
                "UPDATE packages SET description = ?1 WHERE name = ?2",
                (desc, &override_pkg.name),
            )?;
        }
    }

    // Store metadata
    let git_commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());

    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('indexed_at', ?1)",
        [chrono::Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('package_count', ?1)",
        [packages.len().to_string()],
    )?;
    if let Some(commit) = git_commit {
        conn.execute(
            "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('git_commit', ?1)",
            [commit],
        )?;
    }

    println!("Indexed {} packages into {}", packages.len(), db_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn create_test_monorepo(dir: &Path) {
        // npm package
        let npm_dir = dir.join("services/auth");
        fs::create_dir_all(&npm_dir).unwrap();
        let mut f = fs::File::create(npm_dir.join("package.json")).unwrap();
        f.write_all(
            br#"{"name": "auth-service", "version": "1.0.0", "description": "Auth", "dependencies": {"shared-types": "^1.0"}}"#,
        )
        .unwrap();

        // Another npm package (the dependency)
        let shared_dir = dir.join("packages/shared-types");
        fs::create_dir_all(&shared_dir).unwrap();
        let mut f = fs::File::create(shared_dir.join("package.json")).unwrap();
        f.write_all(
            br#"{"name": "shared-types", "version": "1.0.0", "description": "Shared TypeScript types"}"#,
        )
        .unwrap();

        // Go package
        let go_dir = dir.join("services/gateway");
        fs::create_dir_all(&go_dir).unwrap();
        let mut f = fs::File::create(go_dir.join("go.mod")).unwrap();
        f.write_all(b"module github.com/company/gateway\n\ngo 1.22\n").unwrap();
    }

    #[test]
    fn test_build_index_creates_db() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());

        let config = Config::default();
        build_index(dir.path(), &config).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        assert!(db_path.exists());

        let conn = db::open_readonly(&db_path).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3);

        // Check is_internal flag
        let is_internal: bool = conn
            .query_row(
                "SELECT is_internal FROM dependencies WHERE package='auth-service' AND dependency='shared-types'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(is_internal);
    }

    #[test]
    fn test_fts_search_works() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());

        let config = Config::default();
        build_index(dir.path(), &config).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();

        let results: Vec<String> = conn
            .prepare("SELECT name FROM packages_fts WHERE packages_fts MATCH ?1")
            .unwrap()
            .query_map(["auth"], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(results.contains(&"auth-service".to_string()));
    }
}
```

**Step 2: Wire up the build command in main.rs**

Update the `Build` match arm in `main.rs`:

```rust
Commands::Build { root } => {
    let root = std::fs::canonicalize(&root)?;
    let config = config::load_config(&root)?;
    index::build_index(&root, &config)
}
```

**Step 3: Run tests**

Run: `cargo test index::tests`
Expected: 2 tests pass

Run: `cargo test`
Expected: All tests pass (12+ tests total)

**Step 4: Commit**

```bash
git add src/index/mod.rs src/main.rs
git commit -m "feat: index orchestrator - walks repo, parses manifests, populates SQLite"
```

---

### Task 7: Query Layer

**Files:**
- Modify: `src/db/queries.rs` (add all query functions)

**Step 1: Write query functions with tests**

Replace `src/db/queries.rs`:

```rust
use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct PackageRow {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DependencyRow {
    pub package: String,
    pub dependency: String,
    pub dep_kind: String,
    pub version_req: Option<String>,
    pub is_internal: bool,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub dep_kind: String,
}

#[derive(Debug, Serialize)]
pub struct IndexStatus {
    pub indexed_at: Option<String>,
    pub git_commit: Option<String>,
    pub package_count: Option<String>,
}

pub fn search_packages(conn: &Connection, query: &str) -> Result<Vec<PackageRow>> {
    let mut stmt = conn.prepare(
        "SELECT p.name, p.path, p.kind, p.version, p.description, p.metadata
         FROM packages_fts f
         JOIN packages p ON p.rowid = f.rowid
         WHERE packages_fts MATCH ?1
         LIMIT 20",
    )?;
    let rows = stmt
        .query_map([query], |row| {
            Ok(PackageRow {
                name: row.get(0)?,
                path: row.get(1)?,
                kind: row.get(2)?,
                version: row.get(3)?,
                description: row.get(4)?,
                metadata: row.get(5)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

pub fn get_package(conn: &Connection, name: &str) -> Result<Option<PackageRow>> {
    let mut stmt = conn.prepare(
        "SELECT name, path, kind, version, description, metadata
         FROM packages WHERE name = ?1",
    )?;
    let row = stmt
        .query_row([name], |row| {
            Ok(PackageRow {
                name: row.get(0)?,
                path: row.get(1)?,
                kind: row.get(2)?,
                version: row.get(3)?,
                description: row.get(4)?,
                metadata: row.get(5)?,
            })
        })
        .ok();
    Ok(row)
}

pub fn package_dependencies(
    conn: &Connection,
    name: &str,
    internal_only: bool,
) -> Result<Vec<DependencyRow>> {
    let sql = if internal_only {
        "SELECT package, dependency, dep_kind, version_req, is_internal
         FROM dependencies WHERE package = ?1 AND is_internal = 1"
    } else {
        "SELECT package, dependency, dep_kind, version_req, is_internal
         FROM dependencies WHERE package = ?1"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([name], |row| {
            Ok(DependencyRow {
                package: row.get(0)?,
                dependency: row.get(1)?,
                dep_kind: row.get(2)?,
                version_req: row.get(3)?,
                is_internal: row.get(4)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

pub fn package_dependents(conn: &Connection, name: &str) -> Result<Vec<DependencyRow>> {
    let mut stmt = conn.prepare(
        "SELECT package, dependency, dep_kind, version_req, is_internal
         FROM dependencies WHERE dependency = ?1",
    )?;
    let rows = stmt
        .query_map([name], |row| {
            Ok(DependencyRow {
                package: row.get(0)?,
                dependency: row.get(1)?,
                dep_kind: row.get(2)?,
                version_req: row.get(3)?,
                is_internal: row.get(4)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

pub fn dependency_graph(
    conn: &Connection,
    root: &str,
    max_depth: u32,
    internal_only: bool,
) -> Result<Vec<GraphEdge>> {
    let mut edges = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((root.to_string(), 0u32));

    while let Some((pkg, depth)) = queue.pop_front() {
        if depth >= max_depth || !visited.insert(pkg.clone()) {
            continue;
        }
        let deps = package_dependencies(conn, &pkg, internal_only)?;
        for dep in deps {
            edges.push(GraphEdge {
                from: pkg.clone(),
                to: dep.dependency.clone(),
                dep_kind: dep.dep_kind,
            });
            if dep.is_internal {
                queue.push_back((dep.dependency, depth + 1));
            }
        }
    }
    Ok(edges)
}

pub fn list_packages(conn: &Connection, kind: Option<&str>) -> Result<Vec<PackageRow>> {
    let rows = if let Some(kind) = kind {
        let mut stmt = conn.prepare(
            "SELECT name, path, kind, version, description, metadata
             FROM packages WHERE kind = ?1 ORDER BY name",
        )?;
        stmt.query_map([kind], |row| {
            Ok(PackageRow {
                name: row.get(0)?,
                path: row.get(1)?,
                kind: row.get(2)?,
                version: row.get(3)?,
                description: row.get(4)?,
                metadata: row.get(5)?,
            })
        })?
        .filter_map(Result::ok)
        .collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT name, path, kind, version, description, metadata
             FROM packages ORDER BY name",
        )?;
        stmt.query_map([], |row| {
            Ok(PackageRow {
                name: row.get(0)?,
                path: row.get(1)?,
                kind: row.get(2)?,
                version: row.get(3)?,
                description: row.get(4)?,
                metadata: row.get(5)?,
            })
        })?
        .filter_map(Result::ok)
        .collect()
    };
    Ok(rows)
}

pub fn index_status(conn: &Connection) -> Result<IndexStatus> {
    let get_meta = |key: &str| -> Option<String> {
        conn.query_row(
            "SELECT value FROM shire_meta WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok()
    };

    Ok(IndexStatus {
        indexed_at: get_meta("indexed_at"),
        git_commit: get_meta("git_commit"),
        package_count: get_meta("package_count"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        db::create_schema_for_test(&conn);

        conn.execute(
            "INSERT INTO packages (name, path, kind, version, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("auth-service", "services/auth", "npm", "1.0.0", "Authentication service"),
        ).unwrap();
        conn.execute(
            "INSERT INTO packages (name, path, kind, version, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("shared-types", "packages/shared", "npm", "1.0.0", "Shared types"),
        ).unwrap();
        conn.execute(
            "INSERT INTO packages (name, path, kind, version, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("api-gateway", "services/gateway", "go", "0.1.0", "API gateway"),
        ).unwrap();

        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, is_internal) VALUES (?1, ?2, ?3, ?4)",
            ("auth-service", "shared-types", "runtime", true),
        ).unwrap();
        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, is_internal) VALUES (?1, ?2, ?3, ?4)",
            ("auth-service", "express", "runtime", false),
        ).unwrap();
        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, is_internal) VALUES (?1, ?2, ?3, ?4)",
            ("api-gateway", "auth-service", "runtime", true),
        ).unwrap();

        conn.execute(
            "INSERT INTO shire_meta (key, value) VALUES ('package_count', '3')",
            [],
        ).unwrap();

        conn
    }

    #[test]
    fn test_search_packages() {
        let conn = setup_test_db();
        let results = search_packages(&conn, "auth").unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.name == "auth-service"));
    }

    #[test]
    fn test_get_package() {
        let conn = setup_test_db();
        let pkg = get_package(&conn, "auth-service").unwrap().unwrap();
        assert_eq!(pkg.name, "auth-service");
        assert_eq!(pkg.path, "services/auth");
    }

    #[test]
    fn test_get_package_not_found() {
        let conn = setup_test_db();
        let pkg = get_package(&conn, "nonexistent").unwrap();
        assert!(pkg.is_none());
    }

    #[test]
    fn test_package_dependencies_all() {
        let conn = setup_test_db();
        let deps = package_dependencies(&conn, "auth-service", false).unwrap();
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_package_dependencies_internal_only() {
        let conn = setup_test_db();
        let deps = package_dependencies(&conn, "auth-service", true).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].dependency, "shared-types");
    }

    #[test]
    fn test_package_dependents() {
        let conn = setup_test_db();
        let deps = package_dependents(&conn, "auth-service").unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].package, "api-gateway");
    }

    #[test]
    fn test_dependency_graph() {
        let conn = setup_test_db();
        let edges = dependency_graph(&conn, "api-gateway", 3, true).unwrap();
        // api-gateway -> auth-service -> shared-types
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_list_packages_by_kind() {
        let conn = setup_test_db();
        let pkgs = list_packages(&conn, Some("go")).unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "api-gateway");
    }

    #[test]
    fn test_list_packages_all() {
        let conn = setup_test_db();
        let pkgs = list_packages(&conn, None).unwrap();
        assert_eq!(pkgs.len(), 3);
    }

    #[test]
    fn test_index_status() {
        let conn = setup_test_db();
        let status = index_status(&conn).unwrap();
        assert_eq!(status.package_count.as_deref(), Some("3"));
    }
}
```

**Step 2: Expose create_schema for tests**

Add a test helper to `src/db/mod.rs`:

```rust
#[cfg(test)]
pub fn create_schema_for_test(conn: &Connection) {
    create_schema(conn).unwrap();
}
```

**Step 3: Run tests**

Run: `cargo test db::queries::tests`
Expected: 10 tests pass

**Step 4: Commit**

```bash
git add src/db/
git commit -m "feat: query layer with search, deps, graph, and list"
```

---

### Task 8: MCP Server

**Files:**
- Create: `src/mcp/mod.rs`
- Create: `src/mcp/tools.rs`
- Modify: `src/main.rs` (wire up `serve` command)

This is the critical integration task. The MCP server uses `rmcp` 0.3 with stdio transport to expose query functions as tools.

**Step 1: Write the MCP tool definitions**

In `src/mcp/tools.rs`:

```rust
use crate::db::queries;
use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::*,
    schemars, tool, tool_router,
};
use rusqlite::Connection;
use serde::Deserialize;
use std::borrow::Cow;
use std::sync::Mutex;

#[derive(Debug)]
pub struct ShireService {
    conn: Mutex<Connection>,
    pub tool_router: ToolRouter<ShireService>,
}

impl ShireService {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
            tool_router: Self::tool_router(),
        }
    }

    fn mcp_err(msg: String) -> ErrorData {
        ErrorData {
            code: ErrorCode(-32603),
            message: Cow::from(msg),
            data: None,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Search query to find packages by name or description
    pub query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPackageParams {
    /// Exact package name
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DepsParams {
    /// Package name to look up dependencies for
    pub name: String,
    /// If true, only return dependencies that are also packages in this repo
    #[serde(default)]
    pub internal_only: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DependentsParams {
    /// Package name to find dependents of
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GraphParams {
    /// Root package to start the graph from
    pub name: String,
    /// Maximum depth to traverse (default 3)
    #[serde(default = "default_depth")]
    pub depth: u32,
    /// If true, only follow internal dependencies
    #[serde(default)]
    pub internal_only: bool,
}

fn default_depth() -> u32 {
    3
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Filter by package kind: "npm", "go", "cargo", "python"
    pub kind: Option<String>,
}

#[tool_router]
impl ShireService {
    #[tool(description = "Search packages by name or description using full-text search")]
    fn search_packages(
        &self,
        #[tool(aggr)] params: SearchParams,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results =
            queries::search_packages(&conn, &params.query).map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get full details for a specific package by exact name")]
    fn get_package(
        &self,
        #[tool(aggr)] params: GetPackageParams,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let result =
            queries::get_package(&conn, &params.name).map_err(|e| Self::mcp_err(e.to_string()))?;
        match result {
            Some(pkg) => {
                let json =
                    serde_json::to_string_pretty(&pkg).map_err(|e| Self::mcp_err(e.to_string()))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Package '{}' not found",
                params.name
            ))])),
        }
    }

    #[tool(
        description = "List what a package depends on. Set internal_only=true to see only dependencies that are other packages in this repo."
    )]
    fn package_dependencies(
        &self,
        #[tool(aggr)] params: DepsParams,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::package_dependencies(&conn, &params.name, params.internal_only)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Find all packages that depend on this package (reverse dependency lookup)")]
    fn package_dependents(
        &self,
        #[tool(aggr)] params: DependentsParams,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::package_dependents(&conn, &params.name)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Get the transitive dependency graph starting from a package. Returns a list of edges. Set internal_only=true to only follow dependencies within this repo."
    )]
    fn dependency_graph(
        &self,
        #[tool(aggr)] params: GraphParams,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let edges =
            queries::dependency_graph(&conn, &params.name, params.depth, params.internal_only)
                .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&edges).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all indexed packages, optionally filtered by kind (npm, go, cargo, python)")]
    fn list_packages(
        &self,
        #[tool(aggr)] params: ListParams,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::list_packages(&conn, params.kind.as_deref())
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get index status: when it was built, git commit, and package count")]
    fn index_status(&self) -> Result<CallToolResult, ErrorData> {
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let status =
            queries::index_status(&conn).map_err(|e| Self::mcp_err(e.to_string()))?;
        let json =
            serde_json::to_string_pretty(&status).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
```

**Step 2: Write the MCP server setup**

In `src/mcp/mod.rs`:

```rust
pub mod tools;

use crate::db;
use anyhow::Result;
use rmcp::{model::*, tool_handler, ServiceExt, ServerHandler};
use std::path::Path;

#[tool_handler]
impl ServerHandler for tools::ShireService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "shire".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: Some(
                "Shire indexes monorepo packages and their dependency graph. \
                 Use search_packages to find packages, package_dependencies/package_dependents \
                 to navigate the graph, and dependency_graph for transitive lookups."
                    .into(),
            ),
        }
    }
}

pub async fn run_server(db_path: &Path) -> Result<()> {
    let conn = db::open_readonly(db_path)?;
    let service = tools::ShireService::new(conn);
    let server = service.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
```

**Step 3: Wire up serve command in main.rs**

Update `src/main.rs` to add the `mcp` module and make main async:

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod config;
mod db;
mod index;
mod mcp;

#[derive(Parser)]
#[command(name = "shire", about = "Monorepo package index and MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan the repository and build the package index
    Build {
        /// Root directory of the repository (defaults to current directory)
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Start the MCP server over stdio
    Serve {
        /// Path to the index database (defaults to .shire/index.db)
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Build { root } => {
            let root = std::fs::canonicalize(&root)?;
            let config = config::load_config(&root)?;
            index::build_index(&root, &config)
        }
        Commands::Serve { db } => {
            let db_path = db.unwrap_or_else(|| PathBuf::from(".shire/index.db"));
            if !db_path.exists() {
                anyhow::bail!(
                    "Index not found at {}. Run `shire build` first.",
                    db_path.display()
                );
            }
            mcp::run_server(&db_path).await
        }
    }
}
```

**Step 4: Verify it compiles**

Run: `cargo build`
Expected: Compiles. Note: `rmcp` macro expansion may require adjustments depending on exact API surface. If there are compile errors with the `#[tool_router]`/`#[tool_handler]` macros, consult the [rmcp docs](https://docs.rs/rmcp/latest) and adjust the struct/impl signatures accordingly. The core logic is correct; only the macro annotations may need tweaking.

**Step 5: Commit**

```bash
git add src/mcp/ src/main.rs
git commit -m "feat: MCP server with 7 tools over stdio transport"
```

---

### Task 9: Integration Test

**Files:**
- Create: `tests/integration.rs`

**Step 1: Write an end-to-end integration test**

In `tests/integration.rs`:

```rust
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn cargo_bin() -> std::path::PathBuf {
    // Build in test mode first
    let status = Command::new("cargo")
        .args(["build"])
        .status()
        .expect("Failed to build");
    assert!(status.success());

    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target/debug/shire");
    path
}

fn create_fixture_monorepo(dir: &Path) {
    // npm packages
    let auth = dir.join("services/auth");
    fs::create_dir_all(&auth).unwrap();
    fs::File::create(auth.join("package.json"))
        .unwrap()
        .write_all(
            br#"{
  "name": "auth-service",
  "version": "2.0.0",
  "description": "Handles authentication and authorization",
  "dependencies": { "shared-types": "^1.0", "express": "^4.18" },
  "devDependencies": { "jest": "^29" }
}"#,
        )
        .unwrap();

    let shared = dir.join("packages/shared-types");
    fs::create_dir_all(&shared).unwrap();
    fs::File::create(shared.join("package.json"))
        .unwrap()
        .write_all(br#"{"name": "shared-types", "version": "1.0.0", "description": "Shared TypeScript type definitions"}"#)
        .unwrap();

    let payments = dir.join("services/payments");
    fs::create_dir_all(&payments).unwrap();
    fs::File::create(payments.join("package.json"))
        .unwrap()
        .write_all(
            br#"{
  "name": "payments",
  "version": "1.5.0",
  "description": "Payment processing service",
  "dependencies": { "auth-service": "^2.0", "shared-types": "^1.0" }
}"#,
        )
        .unwrap();

    // Go package
    let gateway = dir.join("services/gateway");
    fs::create_dir_all(&gateway).unwrap();
    fs::File::create(gateway.join("go.mod"))
        .unwrap()
        .write_all(b"module github.com/company/gateway\n\ngo 1.22\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.9.1\n)\n")
        .unwrap();

    // Python package
    let ml = dir.join("services/ml");
    fs::create_dir_all(&ml).unwrap();
    fs::File::create(ml.join("pyproject.toml"))
        .unwrap()
        .write_all(
            br#"[project]
name = "ml-pipeline"
version = "0.3.0"
description = "ML training pipeline"
dependencies = ["torch>=2.0", "numpy"]
"#,
        )
        .unwrap();

    // A node_modules dir that should be skipped
    let nm = dir.join("services/auth/node_modules/leftpad");
    fs::create_dir_all(&nm).unwrap();
    fs::File::create(nm.join("package.json"))
        .unwrap()
        .write_all(br#"{"name": "leftpad", "version": "0.0.1"}"#)
        .unwrap();
}

#[test]
fn test_build_command_indexes_fixture() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "shire build failed.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("Indexed 5 packages"));

    // Verify the db was created
    let db_path = dir.path().join(".shire/index.db");
    assert!(db_path.exists());

    // Verify node_modules was skipped (leftpad should not be indexed)
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE name = 'leftpad'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "node_modules should be excluded");
}
```

**Step 2: Add rusqlite as a dev-dependency for integration tests**

In `Cargo.toml`, update `[dev-dependencies]`:

```toml
[dev-dependencies]
tempfile = "3"
rusqlite = { version = "0.36", features = ["bundled"] }
```

**Step 3: Run integration test**

Run: `cargo test --test integration`
Expected: 1 test passes

**Step 4: Commit**

```bash
git add tests/ Cargo.toml
git commit -m "test: integration test with fixture monorepo"
```

---

### Task 10: Polish and Ship

**Files:**
- Create: `.gitignore`
- Modify: `Cargo.toml` (add metadata for cargo install)

**Step 1: Create .gitignore**

```
/target
.shire/
```

**Step 2: Add publishing metadata to Cargo.toml**

Add under `[package]`:

```toml
license = "MIT"
repository = "https://github.com/YOUR_ORG/shire"
keywords = ["monorepo", "mcp", "index", "search"]
categories = ["command-line-utilities", "development-tools"]
```

**Step 3: Run full test suite**

Run: `cargo test`
Expected: All tests pass (15+ tests)

**Step 4: Verify the binary works end-to-end**

Run: `cargo run -- build --root .`
Expected: Indexes shire's own Cargo.toml as a single package

Run: `cargo run -- serve`
Expected: Starts MCP server (will block waiting for stdio input, Ctrl+C to exit)

**Step 5: Commit**

```bash
git add .gitignore Cargo.toml shire.toml.example
git commit -m "chore: gitignore, publishing metadata, and example config"
```

---

## Summary

| Task | What it builds | Tests |
|------|---------------|-------|
| 1 | Project scaffold, CLI with clap | Manual verify |
| 2 | Database schema (packages, deps, FTS5) | 3 unit tests |
| 3 | ManifestParser trait + npm parser | 3 unit tests |
| 4 | Go, Cargo, Python parsers | 4 unit tests |
| 5 | Config module (shire.toml) | 3 unit tests |
| 6 | Index orchestrator (walk + populate) | 2 unit tests |
| 7 | Query layer (all query functions) | 10 unit tests |
| 8 | MCP server (7 tools, stdio) | Compile check |
| 9 | Integration test (fixture monorepo) | 1 integration test |
| 10 | Polish (.gitignore, metadata) | Full suite |
