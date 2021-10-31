#!/usr/bin/env bash

source .env

cd "$(git rev-parse --show-toplevel)" || exit 1

cargo build --release --bin omegaupload-cli

TEST_PATH="test/*"

PADDING=0

for file in $TEST_PATH; do
  if [ $PADDING -lt ${#file} ]; then
    PADDING=${#file}
  fi
done

for file in $TEST_PATH; do
  printf "%$((PADDING - ${#TEST_PATH} + 1))s: " "${file#$TEST_PATH}"
  ./target/release/omegaupload-cli upload "$PASTE_URL" "$file"
done