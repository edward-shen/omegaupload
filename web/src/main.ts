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

// Exported to main.rs
function loadFromDb(mimeType: string, name?: string, language?: string) {
  console.log("[js] Got name:", name);
  console.log("[js] Got language:", language);
  console.log("[js] Got mime type:", mimeType);

  const dbReq = window.indexedDB.open("omegaupload", 1);
  dbReq.onsuccess = (evt) => {
    const db = (evt.target as IDBRequest).result;
    const obj_store = db
      .transaction("decrypted data")
      .objectStore("decrypted data");
    const fetchReq = obj_store.get(window.location.pathname);
    fetchReq.onsuccess = (evt) => {
      const data = (evt.target as IDBRequest).result;
      switch (data.type) {
        case "string":
          createStringPasteUi(data, mimeType, name, language);
          break;
        case "blob":
          createBlobPasteUi(data, name);
          break;
        case "image":
          createImagePasteUi(data, name);
          break;
        case "audio":
          createAudioPasteUi(data, name);
          break;
        case "video":
          createVideoPasteUi(data, name);
          break;
        case "archive":
          createArchivePasteUi(data, name);
          break;
        default:
          renderMessage("Something went wrong. Try clearing local data.");
          break;
      }

      // IDB was only used as a temporary medium;
      window.onbeforeunload = (e) => {
        // See https://link.eddie.sh/NrIIq on why .commit is necessary.
        const transaction = db.transaction("decrypted data", "readwrite");
        transaction
          .objectStore("decrypted data")
          .delete(window.location.pathname);
        transaction.commit();
        transaction.oncomplete = () => {
          console.log("Item deleted from cache");
        }
      };
    };

    fetchReq.onerror = (evt) => {
      console.log("err");
      console.log(evt);
    };
  };
}

function createStringPasteUi(data, mimeType: string, name?: string, lang?: string) {
  const bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  const mainEle = document.createElement("main");
  const preEle = document.createElement("pre");
  preEle.classList.add("paste");

  const headerEle = document.createElement("p");
  headerEle.classList.add("unselectable");
  headerEle.classList.add("centered");
  headerEle.textContent = data.expiration;
  preEle.appendChild(headerEle);

  const downloadEle = document.createElement("a");
  downloadEle.href = getObjectUrl([data.data], mimeType);
  downloadEle.download = name;

  downloadEle.classList.add("hljs-meta");
  downloadEle.classList.add("centered");
  downloadEle.textContent = "Download file.";
  preEle.appendChild(downloadEle);

  preEle.appendChild(document.createElement("hr"));

  const codeEle = document.createElement("code");
  codeEle.textContent = data.data;
  preEle.appendChild(codeEle);

  mainEle.appendChild(preEle);
  bodyEle.appendChild(mainEle);

  if (!hljs.getLanguage(lang)) {
    console.warn(`[js] User provided language (${lang}) is not known. Ignoring.`);
  } else {
    console.log(`[js] Selecting user provided language ${lang} for highlighting.`);
    hljs.configure({
      languages: [lang],
    });
  }

  hljs.highlightAll();
  hljs.initLineNumbersOnLoad();
}

function createBlobPasteUi(data, name?: string) {
  const bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  const mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");

  const divEle = document.createElement("div");
  divEle.classList.add("centered");

  const expirationEle = document.createElement("p");
  expirationEle.textContent = data.expiration;
  divEle.appendChild(expirationEle);

  const downloadEle = document.createElement("a");
  downloadEle.href = getObjectUrl(data.data, name);
  downloadEle.download = name;
  downloadEle.classList.add("hljs-meta");
  downloadEle.textContent = "Download binary file.";
  divEle.appendChild(downloadEle);

  mainEle.appendChild(divEle);

  const displayAnywayEle = document.createElement("p");
  displayAnywayEle.classList.add("display-anyways");
  displayAnywayEle.classList.add("hljs-comment");
  displayAnywayEle.textContent = "Display anyways?";
  displayAnywayEle.onclick = () => {
    data.data.text().then(text => {
      data.data = text;
      createStringPasteUi(data, "application/octet-stream");
    })
  };
  mainEle.appendChild(displayAnywayEle);
  bodyEle.appendChild(mainEle);
}

