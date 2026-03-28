#!/usr/bin/env bash
set -euo pipefail

REPO_URL="https://github.com/vercel/turborepo.git"
COMMIT_SHA="c5a46903a5f396645357015568344c27314671d2"
REPO_NAME="turborepo"

BENCH_DIR="${HOME}/.cache/shire-bench"
REPO_DIR="${BENCH_DIR}/${REPO_NAME}"

if [ -d "${REPO_DIR}/.git" ]; then
    echo "Benchmark repo already exists at ${REPO_DIR}"
    cd "${REPO_DIR}"
    CURRENT_SHA=$(git rev-parse HEAD)
    if [ "${CURRENT_SHA}" = "${COMMIT_SHA}" ]; then
        echo "Already at pinned commit ${COMMIT_SHA}"
        exit 0
    else
        echo "Resetting to pinned commit ${COMMIT_SHA}"
        git fetch origin
        git checkout "${COMMIT_SHA}"
        exit 0
    fi
fi

echo "Cloning ${REPO_URL} to ${REPO_DIR}..."
mkdir -p "${BENCH_DIR}"
git clone --no-checkout "${REPO_URL}" "${REPO_DIR}"
cd "${REPO_DIR}"
git checkout "${COMMIT_SHA}"
echo "Benchmark repo ready at ${REPO_DIR} (commit ${COMMIT_SHA})"
