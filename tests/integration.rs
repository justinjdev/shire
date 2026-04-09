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

fn cargo_bin_rag() -> std::path::PathBuf {
    let status = Command::new("cargo")
        .args(["build", "--features", "rag"])
        .status()
        .expect("Failed to build with rag feature");
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

    // Hybrid directory with BOTH package.json and go.mod — triggers path dedup
    let hybrid = dir.join("services/hybrid");
    fs::create_dir_all(&hybrid).unwrap();
    fs::File::create(hybrid.join("package.json"))
        .unwrap()
        .write_all(
            br#"{"name": "hybrid-js", "version": "1.0.0", "description": "JS side of hybrid service", "dependencies": {"shared-types": "^1.0"}}"#,
        )
        .unwrap();
    fs::File::create(hybrid.join("go.mod"))
        .unwrap()
        .write_all(
            b"module github.com/company/hybrid\n\ngo 1.22\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.9.1\n)\n",
        )
        .unwrap();

    // Source files in hybrid for symbol extraction (both languages)
    fs::File::create(hybrid.join("index.ts"))
        .unwrap()
        .write_all(
            br#"export function hybridHandler(req: Request): Response {
    return new Response("ok");
}
"#,
        )
        .unwrap();
    fs::File::create(hybrid.join("main.go"))
        .unwrap()
        .write_all(
            br#"package main

func HybridServe(addr string) error {
    return nil
}
"#,
        )
        .unwrap();

    // Default config to isolate tests from any global ~/.claude/shire.toml
    fs::write(dir.join("shire.toml"), "db_path = \".shire/index.db\"\n").unwrap();

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
        stdout.contains("Indexed 6 packages"),
        "Expected 6 packages in output, got: {stdout}"
    );
    assert!(
        stdout.contains("symbols"),
        "Expected symbol count in output, got: {stdout}"
    );
    assert!(
        stdout.contains("files"),
        "Expected file count in output, got: {stdout}"
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

    // Verify all 6 packages indexed (5 original + 1 from hybrid dir path-dedup winner)
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(total, 6);

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
    // Default config to isolate tests from any global ~/.claude/shire.toml
    fs::write(dir.join("shire.toml"), "db_path = \".shire/index.db\"\n").unwrap();

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
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

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
    assert!(
        sig.contains("token"),
        "signature should contain param: {sig}"
    );

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
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

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
        updated_count,
        initial_count
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
fn test_source_change_triggers_reextraction() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();

    // First build — symbols extracted, source hashes stored
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
    // Verify source hash was stored
    let has_hash: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM source_hashes WHERE package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(has_hash, "source hash should be stored after first build");
    drop(conn);

    // Modify a source file WITHOUT touching the manifest
    let auth_ts = dir.path().join("services/auth/src/auth.ts");
    let mut content = fs::read_to_string(&auth_ts).unwrap();
    content.push_str("\nexport function revokeToken(tokenId: string): void {}\n");
    fs::write(&auth_ts, content).unwrap();

    // Second build — manifest unchanged, but source hash differs → Phase 8 re-extracts
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("source-updated"),
        "Should report source-updated in summary: {stdout}"
    );

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
        updated_count,
        initial_count
    );

    // Verify the new symbol is present
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'revokeToken' AND package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "revokeToken should be extracted via source-level incremental"
    );
}

#[test]
fn test_no_source_change_skips_reextraction() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();

    // First build
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Second build — nothing changed at all
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("source-updated"),
        "Should NOT report source-updated when nothing changed: {stdout}"
    );
}

#[test]
fn test_force_clears_and_recomputes_source_hashes() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();

    // First build — stores source hashes
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let hash_count_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_hashes", [], |row| row.get(0))
        .unwrap();
    assert!(
        hash_count_before > 0,
        "should have source hashes after first build"
    );
    drop(conn);

    // Force rebuild
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap(), "--force"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Source hashes should be recomputed (same count as before)
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let hash_count_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_hashes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        hash_count_before, hash_count_after,
        "source hashes should be recomputed after --force"
    );
}

#[test]
fn test_delete_package_removes_source_hash() {
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
    let has_gateway_hash: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM source_hashes WHERE package = 'gateway'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(has_gateway_hash, "gateway should have a source hash");
    drop(conn);

    // Delete the gateway package manifest
    fs::remove_file(dir.path().join("services/gateway/go.mod")).unwrap();

    // Rebuild
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let has_gateway_hash: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM source_hashes WHERE package = 'gateway'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !has_gateway_hash,
        "gateway source hash should be removed after package deletion"
    );

    // Also verify the package itself is gone
    let pkg_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE name = 'gateway'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pkg_count, 0);
}

#[test]
fn test_add_source_file_updates_symbols() {
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
    drop(conn);

    // Add a new source file to auth-service (without touching package.json)
    let auth_src = dir.path().join("services/auth/src");
    fs::File::create(auth_src.join("permissions.ts"))
        .unwrap()
        .write_all(
            br#"export function checkPermission(userId: string, resource: string): boolean {
    return true;
}

export class PermissionManager {
    public grant(userId: string, permission: string): void {}
}
"#,
        )
        .unwrap();

    // Rebuild — source hash changes due to new file, Phase 8 re-extracts
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
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
        "Should have more symbols after adding new file: {} vs {}",
        updated_count,
        initial_count
    );

    // Verify the new file's exports are present
    let check_perm: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'checkPermission' AND package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        check_perm, 1,
        "checkPermission should be extracted from new file"
    );

    let perm_mgr: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'PermissionManager' AND package = 'auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        perm_mgr, 1,
        "PermissionManager should be extracted from new file"
    );
}

