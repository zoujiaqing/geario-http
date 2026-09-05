#!/bin/sh
# Regenerates include/geario.h. Needs cbindgen: cargo install cbindgen
set -e
cd "$(dirname "$0")"
cbindgen --config cbindgen.toml --crate geario-http-ffi --output include/geario_http.h
echo "wrote include/geario_http.h"
