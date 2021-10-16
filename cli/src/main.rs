use std::io::{Read, Write};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use atty::Stream;
use clap::Clap;
use reqwest::blocking::Client;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use sodiumoxide::base64;
use sodiumoxide::base64::Variant::UrlSafe;
use sodiumoxide::crypto::hash::sha256;
use sodiumoxide::crypto::secretbox::{gen_key, gen_nonce, open, seal, Key, Nonce, KEYBYTES};
use url::Url;

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
    sodiumoxide::init().map_err(|_| anyhow!("Failed to init sodiumoxide"))?;
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
        let enc_key = gen_key();
        let nonce = gen_nonce();
        let mut container = Vec::new();
        std::io::stdin().read_to_end(&mut container)?;
        let mut enc = seal(&container, &nonce, &enc_key);

        let pw_used = if let Some(password) = password {
            assert_eq!(sha256::DIGESTBYTES, KEYBYTES);
            let pw_hash = sha256::hash(password.expose_secret().as_bytes());
            let pw_key = Key::from_slice(pw_hash.as_ref()).expect("to succeed");
            enc = seal(&enc, &nonce.increment_le(), &pw_key);
            true
        } else {
            false
        };

        let key = base64::encode(&enc_key, UrlSafe);
        let nonce = base64::encode(&nonce, UrlSafe);

        (enc, nonce, key, pw_used)
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

        assert_eq!(sha256::DIGESTBYTES, KEYBYTES);
        let pw_hash = sha256::hash(input.as_bytes());
        let pw_key = Key::from_slice(pw_hash.as_ref()).expect("to succeed");

        data = open(&data, &url.nonce.increment_le(), &pw_key)
            .map_err(|_| anyhow!("Failed to decrypt data. Incorrect password?"))?;
    }

    data = open(&data, &url.nonce, &url.decryption_key)
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

    Ok(())
}

struct ParsedUrl {
    sanitized_url: Url,
    decryption_key: Key,
    nonce: Nonce,
    needs_password: bool,
}

impl FromStr for ParsedUrl {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut url = Url::from_str(s)?;
        let fragment = url
            .fragment()
            .context("Missing fragment. The decryption key is part of the fragment.")?;
        if fragment.is_empty() {
            bail!("Empty fragment. The decryption key is part of the fragment.");
        }

        let args = fragment.split('!').filter_map(|kv| {
            let (k, v) = {
                let mut iter = kv.split(':');
                (iter.next(), iter.next())
            };

            Some((k?, v))
        });

        let mut decryption_key = None;
        let mut needs_password = false;
        let mut nonce = None;

        for (key, value) in args {
            match (key, value) {
                ("key", Some(value)) => {
                    let key = base64::decode(value, UrlSafe)
                        .map_err(|_| anyhow!("Failed to decode key"))?;
                    let key = Key::from_slice(&key).context("Failed to parse key")?;
                    decryption_key = Some(key);
                }
                ("pw", _) => {
                    needs_password = true;
                }
                ("nonce", Some(value)) => {
                    nonce = Some(
                        Nonce::from_slice(
                            &base64::decode(value, UrlSafe)
                                .map_err(|_| anyhow!("Failed to decode nonce"))?,
                        )
                        .context("Invalid nonce provided")?,
                    );
                }
                _ => (),
            }
        }

        url.set_fragment(None);
        Ok(Self {
            sanitized_url: url,
            decryption_key: decryption_key.context("Missing decryption key")?,
            needs_password,
            nonce: nonce.context("Missing nonce")?,
        })
    }
}