#[test]
fn test_file_index_populates_with_associations() {
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

    // Files table should have entries
    let file_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap();
    assert!(file_count > 0, "files table should have entries");

    // auth.ts should be associated with auth-service
    let pkg: Option<String> = conn
        .query_row(
            "SELECT package FROM files WHERE path = 'services/auth/src/auth.ts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pkg.as_deref(), Some("auth-service"));

    // main.go should be associated with gateway
    let pkg: Option<String> = conn
        .query_row(
            "SELECT package FROM files WHERE path = 'services/gateway/main.go'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pkg.as_deref(), Some("gateway"));

    // Extension should be extracted
    let ext: String = conn
        .query_row(
            "SELECT extension FROM files WHERE path = 'services/auth/src/auth.ts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ext, "ts");
}

#[test]
fn test_file_index_excludes_node_modules() {
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

    // No files from node_modules
    let nm_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path LIKE '%node_modules%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(nm_count, 0, "files in node_modules should be excluded");
}

#[test]
fn test_file_index_null_package_for_unowned_files() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    // Add a file outside any package
    fs::write(dir.path().join("scripts.sh"), "#!/bin/bash\necho hello").unwrap();

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    let pkg: Option<String> = conn
        .query_row(
            "SELECT package FROM files WHERE path = 'scripts.sh'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        pkg.is_none(),
        "file outside any package should have NULL package"
    );
}

#[test]
fn test_file_count_in_shire_meta() {
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

    let file_count: String = conn
        .query_row(
            "SELECT value FROM shire_meta WHERE key = 'file_count'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let count: i64 = file_count.parse().unwrap();
    assert!(count > 0, "file_count should be in shire_meta");
}

#[test]
fn test_file_index_rebuild_includes_new_file() {
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
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap();
    drop(conn);

    // Add a new file
    let auth_src = dir.path().join("services/auth/src");
    fs::write(auth_src.join("utils.ts"), "export function helper() {}").unwrap();

    // Rebuild
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let updated_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        updated_count,
        initial_count + 1,
        "rebuild should include the new file"
    );

    // New file should be present and associated
    let pkg: Option<String> = conn
        .query_row(
            "SELECT package FROM files WHERE path = 'services/auth/src/utils.ts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pkg.as_deref(), Some("auth-service"));
}

// ===== Maven/Gradle Integration Tests =====

#[test]
fn test_maven_index_basic() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("shire.toml"),
        "db_path = \".shire/index.db\"\n",
    )
    .unwrap();
    let bin = cargo_bin();

    // Create a Maven project
    let svc_dir = dir.path().join("services/auth");
    fs::create_dir_all(&svc_dir).unwrap();
    fs::write(
        svc_dir.join("pom.xml"),
        r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
    <artifactId>auth-service</artifactId>
    <version>1.0.0</version>
    <description>Auth service</description>
    <dependencies>
        <dependency>
            <groupId>com.google.guava</groupId>
            <artifactId>guava</artifactId>
            <version>32.1</version>
        </dependency>
        <dependency>
            <groupId>junit</groupId>
            <artifactId>junit</artifactId>
            <version>4.13</version>
            <scope>test</scope>
        </dependency>
    </dependencies>
</project>"#,
    )
    .unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    // Check package indexed correctly
    let name: String = conn
        .query_row(
            "SELECT name FROM packages WHERE kind = 'maven'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "com.example:auth-service");

    // Check dependencies with scope mapping
    let dep_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dependencies WHERE package = 'com.example:auth-service'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dep_count, 2);

    let junit_kind: String = conn
        .query_row(
            "SELECT dep_kind FROM dependencies WHERE dependency = 'junit:junit'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(junit_kind, "dev");
}

#[test]
fn test_maven_parent_pom_inheritance() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("shire.toml"),
        "db_path = \".shire/index.db\"\n",
    )
    .unwrap();
    let bin = cargo_bin();

    // Parent POM at root (aggregator — not indexed as package)
    fs::write(
        dir.path().join("pom.xml"),
        r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>2.0.0</version>
    <packaging>pom</packaging>
    <modules>
        <module>child</module>
    </modules>
    <dependencyManagement>
        <dependencies>
            <dependency>
                <groupId>com.google.guava</groupId>
                <artifactId>guava</artifactId>
                <version>32.1</version>
            </dependency>
        </dependencies>
    </dependencyManagement>
</project>"#,
    )
    .unwrap();

    // Child POM inheriting from parent
    let child_dir = dir.path().join("child");
    fs::create_dir_all(&child_dir).unwrap();
    fs::write(
        child_dir.join("pom.xml"),
        r#"<?xml version="1.0"?>
<project>
    <parent>
        <groupId>com.example</groupId>
        <artifactId>parent</artifactId>
        <version>2.0.0</version>
    </parent>
    <artifactId>child-service</artifactId>
    <dependencies>
        <dependency>
            <groupId>com.google.guava</groupId>
            <artifactId>guava</artifactId>
        </dependency>
    </dependencies>
</project>"#,
    )
    .unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    // Only child should be indexed (parent is aggregator POM)
    let pkg_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(pkg_count, 1);

    // Child inherits groupId from parent
    let name: String = conn
        .query_row("SELECT name FROM packages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "com.example:child-service");

    // Child inherits version from parent
    let version: String = conn
        .query_row("SELECT version FROM packages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, "2.0.0");

    // Guava version resolved from parent's dependencyManagement
    let guava_ver: Option<String> = conn
        .query_row(
            "SELECT version_req FROM dependencies WHERE dependency = 'com.google.guava:guava'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(guava_ver.as_deref(), Some("32.1"));
}

#[test]
fn test_gradle_index_basic() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("shire.toml"),
        "db_path = \".shire/index.db\"\n",
    )
    .unwrap();
    let bin = cargo_bin();

    let app_dir = dir.path().join("app");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("build.gradle"),
        r#"
group = 'com.example'
version = '1.0.0'

dependencies {
    implementation 'com.google.guava:guava:32.1'
    testImplementation 'junit:junit:4.13'
    compileOnly 'javax.servlet:javax.servlet-api:4.0.1'
}
"#,
    )
    .unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    let name: String = conn
        .query_row(
            "SELECT name FROM packages WHERE kind = 'gradle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "com.example:app");

    let dep_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dependencies WHERE package = 'com.example:app'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dep_count, 3);
}

