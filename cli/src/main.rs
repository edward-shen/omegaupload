#![warn(clippy::nursery, clippy::pedantic)]
#![deny(unsafe_code)]

// OmegaUpload CLI Client
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

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use atty::Stream;
use clap::Parser;
use omegaupload_common::crypto::{open_in_place, seal_in_place};
use omegaupload_common::fragment::Builder;
use omegaupload_common::secrecy::{ExposeSecret, SecretString, SecretVec};
use omegaupload_common::{
    base64, Expiration, ParsedUrl, Url, API_ENDPOINT, EXPIRATION_HEADER_NAME,
};
use reqwest::blocking::Client;
use reqwest::header::EXPIRES;
use reqwest::StatusCode;
use rpassword::prompt_password;

#[derive(Parser)]
struct Opts {
    #[clap(subcommand)]
    action: Action,
}

#[derive(Parser)]
enum Action {
    /// Upload a paste to an omegaupload server.
    Upload {
        /// The OmegaUpload instance to upload data to.
        url: Url,
        /// Encrypt the uploaded paste with the provided password, preventing
        /// public access.
        #[clap(short, long)]
        password: bool,
        /// How long for the paste to last, or until someone has read it.
        #[clap(short, long, possible_values = Expiration::variants())]
        duration: Option<Expiration>,
        /// The path to the file to upload. If none is provided, then reads
        /// stdin instead.
        path: Option<PathBuf>,
        /// Hint that the uploaded file should be syntax highlighted with a
        /// specific language.
        #[clap(short, long)]
        language: Option<String>,
        /// Don't provide a file name hint.
        #[clap(short = 'F', long)]
        no_file_name_hint: bool,
    },
    /// Download a paste from an omegaupload server.
    Download {
        /// The paste to download.
        url: ParsedUrl,
    },
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    match opts.action {
        Action::Upload {
            url,
            password,
            duration,
            path,
            language,
            no_file_name_hint,
        } => handle_upload(url, password, duration, path, language, no_file_name_hint),
        Action::Download { url } => handle_download(url),
    }?;

    Ok(())
}

fn handle_upload(
    mut url: Url,
    password: bool,
    duration: Option<Expiration>,
    path: Option<PathBuf>,
    language: Option<String>,
    no_file_name_hint: bool,
) -> Result<()> {
    url.set_fragment(None);

    if password && path.is_none() {
        bail!("Reading data from stdin is incompatible with a password. Provide a path to a file to upload.");
    }

    let (data, key) = {
        let mut container = if let Some(ref path) = path {
            std::fs::read(path)?
        } else {
            let mut container = vec![];
            std::io::stdin().lock().read_to_end(&mut container)?;
            container
        };

        if container.is_empty() {
            bail!("Nothing to upload.");
        }

        let password = if password {
            let maybe_password = prompt_password("Please set the password for this paste: ")?;
            Some(SecretVec::new(maybe_password.into_bytes()))
        } else {
            None
        };

        let enc_key = seal_in_place(&mut container, password)?;
        let key = SecretString::new(base64::encode(&enc_key.expose_secret().as_ref()));
        (container, key)
    };

    let mut res = Client::new().post(url.as_ref());

    if let Some(duration) = duration {
        res = res.header(&*EXPIRATION_HEADER_NAME, duration);
    }

    let res = res.body(data).send().context("Request to server failed")?;

    if res.status() != StatusCode::OK {
        bail!("Upload failed. Got HTTP error {}", res.status());
    }

    url.path_segments_mut()
        .map_err(|_| anyhow!("Failed to get base URL"))?
        .extend(std::iter::once(res.text()?));

    let mut fragment = Builder::new(key);
    if password {
        fragment = fragment.needs_password();
    }

    if !no_file_name_hint {
        let file_name = path.and_then(|path| {
            path.file_name()
                .map(|str| str.to_string_lossy().to_string())
        });
        if let Some(file_name) = file_name {
            fragment = fragment.file_name(file_name);
        }
    }

    if let Some(language) = language {
        fragment = fragment.language(language);
    }

    url.set_fragment(Some(fragment.build().expose_secret()));

    println!("{url}");

    Ok(())
}

fn handle_download(mut url: ParsedUrl) -> Result<()> {
    url.sanitized_url
        .set_path(&format!("{API_ENDPOINT}{}", url.sanitized_url.path()));
    let res = Client::new()
        .get(url.sanitized_url)
        .send()
        .context("Failed to get data")?;

    if res.status() != StatusCode::OK {
        bail!("Got bad response from server: {}", res.status());
    }

    let expiration_text = res
        .headers()
        .get(EXPIRES)
        .and_then(|v| Expiration::try_from(v).ok())
        .as_ref()
        .map_or_else(
            || "This paste will not expire.".to_string(),
            ToString::to_string,
        );

    let mut data = res.bytes()?.as_ref().to_vec();

    let password = if url.needs_password {
        // Only print prompt on interactive, else it messes with output
        let maybe_password = prompt_password("Please enter the password to access this paste: ")?;
        Some(SecretVec::new(maybe_password.into_bytes()))
    } else {
        None
    };

    open_in_place(&mut data, &url.decryption_key, password)?;

    if atty::is(Stream::Stdout) {
        if let Ok(data) = String::from_utf8(data) {
            std::io::stdout().write_all(data.as_bytes())?;
        } else {
            bail!("Binary output detected. Please pipe to a file.");
        }
    } else {
        std::io::stdout().write_all(&data)?;
    }

    eprintln!("{expiration_text}");

    Ok(())
}
