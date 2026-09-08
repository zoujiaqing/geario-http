#!/bin/sh
# Reproducible build of the ntex 4.0 (neon) HTTP bench server.
#
# ntex is a git dependency pinned to a public commit, so no local checkout is
# needed. It does not compile under ntex's own `warnings = deny` on rustc
# 1.98; --cap-lints=allow relaxes lint enforcement only (no codegen change).
# Cross-compiling from macOS is blocked (the homebrew toolchain's glibc lacks
# `getrandom`, which ntex pulls through std), so build on the target OS. On a
# box without github egress, clone ntex at the pinned rev and repoint the
# dependency to a local path; the SHA is the reproducible anchor either way.
#
#   ./build.sh polling   # or: ./build.sh uring
set -eu
FEATURE="${1:-polling}"
cd "$(dirname "$0")"
RUSTFLAGS="--cap-lints=allow" cargo build --release -p server-ntex4 --features "$FEATURE"
echo "built: target/release/server-ntex4 (feature=$FEATURE)"
