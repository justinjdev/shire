#!/usr/bin/env bash
set -euo pipefail

BENCH_DIR="${HOME}/.cache/shire-bench"
mkdir -p "${BENCH_DIR}"

# Each entry: name|url|commit
REPOS=(
    # Small (~900MB, 400 packages, TS/JS/Rust/Go)
    "turborepo|https://github.com/vercel/turborepo.git|c5a46903a5f396645357015568344c27314671d2"
    # Medium (~1.5GB, Go/TS mixed)
    "grafana|https://github.com/grafana/grafana.git|v11.0.0"
    # Large (~2.5GB, Go/Python/Protobuf)
    "kubernetes|https://github.com/kubernetes/kubernetes.git|v1.30.0"
    # Extra-large (Rust monorepo with many crates)
    "rust|https://github.com/rust-lang/rust.git|1.78.0"
)

# Cross-reference index on (so bench exercises the symbol_refs path even
# though refs are off by default). Shared across new-clone and re-use code
# paths to avoid config drift.
write_bench_config() {
    cat > "$1/shire.toml" <<'TOML'
[symbols]
references_enabled = true
TOML
}

setup_repo() {
    local name url commit
    IFS='|' read -r name url commit <<< "$1"
    local repo_dir="${BENCH_DIR}/${name}"

    if [ -d "${repo_dir}/.git" ]; then
        echo "[${name}] Already exists at ${repo_dir}"
        cd "${repo_dir}"
        CURRENT=$(git describe --tags --exact-match 2>/dev/null || git rev-parse HEAD)
        if [ "${CURRENT}" != "${commit}" ]; then
            echo "[${name}] Checking out ${commit}"
            git fetch origin --tags
            git checkout "${commit}" 2>/dev/null || git checkout "tags/${commit}" 2>/dev/null
        else
            echo "[${name}] Already at ${commit}"
        fi
        write_bench_config "${repo_dir}"
        return 0
    fi

    echo "[${name}] Cloning ${url}..."
    git clone --no-checkout --filter=blob:none "${url}" "${repo_dir}"
    cd "${repo_dir}"
    git checkout "${commit}" 2>/dev/null || git checkout "tags/${commit}" 2>/dev/null
    echo "[${name}] Ready at ${repo_dir}"

    write_bench_config "${repo_dir}"
}

# Parse optional filter: setup-bench-repo.sh [small|medium|large|xlarge|all]
FILTER="${1:-all}"

for entry in "${REPOS[@]}"; do
    IFS='|' read -r name _ _ <<< "${entry}"
    case "${FILTER}" in
        all)    setup_repo "${entry}" ;;
        small)  [[ "${name}" == "turborepo" ]] && setup_repo "${entry}" ;;
        medium) [[ "${name}" == "grafana" ]] && setup_repo "${entry}" ;;
        large)  [[ "${name}" == "kubernetes" ]] && setup_repo "${entry}" ;;
        xlarge) [[ "${name}" == "rust" ]] && setup_repo "${entry}" ;;
        *)      echo "Usage: $0 [small|medium|large|xlarge|all]"; exit 1 ;;
    esac
done

echo ""
echo "Benchmark repos ready in ${BENCH_DIR}:"
du -sh "${BENCH_DIR}"/*/ 2>/dev/null || true
