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
)

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
        # Always ensure RAG is disabled for benchmarks
        echo -e '[rag]\nenabled = false' > "${repo_dir}/shire.toml"
        return 0
    fi

    echo "[${name}] Cloning ${url}..."
    git clone --no-checkout --filter=blob:none "${url}" "${repo_dir}"
    cd "${repo_dir}"
    git checkout "${commit}" 2>/dev/null || git checkout "tags/${commit}" 2>/dev/null
    echo "[${name}] Ready at ${repo_dir}"

    # Ensure RAG is disabled for benchmarks
    echo -e '[rag]\nenabled = false' > "${repo_dir}/shire.toml"
}

# Parse optional filter: setup-bench-repo.sh [small|medium|large|all]
FILTER="${1:-all}"

for entry in "${REPOS[@]}"; do
    IFS='|' read -r name _ _ <<< "${entry}"
    case "${FILTER}" in
        all)    setup_repo "${entry}" ;;
        small)  [[ "${name}" == "turborepo" ]] && setup_repo "${entry}" ;;
        medium) [[ "${name}" == "grafana" ]] && setup_repo "${entry}" ;;
        large)  [[ "${name}" == "kubernetes" ]] && setup_repo "${entry}" ;;
        *)      echo "Usage: $0 [small|medium|large|all]"; exit 1 ;;
    esac
done

echo ""
echo "Benchmark repos ready in ${BENCH_DIR}:"
du -sh "${BENCH_DIR}"/*/ 2>/dev/null || true
