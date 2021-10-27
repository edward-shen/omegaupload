// Exported to main.rs
function loadFromDb() {
  const dbReq = window.indexedDB.open("omegaupload", 1);
  dbReq.onsuccess = (evt) => {
    const db = (evt.target as IDBRequest).result;
    const obj_store = db
      .transaction("decrypted data")
      .objectStore("decrypted data");
    let fetchReq = obj_store.get(window.location.pathname);
    fetchReq.onsuccess = (evt) => {
      const data = (evt.target as IDBRequest).result;
      switch (data.type) {
        case "string":
          createStringPasteUi(data);
          break;
        case "blob":
          createBlobPasteUi(data);
          break;
        case "image":
          createImagePasteUi(data);
          break;
        case "audio":
          createAudioPasteUi(data);
          break;
        case "video":
          createVideoPasteUi(data);
          break;
        case "archive":
          createArchivePasteUi(data);
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

function createStringPasteUi(data) {
  let bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  let mainEle = document.createElement("main");
  let preEle = document.createElement("pre");
  preEle.classList.add("paste");

  let headerEle = document.createElement("header");
  headerEle.classList.add("unselectable");
  headerEle.textContent = data.expiration;
  preEle.appendChild(headerEle);

  preEle.appendChild(document.createElement("hr"));

  let codeEle = document.createElement("code");
  codeEle.textContent = data.data;
  preEle.appendChild(codeEle);

  mainEle.appendChild(preEle);
  bodyEle.appendChild(mainEle);

  hljs.highlightAll();
  hljs.initLineNumbersOnLoad();
}

function createBlobPasteUi(data) {
  let bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  let mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");

  let divEle = document.createElement("div");
  divEle.classList.add("centered");

  let expirationEle = document.createElement("p");
  expirationEle.textContent = data.expiration;
  divEle.appendChild(expirationEle);

  let downloadEle = document.createElement("a");
  downloadEle.href = URL.createObjectURL(data.data);
  downloadEle.download = window.location.pathname;
  downloadEle.classList.add("hljs-meta");
  downloadEle.textContent = "Download binary file.";
  divEle.appendChild(downloadEle);


  mainEle.appendChild(divEle);

  let displayAnywayEle = document.createElement("p");
  displayAnywayEle.classList.add("display-anyways");
  displayAnywayEle.classList.add("hljs-comment");
  displayAnywayEle.textContent = "Display anyways?";
  displayAnywayEle.onclick = () => {
    data.data.text().then(text => {
      data.data = text;
      createStringPasteUi(data);
    })
  };
  mainEle.appendChild(displayAnywayEle);
  bodyEle.appendChild(mainEle);
}

function createImagePasteUi({ expiration, data, file_size }) {
  createMultiMediaPasteUi("img", expiration, data, (downloadEle, imgEle) => {
    imgEle.onload = () => {
      downloadEle.textContent = "Download " + file_size + " \u2014 " + imgEle.naturalWidth + " by " + imgEle.naturalHeight;
    }
  });
}

function createAudioPasteUi({ expiration, data }) {
  createMultiMediaPasteUi("audio", expiration, data, "Download");
}

function createVideoPasteUi({ expiration, data }) {
  createMultiMediaPasteUi("video", expiration, data, "Download");
}

function createArchivePasteUi({ expiration, data, entries }) {
  let bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  let mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");

  const downloadLink = URL.createObjectURL(data);

  let expirationEle = document.createElement("p");
  expirationEle.textContent = expiration;
  mainEle.appendChild(expirationEle);

  let mediaEle = document.createElement("table");
  mediaEle.style.width = "50%";
  const tr = mediaEle.insertRow();
  const tdName = tr.insertCell();
  tdName.appendChild(document.createTextNode("Name"));
  const tdSize = tr.insertCell();
  tdSize.appendChild(document.createTextNode("File Size"));
  for (const entry of entries) {
    const tr = mediaEle.insertRow();
    const tdName = tr.insertCell();
    tdName.appendChild(document.createTextNode(entry.name));
    const tdSize = tr.insertCell();
    tdSize.appendChild(document.createTextNode(entry.file_size));
  }
  mainEle.appendChild(mediaEle);

  let downloadEle = document.createElement("a");
  downloadEle.href = downloadLink;
  downloadEle.download = window.location.pathname;
  downloadEle.classList.add("hljs-meta");
  mainEle.appendChild(downloadEle);

  bodyEle.appendChild(mainEle);

  downloadEle.textContent = "Download";
}

function createMultiMediaPasteUi(tag, expiration, data, on_create?) {
  let bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  let mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");

  const downloadLink = URL.createObjectURL(data);

  let expirationEle = document.createElement("p");
  expirationEle.textContent = expiration;
  mainEle.appendChild(expirationEle);

  let mediaEle = document.createElement(tag);
  mediaEle.src = downloadLink;
  mediaEle.controls = true;
  mainEle.appendChild(mediaEle);

  let downloadEle = document.createElement("a");
  downloadEle.href = downloadLink;
  downloadEle.download = window.location.pathname;
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
  let body = document.getElementsByTagName("body")[0];
  body.textContent = '';
  let mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");
  mainEle.textContent = message;
  body.appendChild(mainEle);
}

window.addEventListener("hashchange", () => location.reload());
