#!/usr/bin/env bash

# OmegaUpload Upload Test Script
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

source .env

cd "$(git rev-parse --show-toplevel)" || exit 1

cargo build --release --bin omegaupload

TEST_PATH="test/*"

PADDING=0

for file in $TEST_PATH; do
  if [ $PADDING -lt ${#file} ]; then
    PADDING=${#file}
  fi
done

for file in $TEST_PATH; do
  printf "%$((PADDING - ${#TEST_PATH} + 1))s: " "${file#$TEST_PATH}"
  ./target/release/omegaupload upload "$PASTE_URL" "$file"
done