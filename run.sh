#!/usr/bin/env bash
set -e

# Change directory to project root
cd "$(dirname "$0")"

if [ -f "./target/release/truswap" ]; then
    exec ./target/release/truswap
else
    exec cargo run --release
fi
