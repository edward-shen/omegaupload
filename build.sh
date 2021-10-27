#!/usr/bin/env bash

set -euxo pipefail

# Build frontend assets
yarn
trunk build --release

# Build server
cargo build --release --bin omegaupload-server

# index.html no longer needed, served statically by the upload server
rm dist/index.html

# Prepare assets for upload to webserver
mkdir -p dist/static
# Move everything that's not index.html into a `static` subdir
find dist -type f -exec mv {} dist/static/ ";"

strip target/release/omegaupload-server
cp target/release/omegaupload-server dist/omegaupload-server