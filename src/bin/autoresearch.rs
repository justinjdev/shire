use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let phase = parse_arg(&args, "--phase").unwrap_or_else(|| {
        eprintln!("Usage: autoresearch --phase build|query [--repo <path>] [--size small|medium|large|all]");
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
        "query" => run_query_benchmark(&repos),
        other => {
            eprintln!("error: unknown phase '{}', expected 'build' or 'query'", other);
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
    // Sort by size category: small, medium, large
    repos.sort_by_key(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        match repo_size(name) {
            "small" => 0,
            "medium" => 1,
            "large" => 2,
            _ => 3,
        }
    });
    repos
}

fn dirs_fallback() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
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
            if let Err(e) =
                shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path))
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

            eprintln!("[build] iteration {} completed in {:.1} ms", i + 1, elapsed_ms);
        }

        let measured: Vec<f64> = durations_ms[WARMUP..].to_vec();

        let conn = shire::db::open_readonly(&db_path).expect("failed to open DB readonly");
        let package_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM packages", [], |r| r.get(0))
            .unwrap_or(0);
        let symbol_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap_or(0);
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap_or(0);
        drop(conn);

        let db_size_bytes = std::fs::metadata(&db_path)
            .map(|m| m.len())
            .unwrap_or(0);

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
            eprintln!("[query] {} DB not found, building index first...", repo_name);
            let config = shire::config::load_config(repo_dir).unwrap_or_default();
            if let Err(e) =
                shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path))
            {
                eprintln!("error: build_index failed for {}: {}", repo_name, e);
                std::process::exit(1);
            }
        }

        let conn = shire::db::open_readonly(&db_path).expect("failed to open DB readonly");

        const ITERATIONS: usize = 100;

        eprintln!("\n=== {} ({}) ===", repo_name, size);

        struct QueryBench {
            name: &'static str,
            run: Box<dyn Fn(&rusqlite::Connection)>,
        }

        let queries: Vec<QueryBench> = vec![
            QueryBench {
                name: "search_symbols(\"parse\")",
                run: Box::new(|c| {
                    let _ = shire::db::queries::search_symbols(c, "parse", None, None, 50);
                }),
            },
            QueryBench {
                name: "search_symbols(\"Config\")",
                run: Box::new(|c| {
                    let _ = shire::db::queries::search_symbols(c, "Config", None, None, 50);
                }),
            },
            QueryBench {
                name: "search_files(\"mod\")",
                run: Box::new(|c| {
                    let _ = shire::db::queries::search_files(c, "mod", None, None);
                }),
            },
            QueryBench {
                name: "search_files(\"test\")",
                run: Box::new(|c| {
                    let _ = shire::db::queries::search_files(c, "test", None, None);
                }),
            },
            QueryBench {
                name: "list_packages(None)",
                run: Box::new(|c| {
                    let _ = shire::db::queries::list_packages(c, None);
                }),
            },
        ];

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
    let median = if n % 2 == 0 {
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
