#!/usr/bin/env bash
# Install the jdw CLI binary to ~/.local/bin
set -euo pipefail

BINARY_NAME="jdw"
INSTALL_DIR="${XDG_BIN_HOME:-${HOME}/.local/bin}"

cd "$(dirname "$0")"

echo "Building ${BINARY_NAME} CLI..."
cargo build --release

mkdir -p "${INSTALL_DIR}"
cp "target/release/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"

echo "Installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}"
echo "Ensure ${INSTALL_DIR}/ is in your PATH"
