#!/usr/bin/env bash

# OmegaUpload Update Web Dependencies Script
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

CUR_DIR=$(pwd)
PROJECT_TOP_LEVEL=$(git rev-parse --show-toplevel)

cd "$PROJECT_TOP_LEVEL" || exit 1
git submodule foreach git pull

cd "$CUR_DIR"
