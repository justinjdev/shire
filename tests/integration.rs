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
        "Expected 5 packages, got: {stdout}"
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
