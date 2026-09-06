use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let phase = parse_arg(&args, "--phase").unwrap_or_else(|| {
        eprintln!("Usage: autoresearch --phase build|incremental|query|lifecycle|quality [--repo <path>] [--size small|medium|large|xlarge|all]");
        std::process::exit(1);
    });

    let repos = if let Some(repo_path) = parse_arg(&args, "--repo") {
        vec![PathBuf::from(repo_path)]
    } else {
        let size_filter = parse_arg(&args, "--size").unwrap_or_else(|| "all".to_string());
        find_repos(&size_filter)
    };

    if repos.is_empty() {
        eprintln!("error: no repos found — run scripts/setup-bench-repo.sh first");
        std::process::exit(1);
    }

    match phase.as_str() {
        "build" => run_build_benchmark(&repos),
        "incremental" => run_incremental_benchmark(&repos),
        "query" => run_query_benchmark(&repos),
        "lifecycle" => run_lifecycle_benchmark(&repos),
        "quality" => run_quality_checks(&repos),
        other => {
            eprintln!(
                "error: unknown phase '{}', expected build|incremental|query|lifecycle|quality",
                other
            );
            std::process::exit(1);
        }
    }
}

fn parse_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Repo size categories based on directory name.
fn repo_size(name: &str) -> &'static str {
    match name {
        "turborepo" => "small",
        "grafana" => "medium",
        "kubernetes" => "large",
        "rust" => "xlarge",
        _ => "unknown",
    }
}

fn find_repos(size_filter: &str) -> Vec<PathBuf> {
    let cache_dir = dirs_fallback().join(".cache").join("shire-bench");
    let mut repos = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join(".git").exists() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if size_filter == "all" || repo_size(name) == size_filter {
                    repos.push(path);
                }
            }
        }
    }
    // Sort by size category: small, medium, large, xlarge
    repos.sort_by_key(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        match repo_size(name) {
            "small" => 0,
            "medium" => 1,
            "large" => 2,
            "xlarge" => 3,
            _ => 4,
        }
    });
    repos
}

fn dirs_fallback() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn repo_display_name(repo_dir: &Path) -> &str {
    repo_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
}