#[test]
fn test_gradle_settings_workspace() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("shire.toml"),
        "db_path = \".shire/index.db\"\n",
    )
    .unwrap();
    let bin = cargo_bin();

    // settings.gradle
    fs::write(
        dir.path().join("settings.gradle"),
        r#"
rootProject.name = 'my-project'
include ':app', ':lib'
"#,
    )
    .unwrap();

    // Root build.gradle
    fs::write(
        dir.path().join("build.gradle"),
        "group = 'com.example'\nversion = '1.0.0'\n",
    )
    .unwrap();

    // app subproject
    let app_dir = dir.path().join("app");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("build.gradle"),
        "group = 'com.example'\nversion = '1.0.0'\n\ndependencies {\n    implementation project(':lib')\n}\n",
    )
    .unwrap();

    // lib subproject
    let lib_dir = dir.path().join("lib");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::write(
        lib_dir.join("build.gradle"),
        "group = 'com.example'\nversion = '1.0.0'\n",
    )
    .unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    // 3 packages: root, app, lib
    let pkg_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE kind = 'gradle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pkg_count, 3);

    // app and lib should have gradle_workspace metadata
    let app_meta: Option<String> = conn
        .query_row(
            "SELECT metadata FROM packages WHERE name = 'com.example:app'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(app_meta.is_some());
    let meta: serde_json::Value = serde_json::from_str(app_meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["gradle_workspace"], true);
}

#[test]
fn test_mixed_maven_gradle_ecosystem() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("shire.toml"),
        "db_path = \".shire/index.db\"\n",
    )
    .unwrap();
    let bin = cargo_bin();

    // Maven project
    let maven_dir = dir.path().join("services/auth");
    fs::create_dir_all(&maven_dir).unwrap();
    fs::write(
        maven_dir.join("pom.xml"),
        r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
    <artifactId>auth</artifactId>
    <version>1.0.0</version>
</project>"#,
    )
    .unwrap();

    // Gradle project
    let gradle_dir = dir.path().join("services/web");
    fs::create_dir_all(&gradle_dir).unwrap();
    fs::write(
        gradle_dir.join("build.gradle.kts"),
        "group = \"com.example\"\nversion = \"2.0.0\"\n",
    )
    .unwrap();

    // npm project
    let npm_dir = dir.path().join("frontend");
    fs::create_dir_all(&npm_dir).unwrap();
    fs::write(
        npm_dir.join("package.json"),
        r#"{"name": "frontend", "version": "1.0.0"}"#,
    )
    .unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    // All 3 packages indexed
    let pkg_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(pkg_count, 3);

    // Check each kind
    let maven_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE kind = 'maven'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(maven_count, 1);

    let gradle_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE kind = 'gradle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(gradle_count, 1);

    let npm_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE kind = 'npm'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(npm_count, 1);
}

#[test]
fn test_maven_incremental_rebuild() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("shire.toml"),
        "db_path = \".shire/index.db\"\n",
    )
    .unwrap();
    let bin = cargo_bin();

    let svc_dir = dir.path().join("svc");
    fs::create_dir_all(&svc_dir).unwrap();
    fs::write(
        svc_dir.join("pom.xml"),
        r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
    <artifactId>my-app</artifactId>
    <version>1.0.0</version>
</project>"#,
    )
    .unwrap();

    // First build
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Modify version
    fs::write(
        svc_dir.join("pom.xml"),
        r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
    <artifactId>my-app</artifactId>
    <version>2.0.0</version>
</project>"#,
    )
    .unwrap();

    // Second build — incremental
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("updated"),
        "Should show updated count: {stdout}"
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    let version: String = conn
        .query_row(
            "SELECT version FROM packages WHERE name = 'com.example:my-app'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "2.0.0");
}

