#![warn(clippy::nursery, clippy::pedantic)]
#![deny(unsafe_code)]

use std::io::{Read, Write};

use anyhow::{anyhow, bail, Context, Result};
use atty::Stream;
use clap::Clap;
use omegaupload_common::crypto::{gen_key_nonce, open_in_place, seal_in_place, Key};
use omegaupload_common::{base64, hash, Expiration, ParsedUrl, Url};
use reqwest::blocking::Client;
use reqwest::header::EXPIRES;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};

#[derive(Clap)]
struct Opts {
    #[clap(subcommand)]
    action: Action,
}

#[derive(Clap)]
enum Action {
    Upload {
        url: Url,
        #[clap(short, long)]
        password: Option<SecretString>,
    },
    Download {
        url: ParsedUrl,
    },
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    match opts.action {
        Action::Upload { url, password } => handle_upload(url, password),
        Action::Download { url } => handle_download(url),
    }?;

    Ok(())
}

fn handle_upload(mut url: Url, password: Option<SecretString>) -> Result<()> {
    url.set_fragment(None);

    if atty::is(Stream::Stdin) {
        bail!("This tool requires non interactive CLI. Pipe something in!");
    }

    let (data, nonce, key, pw_used) = {
        let (enc_key, nonce) = gen_key_nonce();
        let mut container = Vec::new();
        std::io::stdin().read_to_end(&mut container)?;
        seal_in_place(&mut container, &nonce, &enc_key)
            .map_err(|_| anyhow!("Failed to encrypt data"))?;

        let pw_used = if let Some(password) = password {
            let pw_hash = hash(password.expose_secret().as_bytes());
            let pw_key = Key::from_slice(pw_hash.as_ref());
            seal_in_place(&mut container, &nonce.increment(), pw_key)
                .map_err(|_| anyhow!("Failed to encrypt data"))?;
            true
        } else {
            false
        };

        let key = base64::encode(&enc_key);
        let nonce = base64::encode(&nonce);

        (container, nonce, key, pw_used)
    };

    let res = Client::new()
        .post(url.as_ref())
        .body(data)
        .send()
        .context("Request to server failed")?;

    if res.status() != StatusCode::OK {
        bail!("Upload failed. Got HTTP error {}", res.status());
    }

    url.path_segments_mut()
        .map_err(|_| anyhow!("Failed to get base URL"))?
        .extend(std::iter::once(res.text()?));

    let mut fragment = format!("key:{}!nonce:{}", key, nonce);

    if pw_used {
        fragment.push_str("!pw");
    }

    url.set_fragment(Some(&fragment));

    println!("{}", url);

    Ok(())
}

fn handle_download(url: ParsedUrl) -> Result<()> {
    let res = Client::new()
        .get(url.sanitized_url)
        .send()
        .context("Failed to get data")?;

    if res.status() != StatusCode::OK {
        bail!("Got bad response from server: {}", res.status());
    }

    let expiration_text = dbg!(res.headers())
        .get(EXPIRES)
        .and_then(|v| Expiration::try_from(v).ok())
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "This paste will not expire.".to_string());

    let mut data = res.bytes()?.as_ref().to_vec();

    if url.needs_password {
        // Only print prompt on interactive, else it messes with output
        if atty::is(Stream::Stdout) {
            print!("Please enter the password to access this document: ");
            std::io::stdout().flush()?;
        }
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        input.pop(); // last character is \n, we need to drop it.

        let pw_hash = hash(input.as_bytes());
        let pw_key = Key::from_slice(pw_hash.as_ref());

        open_in_place(&mut data, &url.nonce.increment(), pw_key)
            .map_err(|_| anyhow!("Failed to decrypt data. Incorrect password?"))?;
    }

    open_in_place(&mut data, &url.nonce, &url.decryption_key)
        .map_err(|_| anyhow!("Failed to decrypt data. Incorrect decryption key?"))?;

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
