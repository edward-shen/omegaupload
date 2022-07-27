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

import './main.scss';
import ReactDom from 'react-dom';
import React, { useState } from 'react';
import { encrypt_string } from '../pkg';

import hljs from 'highlight.js'
(window as any).hljs = hljs;
require('highlightjs-line-numbers.js');

const PasteForm = () => {
  const [value, setValue] = useState("");

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    encrypt_string(value);
  }

  return (
    <pre className='paste'>
      <form className='hljs centered' onSubmit={handleSubmit}>
        <textarea
          placeholder="Sample text"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <input type="submit" value="submit" />
      </form>
    </pre>
  )
}

function createUploadUi() {
  const html = <main className='hljs centered fullscreen'>
    <PasteForm />
  </main>;

  ReactDom.render(html, document.body);
}

function loadFromDb(mimeType: string, name?: string, language?: string) {
  let resolvedName;
  if (name) {
    resolvedName = name;
  } else {
    const pathName = window.location.pathname;
    const leafIndex = pathName.indexOf("/");
    resolvedName = pathName.slice(leafIndex + 1);
  }

  console.log("[js] Resolved name:", resolvedName);
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
          console.info("[js] Rendering string UI.");
          createStringPasteUi(data, mimeType, resolvedName, language);
          break;
        case "blob":
          console.info("[js] Rendering blob UI.");
          createBlobPasteUi(data, resolvedName);
          break;
        case "image":
          console.info("[js] Rendering image UI.");
          createImagePasteUi(data, resolvedName, mimeType);
          break;
        case "audio":
          console.info("[js] Rendering audio UI.");
          createAudioPasteUi(data, resolvedName, mimeType);
          break;
        case "video":
          console.info("[js] Rendering video UI.");
          createVideoPasteUi(data, resolvedName, mimeType);
          break;
        case "archive":
          console.info("[js] Rendering archive UI.");
          createArchivePasteUi(data, resolvedName);
          break;
        default:
          console.info("[js] Rendering unknown UI.");
          renderMessage("Something went wrong. Try clearing local data.");
          break;
      }

      // IDB was only used as a temporary medium;
      window.onbeforeunload = (_e) => {
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

function createStringPasteUi(data, mimeType: string, name: string, lang?: string) {
  const html = <main>
    <pre className='paste'>
      <p className='unselectable centered'>{data.expiration}</p>
      <a href={getObjectUrl([data.data], mimeType)} download={name} className='hljs-meta centered'>
        Download file.
      </a>
      <hr />
      <code>
        {data.data}
      </code>
    </pre>
  </main>;

  ReactDom.render(html, document.body);

  let languages = undefined;

  if (!hljs.getLanguage(lang)) {
    if (lang) {
      console.warn(`[js] User provided language (${lang}) is not known.`);
    } else {
      console.info(`[js] Language hint not provided.`);
    }
  } else {
    languages = [lang];
  }

  // If a language wasn't provided, see if we can use the file extension to give
  // us a better hint for hljs
  if (!languages) {
    if (name) {
      console.log("[js] Trying to infer from file name...");
      const periodIndex = name.indexOf(".");
      if (periodIndex === -1) {
        console.warn("[js] Did not find file extension.")
      } else {
        let extension = name.slice(periodIndex + 1);
        console.info(`[js] Found extension ${extension}.`);
        if (!hljs.getLanguage(extension)) {
          console.warn(`[js] Extension was not recognized by hljs. Giving up.`);
        } else {
          console.info("[js] Successfully inferred language from file extension.");
          languages = [extension];
        }
      }
    } else {
      console.log("[js] No file name hint provided.");
    }
  } else {
    console.info(`[js] Selecting user provided language ${languages[0]} for highlighting.`);
  }

  // If we still haven't set languages here, then we're leaving it up to the
  if (!languages) {
    console.log("[js] Deferring to hljs inference for syntax highlighting.");
  } else {
    hljs.configure({ languages });
  }

  hljs.highlightAll();


  (hljs as any).initLineNumbersOnLoad();
}

function createBlobPasteUi(data, name: string) {
  const html = <main className='hljs centered fullscreen'>
    <div className='centered'>
      <p>{data.expiration}</p>
      <a href={getObjectUrl(data.data, name)} download={name} className='hljs-meta'>
        Download binary file.
      </a>
    </div>
    <p className='display-anyways hljs-comment' onClick={() => {
      data.data.text().then(text => {
        data.data = text;
        createStringPasteUi(data, "application/octet-stream", name);
      })
    }}>Display anyways?</p>
  </main>;

  ReactDom.render(html, document.body);
}

function createImagePasteUi({ expiration, data, file_size }, name: string, mimeType: string) {
  createMultiMediaPasteUi("img", expiration, data, name, mimeType, (downloadEle, imgEle) => {
    imgEle.onload = () => {
      const width = imgEle.naturalWidth || imgEle.width;
      const height = imgEle.naturalHeight || imgEle.height;
      downloadEle.textContent = "Download " + file_size + " \u2014 " + width + " by " + height;
    }
  });
}

function createAudioPasteUi({ expiration, data }, name: string, mimeType: string) {
  createMultiMediaPasteUi("audio", expiration, data, name, mimeType, "Download");
}

function createVideoPasteUi({ expiration, data }, name: string, mimeType: string) {
  createMultiMediaPasteUi("video", expiration, data, name, mimeType, "Download");
}

function createArchivePasteUi({ expiration, data, entries }, name: string) {
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

  const html = <main>
    <section className='paste'>
      <p className='centered'>{expiration}</p>
      <a href={getObjectUrl(data)} download={name} className='hljs-meta centered'>Download</a>
      <hr />
      <table className='archive-table'>
        <thead>
          <tr className='hljs-title'><th>Name</th><th className='align-right'>File Size</th></tr>
        </thead>
        <tbody>
          {
            entries.map(({ name, file_size }) => {
              return <tr><td>{name}</td><td className='align-right hljs-number'>{file_size}</td></tr>;
            })
          }
        </tbody>
      </table>
    </section>
  </main>;

  ReactDom.render(html, document.body);

}

function createMultiMediaPasteUi(tag, expiration, data, name: string, mimeType: string, on_create?: Function | string) {
  const bodyEle = document.body;
  bodyEle.textContent = '';

  const mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");

  const downloadLink = getObjectUrl(data, mimeType);

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
  ReactDom.render(
    <main className='hljs centered fullscreen'>
      {message}
    </main>,
    document.body,
  );
}

function getObjectUrl(data, mimeType?: string) {
  return URL.createObjectURL(new Blob([data], { type: mimeType }));
}

window.addEventListener("hashchange", () => location.reload());

export { renderMessage, createUploadUi, loadFromDb };