/// Whether the worktree is free of uncommitted changes to *tracked* files.
///
/// Untracked paths are deliberately ignored (`--untracked-files=no`). The
/// benchmark drops its own artifacts into the target repo (`.shire/bench.db`
/// from the build/incremental/query phases, `.shire-bench-tmp-*.go` during
/// lifecycle) and `scripts/setup-bench-repo.sh` writes an untracked
/// `shire.toml` into every bench repo — counting those as "dirty" would make
/// the mutating phases skip every repo they are meant to run against.
/// Tracked modifications are the only thing the benchmark can destroy, and
/// they are what this guard protects.
///
/// Returns `Err` when the status cannot be determined at all (git missing, not
/// a repository, git exiting non-zero) so callers can fail closed instead of
/// treating "unknown" as "clean".
fn worktree_is_clean(repo_dir: &Path) -> Result<bool, String> {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .current_dir(repo_dir)
        .output()
        .map_err(|e| format!("could not run `git status` in {}: {e}", repo_dir.display()))?;
    if !out.status.success() {
        return Err(format!(
            "`git status` failed in {} ({}): {}",
            repo_dir.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

/// Pre-flight guard for every phase that rewrites files in the target repo.
///
/// These phases edit tracked source files in place; running them against a
/// dirty worktree risks destroying uncommitted work. The guard fails closed —
/// if the worktree state cannot be established, the repo is skipped rather
/// than mutated.
fn guard_clean_worktree(repo_dir: &Path, phase: &str) -> bool {
    let repo_name = repo_display_name(repo_dir);
    match worktree_is_clean(repo_dir) {
        Ok(true) => true,
        Ok(false) => {
            eprintln!("[{phase}] skipping {repo_name} — worktree has uncommitted changes");
            false
        }
        Err(e) => {
            eprintln!("[{phase}] skipping {repo_name} — cannot verify worktree is clean: {e}");
            false
        }
    }
}

/// Append `line` to `path`, remembering the file's original bytes the first
/// time it is touched so the benchmark can restore exactly what it wrote —
/// never `git checkout .`, which would also discard the user's own edits.
///
/// Reads and writes raw bytes so a source file that is not valid UTF-8 still
/// round-trips unchanged, and leaves an unreadable file completely alone:
/// writing a file whose original contents we failed to capture would destroy
/// it, because "restore" would then write back an empty file.
fn append_bench_line(path: &Path, line: &str, originals: &mut HashMap<PathBuf, Vec<u8>>) {
    let content = match std::fs::read(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "warning: not modifying {} — could not read its original contents: {e}",
                path.display()
            );
            return;
        }
    };
    originals
        .entry(path.to_path_buf())
        .or_insert_with(|| content.clone());
    let mut next = content;
    next.extend_from_slice(line.as_bytes());
    let _ = std::fs::write(path, next);
}

/// Restore the files this run modified from the contents captured before the
/// first write. Only files the benchmark itself touched are rewritten.
fn restore_originals(originals: &HashMap<PathBuf, Vec<u8>>) {
    for (path, original) in originals {
        if let Err(e) = std::fs::write(path, original) {
            let display = path.display();
            eprintln!(
                "warning: failed to restore {display} — recover it with \
                 `git checkout -- {display}`: {e}"
            );
        }
    }
}

/// Remove the benchmark database (and its WAL sidecars) from the target repo,
/// plus the `.shire` directory if the benchmark was what created it.
fn cleanup_bench_db(db_path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut p = db_path.as_os_str().to_os_string();
        p.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
    if let Some(parent) = db_path.parent() {
        // Fails when the directory is not empty, which is exactly what we want.
        let _ = std::fs::remove_dir(parent);
    }
}

fn run_build_benchmark(repos: &[PathBuf]) {
    let mut all_results = Vec::new();

    for repo_dir in repos {
        let repo_name = repo_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let size = repo_size(repo_name);
        let db_path = repo_dir.join(".shire").join("bench.db");
        let config = shire::config::load_config(repo_dir).unwrap_or_default();

        const TOTAL_ITERATIONS: usize = 5;
        const WARMUP: usize = 1;

        eprintln!("\n=== {} ({}) ===", repo_name, size);

        let mut durations_ms: Vec<f64> = Vec::with_capacity(TOTAL_ITERATIONS);

        for i in 0..TOTAL_ITERATIONS {
            let _ = std::fs::remove_file(&db_path);

            eprintln!(
                "[build] iteration {}/{} {}...",
                i + 1,
                TOTAL_ITERATIONS,
                if i < WARMUP { "(warmup)" } else { "" }
            );

            let start = Instant::now();
            if let Err(e) = shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path))
            {
                eprintln!(
                    "error: build_index failed on {} iteration {}: {}",
                    repo_name,
                    i + 1,
                    e
                );
                std::process::exit(1);
            }
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            durations_ms.push(elapsed_ms);

            eprintln!(
                "[build] iteration {} completed in {:.1} ms",
                i + 1,
                elapsed_ms
            );
        }

        let measured: Vec<f64> = durations_ms[WARMUP..].to_vec();

        let conn = shire::db::open_readonly(&db_path).expect("failed to open DB readonly");
        let package_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM packages", [], |r| r.get(0))
            .unwrap_or(0);
        let symbol_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap_or(0);
        let reference_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap_or(0);
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap_or(0);
        drop(conn);

        let db_size_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

        let stats = compute_stats(&measured);

        all_results.push(serde_json::json!({
            "repo": repo_name,
            "size": size,
            "phase": "build",
            "iterations": measured.len(),
            "median_ms": round1(stats.median),
            "p95_ms": round1(stats.p95),
            "min_ms": round1(stats.min),
            "stddev_ms": round1(stats.stddev),
            "package_count": package_count,
            "symbol_count": symbol_count,
            "reference_count": reference_count,
            "file_count": file_count,
            "db_size_bytes": db_size_bytes,
        }));
    }

    let output = serde_json::json!({
        "phase": "build",
        "repos": all_results,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

/// Measure incremental rebuild performance: full build once, then
/// repeated no-change rebuilds (force=false, DB kept between iterations).
fn run_incremental_benchmark(repos: &[PathBuf]) {
    let mut all_results = Vec::new();

    for repo_dir in repos {
        let repo_name = repo_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let size = repo_size(repo_name);
        let db_path = repo_dir.join(".shire").join("bench.db");
        let config = shire::config::load_config(repo_dir).unwrap_or_default();

        const TOTAL_ITERATIONS: usize = 6;
        const WARMUP: usize = 1;

        eprintln!("\n=== {} ({}) — incremental ===", repo_name, size);

        // Initial full build (creates the DB from scratch)
        let _ = std::fs::remove_file(&db_path);
        eprintln!("[incremental] initial full build...");
        let start = Instant::now();
        if let Err(e) = shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path)) {
            eprintln!("error: initial build failed for {}: {}", repo_name, e);
            std::process::exit(1);
        }
        let full_build_ms = start.elapsed().as_secs_f64() * 1000.0;
        eprintln!("[incremental] full build: {:.1} ms", full_build_ms);

        // Incremental rebuilds (no changes, force=false, DB kept)
        let mut durations_ms: Vec<f64> = Vec::with_capacity(TOTAL_ITERATIONS);

        for i in 0..TOTAL_ITERATIONS {
            eprintln!(
                "[incremental] iteration {}/{} {}...",
                i + 1,
                TOTAL_ITERATIONS,
                if i < WARMUP { "(warmup)" } else { "" }
            );

            let start = Instant::now();
            if let Err(e) =
                shire::index::build_index_quiet(repo_dir, &config, false, Some(&db_path))
            {
                eprintln!(
                    "error: incremental build failed on {} iteration {}: {}",
                    repo_name,
                    i + 1,
                    e
                );
                std::process::exit(1);
            }
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            durations_ms.push(elapsed_ms);

            eprintln!(
                "[incremental] iteration {} completed in {:.1} ms",
                i + 1,
                elapsed_ms
            );
        }

        let measured: Vec<f64> = durations_ms[WARMUP..].to_vec();

        let conn = shire::db::open_readonly(&db_path).expect("failed to open DB readonly");
        let package_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM packages", [], |r| r.get(0))
            .unwrap_or(0);
        let symbol_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap_or(0);
        let reference_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap_or(0);
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap_or(0);
        drop(conn);

        let db_size_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        let stats = compute_stats(&measured);

        all_results.push(serde_json::json!({
            "repo": repo_name,
            "size": size,
            "phase": "incremental",
            "full_build_ms": round1(full_build_ms),
            "iterations": measured.len(),
            "median_ms": round1(stats.median),
            "p95_ms": round1(stats.p95),
            "min_ms": round1(stats.min),
            "stddev_ms": round1(stats.stddev),
            "package_count": package_count,
            "symbol_count": symbol_count,
            "reference_count": reference_count,
            "file_count": file_count,
            "db_size_bytes": db_size_bytes,
        }));
    }

    let output = serde_json::json!({
        "phase": "incremental",
        "repos": all_results,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

fn run_query_benchmark(repos: &[PathBuf]) {
    let mut all_results = Vec::new();

    for repo_dir in repos {
        let repo_name = repo_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let size = repo_size(repo_name);
        let db_path = repo_dir.join(".shire").join("bench.db");

        if !db_path.exists() {
            eprintln!(
                "[query] {} DB not found, building index first...",
                repo_name
            );
            let config = shire::config::load_config(repo_dir).unwrap_or_default();
            if let Err(e) = shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path))
            {
                eprintln!("error: build_index failed for {}: {}", repo_name, e);
                std::process::exit(1);
            }
        }

        let conn = shire::db::open_readonly(&db_path).expect("failed to open DB readonly");

        const ITERATIONS: usize = 100;

        eprintln!("\n=== {} ({}) ===", repo_name, size);

        // Sample actual names from symbol_refs so benchmarks measure real
        // index hits, not empty-table lookups against a hard-coded probe.
        // Falls back to "Config" / "main" when the index is empty (refs
        // disabled or repo with no refs yet).
        let ref_name: String = conn
            .query_row(
                "SELECT name FROM symbol_refs WHERE kind = 'type' ORDER BY RANDOM() LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "Config".into());
        let caller_target: String = conn
            .query_row(
                "SELECT name FROM symbol_refs WHERE kind = 'call' ORDER BY RANDOM() LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "New".into());
        let callee_enclosing: String = conn
            .query_row(
                "SELECT enclosing_symbol FROM symbol_refs WHERE kind = 'call' AND enclosing_symbol IS NOT NULL ORDER BY RANDOM() LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "main".into());
        // Leak sampled names into 'static so they can live inside Fn closures.
        let ref_name_static: &'static str = Box::leak(ref_name.into_boxed_str());
        let caller_target_static: &'static str = Box::leak(caller_target.into_boxed_str());
        let callee_enclosing_static: &'static str = Box::leak(callee_enclosing.into_boxed_str());

        struct QueryBench {
            name: String,
            run: Box<dyn Fn(&rusqlite::Connection)>,
        }

        let mut queries: Vec<QueryBench> = vec![
            QueryBench {
                name: "search_symbols(\"parse\")".into(),
                run: Box::new(|c| {
                    let _ = shire::db::queries::search_symbols(c, "parse", None, None, 50);
                }),
            },
            QueryBench {
                name: "search_symbols(\"Config\")".into(),
                run: Box::new(|c| {
                    let _ = shire::db::queries::search_symbols(c, "Config", None, None, 50);
                }),
            },
            QueryBench {
                name: "search_files(\"mod\")".into(),
                run: Box::new(|c| {
                    let _ = shire::db::queries::search_files(c, "mod", None, None);
                }),
            },
            QueryBench {
                name: "search_files(\"test\")".into(),
                run: Box::new(|c| {
                    let _ = shire::db::queries::search_files(c, "test", None, None);
                }),
            },
            QueryBench {
                name: "list_packages(None)".into(),
                run: Box::new(|c| {
                    let _ = shire::db::queries::list_packages(c, None);
                }),
            },
        ];

        // Only run reference-query benchmarks if the index has refs. When the
        // user has disabled references_enabled these tables are empty and the
        // benchmark numbers would all be "DB lookup on empty table", which
        // isn't useful and pollutes the aggregate.
        let ref_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap_or(0);
        if ref_count > 0 {
            queries.push(QueryBench {
                name: format!("query_symbol_references({:?})", ref_name_static),
                run: Box::new(|c| {
                    let _ = shire::db::queries::query_symbol_references(
                        c,
                        ref_name_static,
                        None,
                        None,
                        50,
                    );
                }),
            });
            queries.push(QueryBench {
                name: format!("query_symbol_references({:?}, kind=type)", ref_name_static),
                run: Box::new(|c| {
                    let _ = shire::db::queries::query_symbol_references(
                        c,
                        ref_name_static,
                        Some("type"),
                        None,
                        50,
                    );
                }),
            });
            queries.push(QueryBench {
                name: format!("query_symbol_callers({:?})", caller_target_static),
                run: Box::new(|c| {
                    let _ =
                        shire::db::queries::query_symbol_callers(c, caller_target_static, None, 50);
                }),
            });
            queries.push(QueryBench {
                name: format!("query_symbol_callees({:?})", callee_enclosing_static),
                run: Box::new(|c| {
                    let _ = shire::db::queries::query_symbol_callees(
                        c,
                        callee_enclosing_static,
                        None,
                        50,
                    );
                }),
            });
        } else {
            eprintln!("[query] symbol_refs empty — skipping reference-query benchmarks");
        }

        let mut query_results = Vec::new();

        for query in &queries {
            eprintln!(
                "[query] benchmarking {} ({} iterations)...",
                query.name, ITERATIONS
            );
            let mut durations_ms = Vec::with_capacity(ITERATIONS);

            for _ in 0..ITERATIONS {
                let start = Instant::now();
                (query.run)(&conn);
                durations_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            }

            let stats = compute_stats(&durations_ms);
            eprintln!(
                "[query] {} — median: {:.3} ms, p95: {:.3} ms, min: {:.3} ms",
                query.name, stats.median, stats.p95, stats.min
            );

            query_results.push(serde_json::json!({
                "name": query.name,
                "iterations": ITERATIONS,
                "median_ms": round3(stats.median),
                "p95_ms": round3(stats.p95),
                "min_ms": round3(stats.min),
            }));
        }

        all_results.push(serde_json::json!({
            "repo": repo_name,
            "size": size,
            "queries": query_results,
        }));
    }

    let output = serde_json::json!({
        "phase": "query",
        "repos": all_results,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

/// Simulate real MCP server lifecycle: initial build, then repeated
/// file modifications → incremental rebuild → query cycles.
/// Reports per-cycle timings to detect degradation over time.
fn run_lifecycle_benchmark(repos: &[PathBuf]) {
    use std::fs;

    const CYCLES: usize = 50;

    let mut all_results = Vec::new();

    for repo_dir in repos {
        let repo_name = repo_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let size = repo_size(repo_name);
        let db_path = repo_dir.join(".shire").join("bench.db");
        let config = shire::config::load_config(repo_dir).unwrap_or_default();

        eprintln!("\n=== {} ({}) — lifecycle ===", repo_name, size);

        if !guard_clean_worktree(repo_dir, "lifecycle") {
            continue;
        }

        // Collect source files we can modify, using the same walker the
        // indexer uses so we don't pick vendored/generated files
        let exts = shire::symbols::walker::all_extensions();
        let source_files: Vec<PathBuf> = shire::symbols::walker::walk_source_files(repo_dir, &exts)
            .unwrap_or_default()
            .into_iter()
            .take(200)
            .collect();

        if source_files.is_empty() {
            eprintln!("[lifecycle] no source files found, skipping");
            continue;
        }

        // Phase 1: Fresh full build
        let _ = fs::remove_file(&db_path);
        eprintln!("[lifecycle] initial full build...");
        let start = Instant::now();
        if let Err(e) = shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path)) {
            eprintln!("error: initial build failed: {}", e);
            std::process::exit(1);
        }
        let initial_build_ms = start.elapsed().as_secs_f64() * 1000.0;
        eprintln!("[lifecycle] initial build: {:.1} ms", initial_build_ms);

        // Get initial DB size
        let initial_db_size = fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

        // Phase 2: Incremental rebuild + query cycles
        let mut rebuild_times = Vec::with_capacity(CYCLES);
        let mut query_times = Vec::with_capacity(CYCLES);
        let mut db_sizes = Vec::with_capacity(CYCLES);
        let mut modifications: Vec<(String, String)> = Vec::new(); // (path, action)
        // Original contents of every file this run rewrites, captured before
        // the first write so the repo can be restored without `git checkout .`.
        let mut originals: HashMap<PathBuf, Vec<u8>> = HashMap::new();

        for cycle in 0..CYCLES {
            // Pick a source file to modify (round-robin)
            let target = &source_files[cycle % source_files.len()];

            // Simulate different real-world scenarios in a pattern:
            //   0-2: modify existing files (most common)
            //   3:   no-op (MCP call with no file changes)
            //   4:   create new file
            //   5-7: modify existing files
            //   8:   delete a previously created file
            //   9:   no-op
            let action = match cycle % 10 {
                3 | 9 => {
                    // No-op: don't touch any files, just rebuild
                    "no-op"
                }
                4 => {
                    // Create a new file
                    let new_file = repo_dir.join(format!(".shire-bench-tmp-{}.go", cycle));
                    let _ = fs::write(
                        &new_file,
                        format!("package bench\n\nfunc BenchFunc{}() {{}}\n", cycle),
                    );
                    modifications.push((new_file.to_string_lossy().to_string(), "create".into()));
                    "create"
                }
                8 => {
                    // Delete a previously created file (if any exist)
                    let deleted = modifications
                        .iter()
                        .rev()
                        .find(|(_, a)| a == "create")
                        .map(|(p, _)| p.clone());
                    if let Some(path) = deleted {
                        let _ = fs::remove_file(&path);
                        "delete"
                    } else {
                        // No file to delete, modify instead
                        append_bench_line(
                            target,
                            &format!("\n// bench cycle {}\n", cycle),
                            &mut originals,
                        );
                        modifications.push((target.to_string_lossy().to_string(), "modify".into()));
                        "modify"
                    }
                }
                _ => {
                    // Normal cycle: modify existing file
                    append_bench_line(
                        target,
                        &format!("\n// bench cycle {}\n", cycle),
                        &mut originals,
                    );
                    modifications.push((target.to_string_lossy().to_string(), "modify".into()));
                    "modify"
                }
            };

            // Incremental rebuild (simulates maybe_rebuild)
            let start = Instant::now();
            if let Err(e) =
                shire::index::build_index_quiet(repo_dir, &config, false, Some(&db_path))
            {
                eprintln!("error: rebuild failed on cycle {}: {}", cycle + 1, e);
                std::process::exit(1);
            }
            let rebuild_ms = start.elapsed().as_secs_f64() * 1000.0;
            rebuild_times.push(rebuild_ms);

            // Run a query (simulates MCP tool call after rebuild)
            let conn = shire::db::open_readonly(&db_path).expect("failed to open DB readonly");
            let start = Instant::now();
            let _ = shire::db::queries::search_symbols(&conn, "Config", None, None, 50);
            let _ = shire::db::queries::search_files(&conn, "test", None, None);
            let _ = shire::db::queries::search_packages(&conn, "api", 20);
            // Only benchmark ref-queries when the index actually has refs —
            // otherwise we're just measuring an empty-table lookup, which
            // pollutes the aggregate.
            if config.symbols.references_enabled {
                let _ =
                    shire::db::queries::query_symbol_references(&conn, "Config", None, None, 50);
                let _ = shire::db::queries::query_symbol_callers(&conn, "New", None, 50);
            }
            let query_ms = start.elapsed().as_secs_f64() * 1000.0;
            query_times.push(query_ms);
            drop(conn);

            let db_size = fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
            db_sizes.push(db_size);

            if (cycle + 1) % 10 == 0 || cycle == 0 {
                eprintln!(
                    "[lifecycle] cycle {:>2}: {} {:<6} rebuild={:.1}ms query={:.3}ms db={:.1}MB",
                    cycle + 1,
                    action,
                    "",
                    rebuild_ms,
                    query_ms,
                    db_size as f64 / 1_048_576.0,
                );
            }
        }

        // Clean up temporary files
        for (path, action) in &modifications {
            if action == "create" {
                let _ = fs::remove_file(path);
            }
        }

        // Restore only the files this run rewrote. `git checkout .` would also
        // discard unstaged edits the user made outside the benchmark.
        restore_originals(&originals);
        cleanup_bench_db(&db_path);

        // Analyze degradation: compare first 10 cycles vs last 10 cycles
        let first_10_rebuild: Vec<f64> = rebuild_times[..10].to_vec();
        let last_10_rebuild: Vec<f64> = rebuild_times[CYCLES - 10..].to_vec();
        let first_10_query: Vec<f64> = query_times[..10].to_vec();
        let last_10_query: Vec<f64> = query_times[CYCLES - 10..].to_vec();

        let rebuild_first = compute_stats(&first_10_rebuild);
        let rebuild_last = compute_stats(&last_10_rebuild);
        let query_first = compute_stats(&first_10_query);
        let query_last = compute_stats(&last_10_query);

        let rebuild_degradation =
            (rebuild_last.median - rebuild_first.median) / rebuild_first.median * 100.0;
        let query_degradation =
            (query_last.median - query_first.median) / query_first.median * 100.0;

        eprintln!("\n[lifecycle] {} summary:", repo_name);
        eprintln!(
            "  rebuild: first10={:.1}ms last10={:.1}ms degradation={:+.1}%",
            rebuild_first.median, rebuild_last.median, rebuild_degradation
        );
        eprintln!(
            "  query:   first10={:.3}ms last10={:.3}ms degradation={:+.1}%",
            query_first.median, query_last.median, query_degradation
        );
        eprintln!(
            "  db_size: initial={:.1}MB final={:.1}MB growth={:+.1}%",
            initial_db_size as f64 / 1_048_576.0,
            *db_sizes.last().unwrap_or(&0) as f64 / 1_048_576.0,
            (*db_sizes.last().unwrap_or(&0) as f64 - initial_db_size as f64)
                / initial_db_size as f64
                * 100.0,
        );

        all_results.push(serde_json::json!({
            "repo": repo_name,
            "size": size,
            "cycles": CYCLES,
            "initial_build_ms": round1(initial_build_ms),
            "initial_db_size_bytes": initial_db_size,
            "final_db_size_bytes": db_sizes.last().copied().unwrap_or(0),
            "rebuild": {
                "first_10_median_ms": round1(rebuild_first.median),
                "last_10_median_ms": round1(rebuild_last.median),
                "degradation_pct": round1(rebuild_degradation),
                "all_medians_ms": rebuild_times.iter().map(|v| round1(*v)).collect::<Vec<f64>>(),
            },
            "query": {
                "first_10_median_ms": round3(query_first.median),
                "last_10_median_ms": round3(query_last.median),
                "degradation_pct": round1(query_degradation),
            },
        }));
    }

    let output = serde_json::json!({
        "phase": "lifecycle",
        "repos": all_results,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

/// Verify result quality: symbol counts, search correctness, FTS integrity,
/// deterministic builds, and no data loss from generated file skipping.
fn run_quality_checks(repos: &[PathBuf]) {
    use std::fs;

    let mut all_results = Vec::new();
    let mut total_pass = 0usize;
    let mut total_fail = 0usize;

    for repo_dir in repos {
        let repo_name = repo_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let size = repo_size(repo_name);
        let db_path = repo_dir.join(".shire").join("bench.db");
        let config = shire::config::load_config(repo_dir).unwrap_or_default();

        eprintln!("\n=== {} ({}) — quality checks ===", repo_name, size);

        if !guard_clean_worktree(repo_dir, "quality") {
            continue;
        }

        // Build fresh index
        let _ = fs::remove_file(&db_path);
        if let Err(e) = shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path)) {
            eprintln!("FAIL: initial build failed: {}", e);
            total_fail += 1;
            continue;
        }

        let conn = shire::db::open_readonly(&db_path).expect("failed to open DB");

        let mut checks: Vec<(&str, bool, String)> = Vec::new();

        // 1. Symbol count > 0
        let sym_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap_or(0);
        checks.push((
            "symbols exist",
            sym_count > 0,
            format!("{} symbols", sym_count),
        ));

        // 2. Package count > 0
        let pkg_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM packages", [], |r| r.get(0))
            .unwrap_or(0);
        checks.push((
            "packages exist",
            pkg_count > 0,
            format!("{} packages", pkg_count),
        ));

        // 3. File count > 0
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap_or(0);
        checks.push((
            "files exist",
            file_count > 0,
            format!("{} files", file_count),
        ));

        // 4. Every symbol has a valid package reference
        let orphan_syms: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbols s WHERE NOT EXISTS (SELECT 1 FROM packages p WHERE p.name = s.package)",
                [],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        checks.push((
            "no orphan symbols",
            orphan_syms == 0,
            format!("{} orphans", orphan_syms),
        ));

        // 4b-4e: cross-reference index checks. Only run when the user has
        // opted into refs — otherwise the table is expected empty and
        // asserting "references exist" would be a false-positive failure.
        if config.symbols.references_enabled {
            let ref_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
                .unwrap_or(0);
            checks.push((
                "references exist",
                ref_count > 0,
                format!("{} refs", ref_count),
            ));

            // 4c. Non-null ref packages all resolve to packages table
            let orphan_refs: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM symbol_refs r WHERE r.package IS NOT NULL AND NOT EXISTS (SELECT 1 FROM packages p WHERE p.name = r.package)",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(-1);
            checks.push((
                "no orphan references",
                orphan_refs == 0,
                format!("{} orphans", orphan_refs),
            ));

            // 4d. No self-reference at a definition's own line — covers type/impl
            // self-ref bugs we recently hit for TS non-exported interfaces. The
            // JOIN key is (name, file_path, line, kind) and we exclude Call kind
            // because a method named `foo` calling itself at its own line is
            // structurally possible (e.g. recursive one-liners).
            let self_refs: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM symbol_refs r \
                     JOIN files f ON f.id = r.file_id \
                     JOIN symbols s ON s.name = r.name AND s.file_path = f.path AND s.line = r.line \
                     WHERE r.kind IN ('type', 'impl')",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(-1);
            checks.push((
                "no self-references at definition",
                self_refs == 0,
                format!("{} self-refs", self_refs),
            ));

            // 4e. Reference query round-trips for an actual name in the index.
            // Sample a random ref name instead of hard-coding "Config" — some
            // repos don't have that symbol and we'd false-fail.
            if ref_count > 0 {
                let probe: String = conn
                    .query_row(
                        "SELECT name FROM symbol_refs ORDER BY RANDOM() LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
                let probe_hits =
                    shire::db::queries::query_symbol_references(&conn, &probe, None, None, 10)
                        .map(|r| r.len())
                        .unwrap_or(0);
                checks.push((
                    "reference query returns results",
                    probe_hits > 0,
                    format!("{} refs for {:?}", probe_hits, probe),
                ));
            }
        }

        // 5. FTS integrity (needs read-write connection)
        drop(conn);
        let rw_conn = rusqlite::Connection::open(&db_path).expect("failed to open DB read-write");
        let fts_ok = rw_conn
            .execute_batch("INSERT INTO symbols_fts(symbols_fts) VALUES('integrity-check')")
            .is_ok();
        checks.push(("symbols_fts integrity", fts_ok, String::new()));

        let fts_files_ok = rw_conn
            .execute_batch("INSERT INTO files_fts(files_fts) VALUES('integrity-check')")
            .is_ok();
        checks.push(("files_fts integrity", fts_files_ok, String::new()));
        drop(rw_conn);
        let conn = shire::db::open_readonly(&db_path).expect("failed to reopen DB");

        // 6. FTS returns results for common terms
        let fts_sym_results = shire::db::queries::search_symbols(&conn, "new", None, None, 10)
            .map(|r| r.len())
            .unwrap_or(0);
        checks.push((
            "symbol FTS returns results",
            fts_sym_results > 0,
            format!("{} results for 'new'", fts_sym_results),
        ));

        let fts_file_results = shire::db::queries::search_files(&conn, "src", None, None)
            .map(|r| r.len())
            .unwrap_or(0);
        checks.push((
            "file FTS returns results",
            fts_file_results > 0,
            format!("{} results for 'src'", fts_file_results),
        ));

        // 7. Kind-filtered search works (try common kinds)
        let kind_results: usize = ["function", "method", "type", "struct", "class", "interface"]
            .iter()
            .map(|kind| {
                shire::db::queries::search_symbols(&conn, "new", None, Some(kind), 10)
                    .map(|r| r.len())
                    .unwrap_or(0)
            })
            .sum();
        checks.push((
            "kind-filtered search works",
            kind_results > 0,
            format!("{} results across common kinds", kind_results),
        ));

        // 8. Result relevance: top results should match the query term
        let config_results =
            shire::db::queries::search_symbols(&conn, "Config", None, None, 5).unwrap_or_default();
        // Check that query term appears somewhere in the result (name, signature, or file_path)
        let top_relevant = config_results
            .iter()
            .take(5)
            .filter(|s| {
                let lname = s.name.to_lowercase();
                let lsig = s.signature.as_deref().unwrap_or("").to_lowercase();
                let lpath = s.file_path.to_lowercase();
                lname.contains("config") || lsig.contains("config") || lpath.contains("config")
            })
            .count();
        checks.push((
            "symbol search relevance",
            top_relevant >= 3,
            format!(
                "top 5 for 'Config': {}/5 relate to 'config' (names: {})",
                top_relevant,
                config_results
                    .iter()
                    .take(5)
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));

        // 9. File search relevance: results should contain the query in path
        let file_results =
            shire::db::queries::search_files(&conn, "config", None, None).unwrap_or_default();
        let files_relevant = file_results
            .iter()
            .filter(|f| f.path.to_lowercase().contains("config"))
            .count();
        checks.push((
            "file search relevance",
            files_relevant == file_results.len(),
            format!(
                "{}/{} results contain 'config' in path",
                files_relevant,
                file_results.len()
            ),
        ));

        // 10. No generated files in results
        let generated_in_results = file_results
            .iter()
            .filter(|f| {
                let name = f.path.rsplit('/').next().unwrap_or("");
                name.ends_with("_generated.go")
                    || name.ends_with(".generated.go")
                    || name.starts_with("zz_generated.")
                    || name.ends_with(".pb.go")
            })
            .count();
        checks.push((
            "no generated files in results",
            generated_in_results == 0,
            format!("{} generated files found", generated_in_results),
        ));

        // 11. Symbol kinds are valid
        let invalid_kinds: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE kind NOT IN ('function', 'method', 'type', 'struct', 'class', 'interface', 'enum', 'constant', 'variable', 'field', 'property', 'module', 'trait', 'impl', 'protocol', 'extension', 'macro', 'typedef', 'arrow_function', 'union', 'namespace', 'package', 'rpc', 'message', 'service', 'object', 'alias', 'component', 'hook', 'test', 'sub', 'mixin', 'concern', 'scope', 'callback')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        checks.push((
            "all symbol kinds valid",
            invalid_kinds == 0,
            if invalid_kinds > 0 {
                let sample: String = conn
                    .query_row(
                        "SELECT kind FROM symbols WHERE kind NOT IN ('function', 'method', 'type', 'struct', 'class', 'interface', 'enum', 'constant', 'variable', 'field', 'property', 'module', 'trait', 'impl', 'protocol', 'extension', 'macro', 'typedef', 'arrow_function', 'union', 'namespace', 'package', 'rpc', 'message', 'service', 'object', 'alias', 'component', 'hook', 'test', 'sub', 'mixin', 'concern', 'scope', 'callback') LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
                format!("{} invalid (e.g. '{}')", invalid_kinds, sample)
            } else {
                String::new()
            },
        ));

        // 12. Deterministic: build twice, same symbol count
        drop(conn);
        let _ = fs::remove_file(&db_path);
        let _ = shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path));
        let conn2 = shire::db::open_readonly(&db_path).expect("failed to open DB");
        let sym_count_2: i64 = conn2
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap_or(0);
        // Allow small variance from parallel extraction non-determinism
        let variance = (sym_count - sym_count_2).unsigned_abs();
        let max_variance = (sym_count as f64 * 0.05) as u64; // 5% tolerance
        checks.push((
            "deterministic symbol count",
            variance <= max_variance,
            format!(
                "build1={} build2={} variance={}",
                sym_count, sym_count_2, variance
            ),
        ));

        // 9. Incremental build preserves symbols
        // Modify a file, rebuild, check symbols still exist
        let test_file: Option<PathBuf> = walkdir::WalkDir::new(repo_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_type().is_file()
                    && e.path().extension().and_then(|x| x.to_str()) == Some("go")
            })
            .map(|e| e.into_path());

        if let Some(test_file) = test_file {
            let mut originals: HashMap<PathBuf, Vec<u8>> = HashMap::new();
            append_bench_line(&test_file, "\n// quality check\n", &mut originals);
            let incr_ok =
                shire::index::build_index_quiet(repo_dir, &config, false, Some(&db_path)).is_ok();
            restore_originals(&originals);

            let conn3 = shire::db::open_readonly(&db_path).expect("failed to open DB");
            let sym_count_3: i64 = conn3
                .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
                .unwrap_or(0);
            let incr_variance = (sym_count_2 - sym_count_3).unsigned_abs();
            checks.push((
                "incremental preserves symbols",
                incr_ok && incr_variance <= max_variance,
                format!(
                    "before={} after={} variance={}",
                    sym_count_2, sym_count_3, incr_variance
                ),
            ));
            drop(conn3);
        }

        // 10. No real source files skipped by generated patterns
        // Check that common non-generated filenames are NOT skipped
        let has_main_files: bool = conn2
            .query_row(
                "SELECT COUNT(*) > 0 FROM symbols WHERE file_path LIKE '%main.go' OR file_path LIKE '%index.ts' OR file_path LIKE '%lib.rs'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        checks.push((
            "real source files not skipped",
            has_main_files || file_count < 100, // small repos may not have these
            String::new(),
        ));

        drop(conn2);

        // Report
        let mut repo_checks = Vec::new();
        for (name, passed, detail) in &checks {
            let status = if *passed { "PASS" } else { "FAIL" };
            if *passed {
                total_pass += 1;
            } else {
                total_fail += 1;
            }
            let detail_str = if detail.is_empty() {
                String::new()
            } else {
                format!(" ({})", detail)
            };
            eprintln!("  [{}] {}{}", status, name, detail_str);
            repo_checks.push(serde_json::json!({
                "name": name,
                "passed": passed,
                "detail": detail,
            }));
        }

        // The one file check 9 rewrites is restored inline right after the
        // rebuild, so there is nothing left to undo here — and `git checkout .`
        // would discard the user's own uncommitted edits along with ours.
        cleanup_bench_db(&db_path);

        all_results.push(serde_json::json!({
            "repo": repo_name,
            "size": size,
            "checks": repo_checks,
        }));
    }

    eprintln!(
        "\n=== Quality: {} passed, {} failed ===",
        total_pass, total_fail
    );

    let output = serde_json::json!({
        "phase": "quality",
        "total_pass": total_pass,
        "total_fail": total_fail,
        "repos": all_results,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());

    if total_fail > 0 {
        std::process::exit(1);
    }
}

struct Stats {
    median: f64,
    p95: f64,
    min: f64,
    stddev: f64,
}

fn compute_stats(values: &[f64]) -> Stats {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let n = sorted.len();
    let median = if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    };

    let p95_idx = ((n as f64) * 0.95).ceil() as usize;
    let p95 = sorted[p95_idx.min(n - 1)];

    let min = sorted[0];

    let mean = sorted.iter().sum::<f64>() / n as f64;
    let variance = sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
    let stddev = variance.sqrt();

    Stats {
        median,
        p95,
        min,
        stddev,
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git must be installed to run these tests");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "bench@example.invalid"]);
        git(dir, &["config", "user.name", "bench"]);
        std::fs::write(dir.join("main.go"), "package main\n").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "init"]);
    }

    #[test]
    fn test_worktree_is_clean_on_committed_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        init_repo(dir.path());
        assert_eq!(worktree_is_clean(dir.path()), Ok(true));
        assert!(guard_clean_worktree(dir.path(), "test"));
    }

    #[test]
    fn test_worktree_is_dirty_with_unstaged_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("main.go"), "package main\n// precious\n").unwrap();
        assert_eq!(worktree_is_clean(dir.path()), Ok(false));
        assert!(
            !guard_clean_worktree(dir.path(), "test"),
            "a mutating phase must refuse a dirty worktree"
        );
    }

    /// Untracked files must not block a mutating phase. Every bench repo has
    /// an untracked `shire.toml` (written by scripts/setup-bench-repo.sh) and
    /// an untracked `.shire/bench.db` left by the build/incremental/query
    /// phases; treating those as "dirty" would make lifecycle and quality skip
    /// every repo. The benchmark never destroys untracked files — it restores
    /// from bytes it captured itself.
    #[test]
    fn test_untracked_files_do_not_block_the_guard() {
        let dir = tempfile::TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("shire.toml"), "db_path = \".shire/x.db\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();
        std::fs::write(dir.path().join(".shire/bench.db"), "x").unwrap();
        assert_eq!(worktree_is_clean(dir.path()), Ok(true));
        assert!(guard_clean_worktree(dir.path(), "test"));
    }

    /// A staged-but-uncommitted change is still uncommitted work.
    #[test]
    fn test_worktree_is_dirty_with_staged_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("main.go"), "package main\n// staged\n").unwrap();
        git(dir.path(), &["add", "main.go"]);
        assert_eq!(worktree_is_clean(dir.path()), Ok(false));
    }

    /// Fail closed: an unknown worktree state must skip the repo, not mutate it.
    #[test]
    fn test_guard_fails_closed_outside_a_git_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(worktree_is_clean(dir.path()).is_err());
        assert!(
            !guard_clean_worktree(dir.path(), "test"),
            "an undeterminable worktree state must be treated as dirty"
        );
    }

    #[test]
    fn test_append_and_restore_only_touches_written_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let touched = dir.path().join("touched.go");
        let untouched = dir.path().join("untouched.go");
        std::fs::write(&touched, "package a\n").unwrap();
        std::fs::write(&untouched, "package b\n").unwrap();

        let mut originals: HashMap<PathBuf, Vec<u8>> = HashMap::new();
        append_bench_line(&touched, "\n// cycle 0\n", &mut originals);
        append_bench_line(&touched, "\n// cycle 1\n", &mut originals);
        assert!(
            std::fs::read_to_string(&touched)
                .unwrap()
                .contains("cycle 1")
        );

        // A file edited outside the benchmark must survive the restore.
        std::fs::write(&untouched, "package b\n// user edit\n").unwrap();

        restore_originals(&originals);
        assert_eq!(std::fs::read_to_string(&touched).unwrap(), "package a\n");
        assert_eq!(
            std::fs::read_to_string(&untouched).unwrap(),
            "package b\n// user edit\n",
            "restore must not revert files the benchmark never wrote"
        );
    }

    /// A source file that is not valid UTF-8 must round-trip byte-for-byte.
    /// Reading it as a `String` would yield "" and the restore would then
    /// permanently truncate the file.
    #[test]
    fn test_append_and_restore_preserves_non_utf8_bytes() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("latin1.go");
        let original: Vec<u8> = b"package a // caf\xe9\n".to_vec();
        std::fs::write(&path, &original).unwrap();

        let mut originals: HashMap<PathBuf, Vec<u8>> = HashMap::new();
        append_bench_line(&path, "\n// cycle 0\n", &mut originals);
        let after_write = std::fs::read(&path).unwrap();
        assert!(
            after_write.starts_with(&original),
            "the append must not drop the file's existing bytes"
        );

        restore_originals(&originals);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    /// A file that cannot be read must be left untouched — writing it would
    /// destroy content the restore cannot reproduce.
    #[test]
    fn test_append_skips_unreadable_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist.go");

        let mut originals: HashMap<PathBuf, Vec<u8>> = HashMap::new();
        append_bench_line(&missing, "\n// cycle 0\n", &mut originals);

        assert!(
            originals.is_empty(),
            "nothing was captured, so nothing was written"
        );
        assert!(!missing.exists(), "an unreadable file must not be created");
    }

    #[test]
    fn test_cleanup_bench_db_removes_db_and_empty_shire_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let shire_dir = dir.path().join(".shire");
        std::fs::create_dir_all(&shire_dir).unwrap();
        let db = shire_dir.join("bench.db");
        std::fs::write(&db, "x").unwrap();
        std::fs::write(shire_dir.join("bench.db-wal"), "x").unwrap();

        cleanup_bench_db(&db);
        assert!(!db.exists());
        assert!(!shire_dir.exists());
    }

    #[test]
    fn test_cleanup_bench_db_keeps_non_empty_shire_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let shire_dir = dir.path().join(".shire");
        std::fs::create_dir_all(&shire_dir).unwrap();
        let db = shire_dir.join("bench.db");
        std::fs::write(&db, "x").unwrap();
        let user_db = shire_dir.join("index.db");
        std::fs::write(&user_db, "x").unwrap();

        cleanup_bench_db(&db);
        assert!(!db.exists());
        assert!(user_db.exists(), "the real index must not be removed");
    }
}
