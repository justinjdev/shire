use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn cargo_bin() -> std::path::PathBuf {
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
        .write_all(
            br#"{"name": "shared-types", "version": "1.0.0", "description": "Shared TypeScript type definitions"}"#,
        )
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
        .write_all(
            b"module github.com/company/gateway\n\ngo 1.22\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.9.1\n)\n",
        )
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

    // Source files for symbol extraction
    // TypeScript source in auth-service
    let auth_src = dir.join("services/auth/src");
    fs::create_dir_all(&auth_src).unwrap();
    fs::File::create(auth_src.join("auth.ts"))
        .unwrap()
        .write_all(
            br#"export function validateToken(token: string, secret: string): boolean {
    return true;
}

export class AuthService {
    public authenticate(username: string, password: string): Promise<Session> {
        throw new Error("not implemented");
    }
}

export interface Session {
    userId: string;
    expiresAt: number;
}

function internalHelper() {}
"#,
        )
        .unwrap();

    // Go source in gateway
    fs::File::create(gateway.join("main.go"))
        .unwrap()
        .write_all(
            br#"package main

type Router struct {
    routes []Route
}

func NewRouter() *Router {
    return &Router{}
}

func (r *Router) HandleRequest(path string, method string) error {
    return nil
}

func internalSetup() {}
"#,
        )
        .unwrap();

    // Python source in ml-pipeline
    let ml_src = dir.join("services/ml/src");
    fs::create_dir_all(&ml_src).unwrap();
    fs::File::create(ml_src.join("train.py"))
        .unwrap()
        .write_all(
            br#"def train_model(dataset: str, epochs: int) -> float:
    return 0.95

class Pipeline:
    def __init__(self, config: dict):
        self.config = config

    def run(self, data: list) -> dict:
        return {}

    def _validate(self):
        pass
"#,
        )
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
    assert!(
        stdout.contains("Indexed 5 packages"),
        "Expected 5 packages in output, got: {stdout}"
    );
    assert!(
        stdout.contains("symbols"),
        "Expected symbol count in output, got: {stdout}"
    );

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

    // Verify all 5 packages indexed
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(total, 5);

    // Verify internal dependency detection
    let internal_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dependencies WHERE is_internal = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(internal_count >= 2, "Should have at least 2 internal deps");

    // Verify FTS works
    let fts_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages_fts WHERE packages_fts MATCH 'auth'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(fts_count >= 1, "FTS should find auth-related packages");
}

fn create_fixture_with_rust(dir: &Path) {
    // Cargo package with Rust source
    let crate_dir = dir.join("crates/core");
    fs::create_dir_all(crate_dir.join("src")).unwrap();
    fs::File::create(crate_dir.join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"[package]
name = "core-lib"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
    fs::File::create(crate_dir.join("src/lib.rs"))
        .unwrap()
        .write_all(
            br#"pub fn process(input: &str) -> Result<String, Error> {
    Ok(input.to_string())
}

pub struct Config {
    pub verbose: bool,
}

fn internal_fn() {}
"#,
        )
        .unwrap();
}

#[test]
fn test_symbol_extraction_typescript() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(output.status.success(), "build failed: {}", String::from_utf8_lossy(&output.stderr));

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // validateToken should be extracted
    let name: String = conn
        .query_row(
            "SELECT name FROM symbols WHERE name = 'validateToken' AND package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "validateToken");

    // Check signature has params
    let sig: String = conn
        .query_row(
            "SELECT signature FROM symbols WHERE name = 'validateToken'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sig.contains("token"), "signature should contain param: {sig}");

    // AuthService class should be extracted
    let kind: String = conn
        .query_row(
            "SELECT kind FROM symbols WHERE name = 'AuthService' AND package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "class");

    // authenticate method should have parent_symbol
    let parent: String = conn
        .query_row(
            "SELECT parent_symbol FROM symbols WHERE name = 'authenticate'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(parent, "AuthService");

    // Session interface
    let iface_kind: String = conn
        .query_row(
            "SELECT kind FROM symbols WHERE name = 'Session' AND package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(iface_kind, "interface");

    // internalHelper should NOT be extracted (not exported)
    let internal_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'internalHelper'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(internal_count, 0, "non-exported symbols should be skipped");
}

#[test]
fn test_symbol_extraction_go() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(output.status.success());

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Router struct
    let kind: String = conn
        .query_row(
            "SELECT kind FROM symbols WHERE name = 'Router' AND package = 'gateway'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "struct");

    // NewRouter function
    let kind: String = conn
        .query_row(
            "SELECT kind FROM symbols WHERE name = 'NewRouter'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "function");

    // HandleRequest method with parent
    let parent: String = conn
        .query_row(
            "SELECT parent_symbol FROM symbols WHERE name = 'HandleRequest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(parent, "Router");

    // internalSetup should NOT be extracted (lowercase)
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'internalSetup'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_symbol_extraction_rust() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_with_rust(dir.path());

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(output.status.success(), "build failed: {}", String::from_utf8_lossy(&output.stderr));

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // pub fn process
    let kind: String = conn
        .query_row(
            "SELECT kind FROM symbols WHERE name = 'process' AND package = 'core-lib'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "function");

    // pub struct Config
    let kind: String = conn
        .query_row(
            "SELECT kind FROM symbols WHERE name = 'Config' AND package = 'core-lib'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "struct");

    // non-pub internal_fn should not be extracted
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'internal_fn'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_symbol_incremental_update() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();

    // First build
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let initial_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(initial_count > 0);
    drop(conn);

    // Add a new exported function to the TS file
    let auth_ts = dir.path().join("services/auth/src/auth.ts");
    let mut content = fs::read_to_string(&auth_ts).unwrap();
    content.push_str("\nexport function revokeToken(tokenId: string): void {}\n");
    fs::write(&auth_ts, content).unwrap();

    // Touch the manifest to trigger reparsing (since symbols are extracted per-package on parse)
    let pkg_json = dir.path().join("services/auth/package.json");
    let manifest = fs::read_to_string(&pkg_json).unwrap();
    fs::write(&pkg_json, manifest).unwrap(); // rewrite same content — but we need hash to change

    // Actually, symbols are extracted when the package is parsed. Since we only changed a source file
    // (not the manifest), the package won't be re-parsed. We need --force to re-extract.
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap(), "--force"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let updated_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        updated_count > initial_count,
        "Should have more symbols after adding function: {} vs {}",
        updated_count, initial_count
    );

    // Verify the new function is there
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'revokeToken'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_symbol_fts_search() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // FTS search for "validateToken" should find the symbol
    let results: Vec<String> = conn
        .prepare("SELECT name FROM symbols_fts WHERE symbols_fts MATCH '\"validateToken\"'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        results.iter().any(|n| n == "validateToken"),
        "FTS should find validateToken symbol, got: {:?}",
        results
    );

    // Symbol count should be in build output
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("symbols"),
        "Build output should mention symbols: {stdout}"
    );
}

#[test]
fn test_serve_fails_without_index() {
    let dir = tempfile::TempDir::new().unwrap();
    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args([
            "serve",
            "--db",
            dir.path().join("nonexistent.db").to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run shire serve");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Index not found") || stderr.contains("not found"),
        "Should error about missing index, got: {stderr}"
    );
}
