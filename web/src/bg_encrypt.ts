// OmegaUpload Web Frontend
// Copyright (C) 2021  Edward Shen
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

import { encrypt_array_buffer } from '../pkg';

interface BgData {
  location: string,
  data: any
}

addEventListener('message', (event: MessageEvent<BgData>) => {
  let { location, data } = event.data;
  console.log('[js-worker] Sending data to rust in a worker thread...');
  encrypt_array_buffer(location, data).then(url => {
    console.log("[js-worker] Encryption done.");
    postMessage(url);
  }).catch(e => console.error(e));
})

postMessage("init");