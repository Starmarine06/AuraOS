#!/usr/bin/env bash
# AuraOS Build Orchestration Script (Debian & Rust Init Base)
# This script builds the workspace and packages everything using Docker.

set -e
REPO_DIR="$PWD"

echo "============================================="
# AuraOS Build System
echo "        Starting AuraOS Build System         "
echo "============================================="

# 1. Verify Docker is available
if ! command -v docker &> /dev/null; then
    echo "ERROR: Docker is not installed or not in PATH."
    echo "Please install Docker and ensure the daemon is running."
    exit 1
fi

if ! docker ps &> /dev/null; then
    echo "ERROR: Docker daemon is not running."
    echo "Please start Docker Desktop and try again."
    exit 1
fi

# 2. Setup build directory
mkdir -p "$REPO_DIR"/out

# 3. Compile/Build ISO using Docker
echo "--> Building Docker image 'auraos-builder'..."
docker build -t auraos-builder -f "$REPO_DIR"/Dockerfile "$REPO_DIR"

echo "--> Generating bootable ISO 'out/auraos.iso'..."
# Mount host's out/ directory to container's /out/ directory to retrieve the built ISO
docker run --rm -v "$REPO_DIR"/out:/out auraos-builder

echo "============================================="
echo "   Build completed! ISO saved in:            "
echo "   $REPO_DIR/out/auraos.iso                  "
echo "============================================="
