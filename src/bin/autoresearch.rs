use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let phase = parse_arg(&args, "--phase").unwrap_or_else(|| {
        eprintln!("Usage: autoresearch --phase build|query [--repo <path>]");
        std::process::exit(1);
    });

    let repo_dir = parse_arg(&args, "--repo")
        .map(PathBuf::from)
        .unwrap_or_else(|| find_default_repo());

    if !repo_dir.is_dir() {
        eprintln!("error: repo directory does not exist: {}", repo_dir.display());
        std::process::exit(1);
    }

    let db_path = repo_dir.join(".shire").join("bench.db");

    match phase.as_str() {
        "build" => run_build_benchmark(&repo_dir, &db_path),
        "query" => run_query_benchmark(&repo_dir, &db_path),
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

fn find_default_repo() -> PathBuf {
    let cache_dir = dirs_fallback().join(".cache").join("shire-bench");
    if cache_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join(".git").exists() {
                    return path;
                }
            }
        }
    }
    eprintln!("error: no repo found in ~/.cache/shire-bench/ — pass --repo <path>");
    std::process::exit(1);
}

fn dirs_fallback() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn run_build_benchmark(repo_dir: &Path, db_path: &Path) {
    let config = shire::config::load_config(repo_dir).unwrap_or_default();

    const TOTAL_ITERATIONS: usize = 5;
    const WARMUP: usize = 1;

    let mut durations_ms: Vec<f64> = Vec::with_capacity(TOTAL_ITERATIONS);

    for i in 0..TOTAL_ITERATIONS {
        // Delete DB to force full rebuild
        let _ = std::fs::remove_file(db_path);

        eprintln!(
            "[build] iteration {}/{} {}...",
            i + 1,
            TOTAL_ITERATIONS,
            if i < WARMUP { "(warmup)" } else { "" }
        );

        let start = Instant::now();
        if let Err(e) = shire::index::build_index_quiet(repo_dir, &config, true, Some(db_path)) {
            eprintln!("error: build_index failed on iteration {}: {}", i + 1, e);
            std::process::exit(1);
        }
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        durations_ms.push(elapsed_ms);

        eprintln!("[build] iteration {} completed in {:.1} ms", i + 1, elapsed_ms);
    }

    // Discard warmup iterations
    let measured: Vec<f64> = durations_ms[WARMUP..].to_vec();

    // Gather stats from the DB
    let conn = shire::db::open_readonly(db_path).expect("failed to open DB readonly");
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

    let db_size_bytes = std::fs::metadata(db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let stats = compute_stats(&measured);

    let output = serde_json::json!({
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
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

fn run_query_benchmark(repo_dir: &Path, db_path: &Path) {
    // Build index if DB doesn't exist
    if !db_path.exists() {
        eprintln!("[query] DB not found, building index first...");
        let config = shire::config::load_config(repo_dir).unwrap_or_default();
        if let Err(e) = shire::index::build_index_quiet(repo_dir, &config, true, Some(db_path)) {
            eprintln!("error: build_index failed: {}", e);
            std::process::exit(1);
        }
    }

    let conn = shire::db::open_readonly(db_path).expect("failed to open DB readonly");

    const ITERATIONS: usize = 100;

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
        eprintln!("[query] benchmarking {} ({} iterations)...", query.name, ITERATIONS);
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

    let output = serde_json::json!({
        "phase": "query",
        "queries": query_results,
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