/// Regression test for https://github.com/justinjdev/shire/issues/1
/// INSERT OR REPLACE INTO packages acts as DELETE+INSERT, which triggers FK
/// violations on child tables (dependencies, symbols) that reference the old row.
/// The fix uses ON CONFLICT ... DO UPDATE instead.
#[test]
fn test_rebuild_no_fk_violation() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    let bin = cargo_bin();

    // First build — populates packages, dependencies, symbols
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "First build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify child rows exist so a DELETE on the parent would violate FKs
    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let dep_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM dependencies", [], |row| row.get(0))
        .unwrap();
    assert!(dep_count > 0, "Should have dependencies after first build");
    let sym_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))
        .unwrap();
    assert!(sym_count > 0, "Should have symbols after first build");
    drop(conn);

    // Force rebuild — this would trigger FK violation with INSERT OR REPLACE
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap(), "--force"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Force rebuild should not fail with FK violation: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify data is still intact after rebuild
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let pkg_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .unwrap();
    assert!(pkg_count > 0, "Packages should still exist after rebuild");
    let dep_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM dependencies", [], |row| row.get(0))
        .unwrap();
    assert!(
        dep_count > 0,
        "Dependencies should still exist after rebuild"
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

#[test]
fn test_dual_manifest_same_directory_no_fk_violation() {
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
        "Build should succeed with dual manifests.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Exactly one package should exist at path services/hybrid
    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let hybrid_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE path = 'services/hybrid'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        hybrid_count, 1,
        "Path dedup should keep exactly one package for services/hybrid"
    );

    // The surviving package should have symbols
    let winner: String = conn
        .query_row(
            "SELECT name FROM packages WHERE path = 'services/hybrid'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let sym_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE package = ?1",
            [&winner],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        sym_count > 0,
        "Surviving package '{}' should have symbols, got {}",
        winner,
        sym_count
    );

    // No orphaned symbols should reference the loser
    let orphaned: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE package NOT IN (SELECT name FROM packages)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orphaned, 0, "No orphaned symbols should exist");

    // Force rebuild should also succeed
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap(), "--force"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Force rebuild with dual manifests should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_kind_agnostic_symbol_extraction_proto_in_gradle() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(
        dir.path().join("shire.toml"),
        "db_path = \".shire/index.db\"\n",
    )
    .unwrap();
    let bin = cargo_bin();

    // Gradle package with a proto file inside
    let svc_dir = dir.path().join("services/api");
    fs::create_dir_all(&svc_dir).unwrap();
    fs::write(
        svc_dir.join("build.gradle"),
        "group = 'com.example'\nversion = '1.0.0'\n",
    )
    .unwrap();

    // Proto file in the same package directory
    let proto_dir = svc_dir.join("src/main/proto");
    fs::create_dir_all(&proto_dir).unwrap();
    fs::write(
        proto_dir.join("service.proto"),
        r#"syntax = "proto3";
package com.example.api;

message PaymentRequest {
  string currency = 1;
  double amount = 2;
}

service PaymentAPI {
  rpc ProcessPayment(PaymentRequest) returns (PaymentResponse) {}
}

message PaymentResponse {
  bool success = 1;
}
"#,
    )
    .unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Proto message extracted as struct
    let kind: String = conn
        .query_row(
            "SELECT kind FROM symbols WHERE name = 'PaymentRequest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "struct");

    // Proto service extracted as interface
    let kind: String = conn
        .query_row(
            "SELECT kind FROM symbols WHERE name = 'PaymentAPI'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "interface");

    // Proto RPC extracted as method with parent
    let parent: String = conn
        .query_row(
            "SELECT parent_symbol FROM symbols WHERE name = 'ProcessPayment'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(parent, "PaymentAPI");

    // All symbols belong to the gradle package
    let pkg: String = conn
        .query_row(
            "SELECT package FROM symbols WHERE name = 'PaymentRequest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pkg, "com.example:api");
}

#[test]
fn test_exclude_extensions_config() {
    let dir = tempfile::TempDir::new().unwrap();
    let bin = cargo_bin();

    // Gradle package with a proto file
    let svc_dir = dir.path().join("services/api");
    fs::create_dir_all(&svc_dir).unwrap();
    fs::write(
        svc_dir.join("build.gradle"),
        "group = 'com.example'\nversion = '1.0.0'\n",
    )
    .unwrap();
    fs::write(
        svc_dir.join("service.proto"),
        r#"syntax = "proto3";
message Ignored {
  string field = 1;
}
"#,
    )
    .unwrap();

    // Config to exclude proto
    fs::write(
        dir.path().join("shire.toml"),
        r#"
[symbols]
exclude_extensions = [".proto"]
"#,
    )
    .unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Proto symbols should NOT be extracted
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'Ignored'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "Proto symbols should be excluded by config");
}

// RAG integration tests — require network for model download on first run.
// Run with: cargo test --test integration rag -- --ignored

#[test]
#[ignore]
fn test_rag_build_populates_embeddings() {
    let bin = cargo_bin_rag();
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    // Write config with RAG enabled
    fs::File::create(dir.path().join("shire.toml"))
        .unwrap()
        .write_all(
            br#"
[rag]
enabled = true
"#,
        )
        .unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Verify symbol_embeddings table exists and is populated
    let embed_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbol_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(
        embed_count > 0,
        "Expected symbol_embeddings to be populated, got {embed_count}"
    );

    // Verify embedding count roughly matches symbol count
    let symbol_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        embed_count, symbol_count,
        "Expected one embedding per symbol: {embed_count} embeddings vs {symbol_count} symbols"
    );

    eprintln!("RAG build: {symbol_count} symbols, {embed_count} embeddings");
}

#[test]
#[ignore]
fn test_rag_disabled_no_embeddings() {
    let bin = cargo_bin_rag();
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    // No [rag] section → disabled by default
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(output.status.success());

    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // symbol_embeddings table should not exist when RAG is disabled
    // (vec0 extension is loaded but init_table is skipped)
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='symbol_embeddings'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        > 0;
    assert!(
        !table_exists,
        "symbol_embeddings should not exist when RAG is disabled"
    );
}

fn git_init_repo(dir: &Path) {
    let output = Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .expect("git init failed");
    assert!(output.status.success());
    let output = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .expect("git commit failed");
    assert!(output.status.success());
}

fn create_minimal_fixture(dir: &Path) {
    let pkg = dir.join("pkg");
    fs::create_dir_all(&pkg).unwrap();
    fs::File::create(pkg.join("package.json"))
        .unwrap()
        .write_all(br#"{"name": "test-pkg", "version": "1.0.0", "description": "A test"}"#)
        .unwrap();
}

#[test]
fn test_build_main_worktree_db_path() {
    let bin = cargo_bin();
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().join("my-project");
    fs::create_dir(&repo).unwrap();
    git_init_repo(&repo);
    create_minimal_fixture(&repo);

    let db_base = dir.path().join("dbs");
    let config_content = format!(
        "db_path = \"{}/{{repo}}/{{worktree}}/index.db\"\n",
        db_base.display()
    );
    fs::write(repo.join("shire.toml"), &config_content).unwrap();

    let output = Command::new(&bin)
        .args(["build", "--root", repo.to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let expected_db = db_base.join("my-project").join("_primary").join("index.db");
    assert!(
        expected_db.exists(),
        "Expected DB at {} but it doesn't exist",
        expected_db.display()
    );

    let conn = rusqlite::Connection::open(&expected_db).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .unwrap();
    assert!(count > 0, "DB should have packages");
}

#[test]
fn test_build_linked_worktree_db_path() {
    let bin = cargo_bin();
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().join("my-project");
    fs::create_dir(&repo).unwrap();
    git_init_repo(&repo);
    create_minimal_fixture(&repo);

    let db_base = dir.path().join("dbs");
    let config_content = format!(
        "db_path = \"{}/{{repo}}/{{worktree}}/index.db\"\n",
        db_base.display()
    );
    fs::write(repo.join("shire.toml"), &config_content).unwrap();

    // Commit so config is available in worktree
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add fixture"])
        .current_dir(&repo)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();

    // Create a linked worktree
    let wt_path = dir.path().join("feat-branch");
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            wt_path.to_str().unwrap(),
            "-b",
            "feat-branch",
        ])
        .current_dir(&repo)
        .output()
        .expect("git worktree add failed");
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Build from the worktree
    let output = Command::new(&bin)
        .args(["build", "--root", wt_path.to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build in worktree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // DB should be at <db_base>/my-project/feat-branch/index.db
    let expected_db = db_base
        .join("my-project")
        .join("feat-branch")
        .join("index.db");
    assert!(
        expected_db.exists(),
        "Expected DB at {} but it doesn't exist",
        expected_db.display()
    );

    // {repo} should resolve to main repo name, not worktree dir name
    let wrong_db = db_base
        .join("feat-branch")
        .join("feat-branch")
        .join("index.db");
    assert!(
        !wrong_db.exists(),
        "DB should NOT be at {} (wrong repo name)",
        wrong_db.display()
    );

    let conn = rusqlite::Connection::open(&expected_db).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .unwrap();
    assert!(count > 0, "Worktree DB should have packages");
}

#[test]
fn test_worktree_seed_from_main_db() {
    let bin = cargo_bin();
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().join("seed-project");
    fs::create_dir(&repo).unwrap();
    git_init_repo(&repo);
    create_minimal_fixture(&repo);

    let db_base = dir.path().join("dbs");
    let config_content = format!(
        "db_path = \"{}/{{repo}}/{{worktree}}/index.db\"\n",
        db_base.display()
    );
    fs::write(repo.join("shire.toml"), &config_content).unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add fixture"])
        .current_dir(&repo)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();

    // Build main worktree first
    let output = Command::new(&bin)
        .args(["build", "--root", repo.to_str().unwrap()])
        .output()
        .expect("Failed to run shire build on main");
    assert!(
        output.status.success(),
        "Main build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let main_db = db_base
        .join("seed-project")
        .join("_primary")
        .join("index.db");
    assert!(
        main_db.exists(),
        "Main DB should exist at {}",
        main_db.display()
    );

    // Create a linked worktree and build
    let wt_path = dir.path().join("seed-branch");
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            wt_path.to_str().unwrap(),
            "-b",
            "seed-branch",
        ])
        .current_dir(&repo)
        .output()
        .expect("git worktree add failed");
    assert!(output.status.success());

    let output = Command::new(&bin)
        .args(["build", "--root", wt_path.to_str().unwrap()])
        .output()
        .expect("Failed to run shire build on worktree");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "Worktree build failed: {stderr}");
    assert!(
        stderr.contains("Seeded DB from"),
        "Should print seed message, got stderr: {stderr}"
    );

    // Verify worktree DB has data
    let wt_db = db_base
        .join("seed-project")
        .join("seed-branch")
        .join("index.db");
    assert!(wt_db.exists());
    let conn = rusqlite::Connection::open(&wt_db).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
        .unwrap();
    assert!(count > 0, "Worktree DB should have packages from seed");
    drop(conn);

    // Rebuild should NOT re-seed
    let output = Command::new(&bin)
        .args(["build", "--root", wt_path.to_str().unwrap()])
        .output()
        .expect("Failed to run shire rebuild on worktree");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success());
    assert!(
        !stderr.contains("Seeded DB from"),
        "Should NOT re-seed on second build"
    );
}

#[test]
fn test_docs_indexing_and_search() {
    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    // Add documentation files
    let auth_docs = dir.path().join("services/auth/docs");
    fs::create_dir_all(&auth_docs).unwrap();
    fs::File::create(auth_docs.join("setup.md"))
        .unwrap()
        .write_all(b"# Authentication Setup\n\nHow to configure OAuth providers and JWT tokens for the auth service.\n")
        .unwrap();

    fs::File::create(dir.path().join("README.md"))
        .unwrap()
        .write_all(
            b"# Monorepo Overview\n\nThis project contains multiple services for the platform.\n",
        )
        .unwrap();

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to run shire build");
    assert!(
        output.status.success(),
        "Build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("docs"),
        "Build output should mention docs: {}",
        stdout
    );

    // Verify docs were indexed
    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    let doc_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM docs", [], |row| row.get(0))
        .unwrap();
    assert!(
        doc_count >= 2,
        "Expected at least 2 docs indexed, got {}",
        doc_count
    );

    // Verify FTS search works
    let results: Vec<String> = conn
        .prepare("SELECT path FROM docs_fts WHERE docs_fts MATCH '\"OAuth\"' ORDER BY rank")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!results.is_empty(), "Should find OAuth in docs FTS");
    assert!(results.iter().any(|p| p.contains("setup.md")));

    // Verify title extraction
    let title: Option<String> = conn
        .query_row(
            "SELECT title FROM docs WHERE path LIKE '%setup.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title.as_deref(), Some("Authentication Setup"));

    // Verify package association
    let pkg: Option<String> = conn
        .query_row(
            "SELECT package FROM docs WHERE path LIKE '%setup.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pkg.as_deref(), Some("auth-service"));
}

#[test]
fn test_references_incremental_rebuild() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    // Write shire.toml to isolate from any global ~/.claude/shire.toml and
    // explicitly enable the experimental cross-reference index (off by
    // default since it's opt-in).
    fs::write(
        root.join("shire.toml"),
        "db_path = \".shire/index.db\"\n\n[symbols]\nreferences_enabled = true\n",
    )
    .unwrap();

    fs::create_dir_all(root.join("svc")).unwrap();
    fs::write(root.join("svc/go.mod"), "module svc\n\ngo 1.22\n").unwrap();
    fs::write(
        root.join("svc/a.go"),
        "package svc\n\nfunc A() { B() }\nfunc B() {}\n",
    )
    .unwrap();

    let bin = cargo_bin();

    // First build
    let output = Command::new(&bin)
        .args(["build", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "first build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = root.join(".shire/index.db");
    let count_b_before: i64 = {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'B' AND kind = 'call'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        count_b_before, 1,
        "call-ref to B should exist after first build"
    );

    // Modify file: remove the call to B()
    fs::write(
        root.join("svc/a.go"),
        "package svc\n\nfunc A() {}\nfunc B() {}\n",
    )
    .unwrap();

    // Incremental build
    let output = Command::new(&bin)
        .args(["build", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "incremental build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let count_b_after: i64 = {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'B' AND kind = 'call'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        count_b_after, 0,
        "call-ref to B should be removed after file modification"
    );
}

#[test]
fn test_cross_reference_index_end_to_end() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    // Write shire.toml to isolate from any global ~/.claude/shire.toml and
    // explicitly enable the experimental cross-reference index.
    fs::write(
        root.join("shire.toml"),
        "db_path = \".shire/index.db\"\n\n[symbols]\nreferences_enabled = true\n",
    )
    .unwrap();

    // Create minimal Go package with call and type references
    fs::create_dir_all(root.join("svc")).unwrap();
    fs::write(root.join("svc/go.mod"), "module svc\n\ngo 1.22\n").unwrap();
    fs::write(
        root.join("svc/main.go"),
        r#"package svc

func ParseConfig(raw string) Config {
    return Config{}
}

func Handle(req string) {
    cfg := ParseConfig(req)
    Validate(cfg)
}

func Validate(c Config) {}

type Config struct{}
"#,
    )
    .unwrap();

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "shire build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db_path = root.join(".shire/index.db");
    let conn =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    // At least one call-ref to ParseConfig
    let calls_to_parse: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'ParseConfig' AND kind = 'call'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        calls_to_parse >= 1,
        "expected at least one call-ref to ParseConfig, got {calls_to_parse}"
    );

    // The call to ParseConfig is enclosed in Handle
    let enclosing: String = conn
        .query_row(
            "SELECT enclosing_symbol FROM symbol_refs \
             WHERE name = 'ParseConfig' AND kind = 'call' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        enclosing, "Handle",
        "call to ParseConfig should be inside Handle"
    );

    // At least one call-ref to Validate
    let calls_to_validate: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'Validate' AND kind = 'call'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        calls_to_validate >= 1,
        "expected call-ref to Validate, got {calls_to_validate}"
    );

    // At least one type-ref to Config (used as parameter or return type)
    let config_type_refs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'Config' AND kind = 'type'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        config_type_refs >= 1,
        "expected type-ref to Config, got {config_type_refs}"
    );
}