function createImagePasteUi({ expiration, data, file_size }, name?: string) {
  createMultiMediaPasteUi("img", expiration, data, name, (downloadEle, imgEle) => {
    imgEle.onload = () => {
      const width = imgEle.naturalWidth || imgEle.width;
      const height = imgEle.naturalHeight || imgEle.height;
      downloadEle.textContent = "Download " + file_size + " \u2014 " + width + " by " + height;
    }
  });
}

function createAudioPasteUi({ expiration, data }, name?: string) {
  createMultiMediaPasteUi("audio", expiration, data, name, "Download");
}

function createVideoPasteUi({ expiration, data }, name?: string) {
  createMultiMediaPasteUi("video", expiration, data, name, "Download");
}

function createArchivePasteUi({ expiration, data, entries }, name?: string) {
  const bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  const mainEle = document.createElement("main");

  const sectionEle = document.createElement("section");
  sectionEle.classList.add("paste");

  const expirationEle = document.createElement("p");
  expirationEle.textContent = expiration;
  expirationEle.classList.add("centered");
  sectionEle.appendChild(expirationEle);

  const downloadEle = document.createElement("a");
  downloadEle.href = getObjectUrl(data);
  downloadEle.download = name;
  downloadEle.textContent = "Download";
  downloadEle.classList.add("hljs-meta");
  downloadEle.classList.add("centered");
  sectionEle.appendChild(downloadEle);

  sectionEle.appendChild(document.createElement("hr"));

  const mediaEle = document.createElement("table");
  mediaEle.classList.add("archive-table");
  const tr = mediaEle.insertRow();
  tr.classList.add("hljs-title");
  const tdName = tr.insertCell();
  tdName.textContent = "Name";
  const tdSize = tr.insertCell();
  tdSize.classList.add("align-right");
  tdSize.textContent = "File Size";

  // Because it's a stable sort, we can first sort by name (to get all folder
  // items grouped together) and then sort by if there's a / or not.
  entries.sort((a, b) => {
    return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
  });

  // This doesn't get sub directories and their folders, but hey it's close
  // enough
  entries.sort((a, b) => {
    return b.name.includes("/") - a.name.includes("/");
  });

  for (const { name, file_size } of entries) {
    const tr = mediaEle.insertRow();
    const tdName = tr.insertCell();
    tdName.textContent = name;
    const tdSize = tr.insertCell();
    tdSize.textContent = file_size;
    tdSize.classList.add("align-right");
    tdSize.classList.add("hljs-number");
  }

  sectionEle.appendChild(mediaEle);
  mainEle.appendChild(sectionEle);
  bodyEle.appendChild(mainEle);
}

function createMultiMediaPasteUi(tag, expiration, data, name?: string, on_create?) {
  const bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  const mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");

  const downloadLink = getObjectUrl(data, name);

  const expirationEle = document.createElement("p");
  expirationEle.textContent = expiration;
  mainEle.appendChild(expirationEle);

  const mediaEle = document.createElement(tag);
  mediaEle.src = downloadLink;
  mediaEle.controls = true;
  mainEle.appendChild(mediaEle);


  const downloadEle = document.createElement("a");
  downloadEle.href = downloadLink;
  downloadEle.download = name;
  downloadEle.classList.add("hljs-meta");
  mainEle.appendChild(downloadEle);

  bodyEle.appendChild(mainEle);

  if (on_create instanceof Function) {
    on_create(downloadEle, mediaEle);
  } else {
    downloadEle.textContent = on_create;
  }
}

function renderMessage(message) {
  const body = document.getElementsByTagName("body")[0];
  body.textContent = '';
  const mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");
  mainEle.textContent = message;
  body.appendChild(mainEle);
}

function getObjectUrl(data, mimeType?: string) {
  return URL.createObjectURL(new Blob(data, {
    type: mimeType,
  }));
}

window.addEventListener("hashchange", () => location.reload());
