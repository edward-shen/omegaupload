#!/usr/bin/env bash

set -euxo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

# Build frontend assets
yarn
trunk build --release

sed -i 's#/index#/static/index#g' dist/index.html
sed -i 's#stylesheet" href="/main#stylesheet" href="/static/main#g' dist/index.html

# Build server
cargo build --release --bin omegaupload-server

# Prepare assets for upload to webserver
mkdir -p dist/static
# Move everything that's not index.html into a `static` subdir
find dist -type f -exec mv {} dist/static/ ";"

strip target/release/omegaupload-server
cp target/release/omegaupload-server dist/omegaupload-server

tar -cvf dist.tar dist
rm -rf dist.tar.zst
zstd -T0 --ultra --rm -22 dist.tar