/// Exercises the three states the `references_enabled` flag can be in:
///  1. Disabled (default): symbol_refs stays empty, flag persisted as false.
///  2. Transition off→on without --force: unchanged files would normally
///     hash-match and skip extraction, leaving refs empty. The build must
///     detect the transition and invalidate file_hashes so refs get
///     populated for every source file.
///  3. Transition on→off: symbol_refs wiped, flag persisted as false.
///
/// This is the load-bearing safety guarantee behind the 3 MCP ref tools —
/// without it, an LLM calling `symbol_callers` on a partially-populated
/// index gets a false-negative answer and ships a broken refactor.
#[test]
fn test_references_enabled_flag_transitions() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("svc")).unwrap();
    fs::write(root.join("svc/go.mod"), "module svc\n\ngo 1.22\n").unwrap();
    fs::write(
        root.join("svc/a.go"),
        "package svc\n\nfunc A() { B() }\nfunc B() {}\n",
    )
    .unwrap();

    let bin = cargo_bin();
    let db_path = root.join(".shire/index.db");

    // State 1: default (references_enabled omitted = false)
    fs::write(root.join("shire.toml"), "db_path = \".shire/index.db\"\n").unwrap();
    let output = Command::new(&bin)
        .args(["build", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "disabled-refs build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let refs: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(refs, 0, "symbol_refs must be empty when flag is off");
        let flag: String = conn
            .query_row(
                "SELECT value FROM shire_meta WHERE key = 'references_enabled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(flag, "false", "flag must be persisted as 'false'");
    }

    // State 2: flip false→true. Don't touch source files. The build must
    // detect the transition and invalidate file_hashes so `a.go` gets
    // re-extracted (otherwise the hash matches and no refs are written).
    fs::write(
        root.join("shire.toml"),
        "db_path = \".shire/index.db\"\n\n[symbols]\nreferences_enabled = true\n",
    )
    .unwrap();
    let output = Command::new(&bin)
        .args(["build", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "enabled-refs build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let call_refs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbol_refs WHERE name = 'B' AND kind = 'call'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            call_refs, 1,
            "false→true transition must force refs to populate for unchanged files"
        );
        let flag: String = conn
            .query_row(
                "SELECT value FROM shire_meta WHERE key = 'references_enabled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(flag, "true");
    }

    // State 3: flip true→false. symbol_refs must be wiped.
    fs::write(root.join("shire.toml"), "db_path = \".shire/index.db\"\n").unwrap();
    let output = Command::new(&bin)
        .args(["build", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success(), "disable-refs build failed");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let refs: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            refs, 0,
            "symbol_refs must be wiped when flag flips to false"
        );
        let flag: String = conn
            .query_row(
                "SELECT value FROM shire_meta WHERE key = 'references_enabled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(flag, "false");
    }
}

/// Regression test for CodeRabbit finding on PR #96: when the user flips
/// `references_enabled` in `shire.toml` without touching source or git
/// state, `.git/index` mtime is unchanged, so `git_index_changed == false`
/// and the fast-path in `phase_extract_symbols` would skip
/// `phase_source_incremental` — leaving `symbol_refs` empty for every
/// unchanged package despite the `file_hashes` wipe.
///
/// This test seeds a `.git/index` file with an old mtime, persists a
/// `last_build_at` that's *newer* than that mtime (so
/// `git_index_changed_since_build` returns false), flips the refs flag,
/// and asserts refs get populated anyway.
#[test]
fn test_refs_toggle_forces_extraction_when_git_index_unchanged() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("svc")).unwrap();
    fs::write(root.join("svc/go.mod"), "module svc\n\ngo 1.22\n").unwrap();
    fs::write(
        root.join("svc/a.go"),
        "package svc\n\nfunc A() { B() }\nfunc B() {}\n",
    )
    .unwrap();

    // Create a .git/index file. Its mtime will become the baseline that
    // `git_index_changed_since_build` compares against. We set it to an
    // old time so subsequent `last_build_at` timestamps will be newer.
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/index"), "dummy").unwrap();

    let bin = cargo_bin();
    let db_path = root.join(".shire/index.db");

    // First build: refs disabled. Populates the DB, sets last_build_at.
    fs::write(root.join("shire.toml"), "db_path = \".shire/index.db\"\n").unwrap();
    let output = Command::new(&bin)
        .args(["build", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "first build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Don't touch .git/index. The default `last_build_at` written by the
    // first build is already newer than the .git/index mtime (both were
    // created seconds ago, and last_build_at is set at the end of the
    // build after .git/index was created). To make the test deterministic
    // anyway, force `last_build_at` forward by 1 hour so
    // `git_index_changed_since_build` is guaranteed to return false.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let future = (chrono::Utc::now() + chrono::Duration::hours(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        conn.execute(
            "UPDATE shire_meta SET value = ?1 WHERE key = 'last_build_at'",
            [&future],
        )
        .unwrap();
    }

    // Second build: flip to enabled. The only delta is shire.toml.
    // .git/index is untouched, so the `git_index_changed` fast-path is
    // off — the fix under test must still force re-extraction via the
    // `refs_just_enabled` branch.
    fs::write(
        root.join("shire.toml"),
        "db_path = \".shire/index.db\"\n\n[symbols]\nreferences_enabled = true\n",
    )
    .unwrap();
    let output = Command::new(&bin)
        .args(["build", "--root", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "enable-refs build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let call_refs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'B' AND kind = 'call'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        call_refs, 1,
        "refs must populate for unchanged packages even when .git/index \
         is unchanged — otherwise flipping the flag leaves a silent \
         partial index that breaks refactor-safety queries"
    );
}

/// The `references_enabled` flag in shire_meta is the MCP safety gate —
/// it MUST match the committed state of symbol_refs at every DB-commit
/// boundary. If symbol_refs is empty (wiped by --force, wiped by the
/// disabled path) but the flag still reads `true`, MCP tools will return
/// silent `[]` for refactor-safety queries.
///
/// This test exercises the `--force` path specifically, which wipes
/// symbol_refs in its own transaction at the start of the build — long
/// before the extraction transaction runs. The fix writes `false` in
/// the wipe transaction and `true` again in the extraction transaction.
#[test]
fn test_force_rebuild_keeps_refs_flag_consistent_with_refs_table() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("svc")).unwrap();
    fs::write(root.join("svc/go.mod"), "module svc\n\ngo 1.22\n").unwrap();
    fs::write(
        root.join("svc/a.go"),
        "package svc\n\nfunc A() { B() }\nfunc B() {}\n",
    )
    .unwrap();
    fs::write(
        root.join("shire.toml"),
        "db_path = \".shire/index.db\"\n\n[symbols]\nreferences_enabled = true\n",
    )
    .unwrap();

    let bin = cargo_bin();
    let db_path = root.join(".shire/index.db");

    // Initial build populates symbol_refs and sets flag=true.
    assert!(
        Command::new(&bin)
            .args(["build", "--root", root.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );

    // Force rebuild: wipes symbol_refs in an early transaction, then
    // re-populates in the extraction transaction. Both writes must
    // happen atomically with the matching flag write.
    assert!(
        Command::new(&bin)
            .args(["build", "--force", "--root", root.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let refs: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
        .unwrap();
    let flag: String = conn
        .query_row(
            "SELECT value FROM shire_meta WHERE key = 'references_enabled'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(refs > 0, "symbol_refs must be re-populated after --force");
    assert_eq!(
        flag, "true",
        "flag must match the populated state of symbol_refs"
    );

    // The invariant we care about: flag==true ⇒ symbol_refs is the
    // ground truth (was populated by a committed extraction). A failed
    // build would leave the atomic wipe committed (flag=false, refs
    // empty) — a consistent state MCP can reason about.
}

/// Regression test for CodeRabbit finding on PR #96: during a refs
/// off→on transition, deleted-file cleanup in `phase_source_incremental`
/// depends on the stored `file_hashes` map — it diffs current files
/// against those hashes to compute `deleted_files`. Wiping
/// `file_hashes` to force re-extraction would make that diff empty,
/// leaving stale symbols for files that were removed while refs were
/// off.
///
/// This test: deletes `old.go` while refs are off, flips refs on,
/// rebuilds, and asserts the deleted file's symbols are gone AND the
/// surviving file's refs populate.
#[test]
fn test_refs_transition_cleans_up_files_deleted_while_refs_off() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("svc")).unwrap();
    fs::write(root.join("svc/go.mod"), "module svc\n\ngo 1.22\n").unwrap();
    fs::write(
        root.join("svc/a.go"),
        "package svc\n\nfunc Alive() { Callee() }\nfunc Callee() {}\n",
    )
    .unwrap();
    fs::write(root.join("svc/old.go"), "package svc\n\nfunc Doomed() {}\n").unwrap();
    fs::write(root.join("shire.toml"), "db_path = \".shire/index.db\"\n").unwrap();

    let bin = cargo_bin();
    let db_path = root.join(".shire/index.db");

    // Build 1: refs disabled. Both Doomed and Alive get indexed into
    // symbols (no refs extraction), file_hashes captures both files.
    assert!(
        Command::new(&bin)
            .args(["build", "--root", root.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let doomed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE name = 'Doomed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(doomed > 0, "Doomed should be indexed after first build");
    }

    // Delete old.go while refs are still off.
    fs::remove_file(root.join("svc/old.go")).unwrap();

    // Build 2: flip refs on. The transition triggers
    // force_source_reextract, which must still let the deleted-file
    // diff compute correctly against file_hashes.
    fs::write(
        root.join("shire.toml"),
        "db_path = \".shire/index.db\"\n\n[symbols]\nreferences_enabled = true\n",
    )
    .unwrap();
    assert!(
        Command::new(&bin)
            .args(["build", "--root", root.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let doomed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'Doomed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        doomed, 0,
        "Doomed must be removed — old.go was deleted before this build"
    );

    // Alive should still be indexed and its call-ref to Callee should
    // now be populated (proves the refs transition actually took effect).
    let alive: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'Alive'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(alive, 1, "Alive should survive the rebuild");
    let call_refs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'Callee' AND kind = 'call'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        call_refs, 1,
        "call-ref to Callee must populate — the point of the refs transition"
    );
}
