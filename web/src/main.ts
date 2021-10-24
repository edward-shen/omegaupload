// Exported to main.rs
function loadFromDb() {
  const dbReq = window.indexedDB.open("omegaupload", 1);
  dbReq.onsuccess = (evt) => {
    const db = (evt.target as IDBRequest).result;
    const obj_store = db
      .transaction("decrypted data", "readonly")
      .objectStore("decrypted data")
      .get(window.location.pathname);
    obj_store.onsuccess = (evt) => {
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
        default:
          createBrokenStateUi();
          break;
      }
    };

    obj_store.onerror = (evt) => {
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

function createImagePasteUi(data) {
  let bodyEle = document.getElementsByTagName("body")[0];
  bodyEle.textContent = '';

  let mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");

  const downloadLink = URL.createObjectURL(data.data);

  let expirationEle = document.createElement("p");
  expirationEle.textContent = data.expiration;
  mainEle.appendChild(expirationEle);

  let imgEle = document.createElement("img");
  imgEle.src = downloadLink;
  mainEle.appendChild(imgEle);


  let downloadEle = document.createElement("a");
  downloadEle.href = downloadLink;
  downloadEle.download = window.location.pathname;
  downloadEle.classList.add("hljs-meta");
  downloadEle.textContent = data.button;
  mainEle.appendChild(downloadEle);


  bodyEle.appendChild(mainEle);
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

// Exported to main.rs
function createNotFoundUi() {
  let body = document.getElementsByTagName("body")[0];
  body.textContent = '';
  body.appendChild(createGenericError("Either the paste has been burned or one never existed."));
}

function createBrokenStateUi() {
  let body = document.getElementsByTagName("body")[0];
  body.textContent = '';
  body.appendChild(createGenericError("Something went wrong. Try clearing local data."));
}

function createGenericError(message) {
  let mainEle = document.createElement("main");
  mainEle.classList.add("hljs");
  mainEle.classList.add("centered");
  mainEle.classList.add("fullscreen");
  mainEle.textContent = message;
  return mainEle;
}

window.addEventListener("hashchange", () => location.reload());