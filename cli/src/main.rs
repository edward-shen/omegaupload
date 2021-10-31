#![warn(clippy::nursery, clippy::pedantic)]
#![deny(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use atty::Stream;
use clap::Parser;
use omegaupload_common::crypto::{open_in_place, seal_in_place};
use omegaupload_common::secrecy::{ExposeSecret, SecretVec};
use omegaupload_common::{
    base64, Expiration, ParsedUrl, Url, API_ENDPOINT, EXPIRATION_HEADER_NAME,
};
use reqwest::blocking::Client;
use reqwest::header::EXPIRES;
use reqwest::StatusCode;
use rpassword::prompt_password_stderr;

#[derive(Parser)]
struct Opts {
    #[clap(subcommand)]
    action: Action,
}

#[derive(Parser)]
enum Action {
    Upload {
        /// The OmegaUpload instance to upload data to.
        url: Url,
        /// Encrypt the uploaded paste with the provided password, preventing
        /// public access.
        #[clap(short, long)]
        password: bool,
        #[clap(short, long)]
        duration: Option<Expiration>,
        path: PathBuf,
    },
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
        } => handle_upload(url, password, duration, path),
        Action::Download { url } => handle_download(url),
    }?;

    Ok(())
}

fn handle_upload(
    mut url: Url,
    password: bool,
    duration: Option<Expiration>,
    path: PathBuf,
) -> Result<()> {
    url.set_fragment(None);

    let (data, key) = {
        let mut container = std::fs::read(path)?;
        let password = if password {
            let maybe_password =
                prompt_password_stderr("Please set the password for this paste: ")?;
            Some(SecretVec::new(maybe_password.into_bytes()))
        } else {
            None
        };

        let enc_key = seal_in_place(&mut container, password)?;
        let key = base64::encode(&enc_key.expose_secret().as_ref());
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

    let fragment = if password {
        format!("key:{}!pw", key)
    } else {
        key
    };

    url.set_fragment(Some(&fragment));

    println!("{}", url);

    Ok(())
}

fn handle_download(mut url: ParsedUrl) -> Result<()> {
    url.sanitized_url
        .set_path(&format!("{}{}", API_ENDPOINT, url.sanitized_url.path()));
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

    let mut password = None;
    if url.needs_password {
        // Only print prompt on interactive, else it messes with output
        let maybe_password =
            prompt_password_stderr("Please enter the password to access this paste: ")?;
        password = Some(SecretVec::new(maybe_password.into_bytes()));
    }

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

    eprintln!("{}", expiration_text);

    Ok(())
}
