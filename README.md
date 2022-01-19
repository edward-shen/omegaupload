# OmegaUpload

OmegaUpload is a zero-knowledge temporary file hosting service.

## Examples

```bash
# Uploading a file:
$ omegaupload upload https://paste.example.com path/to/file
https://paste.example.com/PgRG8Hfrr9rR#I1FG2oejo2gSjB3Ym1mEmRfcN4X8GXc2pZtZeiSsWFo=

# Uploading a file with a password:
$ omegaupload upload -p https://paste.example.com path/to/file
Please set the password for this paste:
https://paste.example.com/862vhXVp3v9R#key:tbGxzHBNnXjS2eq89X9uvZKz_i8bvapLPEp8g0waQrc=!pw

# Downloading a file:
$ omegaupload download https://paste.example.com/PgRG8Hfrr9rR#I1FG2oejo2gSjB3Ym1mEmRfcN4X8GXc2pZtZeiSsWFo=
```

## Features

- Server has zero knowledge of uploaded data when uploading through a supported
  frontend (Direct, plaintext upload is possible but unsupported).
- Only metadata stored on server is expiration time. This is a strong guarantee.
- All cryptographic functions are performed on the client side and are done via
  a single common library, to minimize risk of programming error.
- Modern crypto functions are used with recommended parameters:
  XChaCha20Poly1305 for encryption and Argon2id for KDF.
- Customizable expiration times, from burn-after-read to 1 day.

## Building from source

Prerequisites:
- `yarn` 1.22.17 or later (Earlier versions untested but likely to work)
- Cargo, with support for the latest Rust version
- _(Optional)_ zstd, for zipping up the file for distribution

First, run `git submodule update --init --recursive`.

Then, run `./bin/build.sh` for a `dist.tar.zst` to be generated, where you can
simply extract that folder and run the binary provided. The server will listen
on port `8080`.

### Running a local server

After running `./bin/build.sh`, you can cd into the `dist` and run
`./omegaupload-server`. It will run on port 8000, and will respond to HTTP
requests.

You can then point an omegaupload CLI instance (or run
`cargo run --bin omegaupload`) as an upload server.

If you're only changing the frontend (and not updating the server code), you can
run `yarn build` for faster iteration.

## Why OmegaUpload?

OmegaUpload's primary benefit is that the frontends use a unified common library
utilizing XChaCha20Poly1305 to encrypt and decrypt files.

### Security

The primary goal was to provide a unified library across both a CLI tool and
through the web frontend to minimize risk of compromise. As a result, the CLI
tool and the web frontend both utilize a Rust library whose crypto module
exposes two functions to encrypt and decrypt that only accept a message and
necessarily key material or return only necessary key material. This small API
effectively makes it impossible to have differences between the frontend, and
ensures that the attack surface is limited to these functions.

#### Password KDF

If a password is provided at encryption time, argon2 is used as a key derivation
function. Specifically, the library meets or exceeds OWASP recommended
parameters:
 - Argon2id is used.
 - Algorithm version is `0x13`.
 - Parameters are `m = 15MiB`, `t = 2`, `p = 2`.

 Additionally, a salt size of 16 bytes are used.

#### Blob Encryption

XChaCha20Poly1305 was used as the encryption method as it is becoming the
mainstream recommended method for encrypting messages. This was chosen over AES
primarily due to its strength in related-key attacks, as well as its widespread
recognition and usage in WireGuard, Quic, and TLS.

As this crate uses `XChaCha20`, a 24 byte nonce and a 32 bytes key are used.

#### Secrecy

Encryption and decryption functions offered by the common crate only accept or
return key material that will be properly zeroed on destruction. This is
enforced by the `secrecy` crate, which, on top of offering type wrappers that
zero the memory on drop, provide an easy way to audit when secrets are exposed.

This also means that to use these two functions necessarily requires the caller
to enclose key material in the wrapped type first, reducing possibility for key
material to remain in memory.

#### Memory Safety

Rust eliminates an entire class of memory-related bugs, and any `unsafe` block
is documented with a safety comment. This allows for easy auditing of memory
suspect code, and permits

## Why not OmegaUpload?

There are a few reasons to not use OmegaUpload:
 - Limited to 3GB uploads&mdash;this is a soft limit of RocksDB.
 - Cannot download files larger than 512 MiB through the web frontend&mdash;this
   is a technical limitation of the current web frontend not using a web worker
   in addition to the fact that browsers are not optimized for XChaCha20.
 - Right now, you must upload via the CLI tool.
 - The frontend uses WASM, which is a novel attack surface.
