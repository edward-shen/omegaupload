#!/usr/bin/env bash

# OmegaUpload Build Script
# Copyright (C) 2021  Edward Shen
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

set -euxo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

# Build frontend assets
yarn

rm -rf dist
yarn webpack

# Build server
cargo build --release --bin omegaupload-server

# Prepare assets for upload to webserver
mkdir -p dist/static
# Move everything that's not index.html into a `static` subdir
find dist -not -name index.html -type f -exec mv {} dist/static/ ";"

strip target/release/omegaupload-server
cp target/release/omegaupload-server dist/omegaupload-server

tar -cvf dist.tar dist
rm -rf dist.tar.zst
zstd -T0 --ultra --rm -22 dist.tar