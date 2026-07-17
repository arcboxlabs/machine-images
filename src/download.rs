//! Verified streaming downloads.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// Builds the HTTP client used for catalog and image fetches.
///
/// No total-request timeout: rootfs images run to hundreds of MB and their
/// transfer time is unbounded by design; the connect timeout still catches
/// dead servers.
///
/// # Errors
/// Returns an error when the TLS backend fails to initialize.
pub fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("machine-images/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(None)
        .build()
        .context("build HTTP client")
}

/// Downloads `url` to `dest`, streaming through SHA-256. On any size or
/// digest mismatch the partial file is removed and no file exists at `dest`.
///
/// # Errors
/// Returns an error on network/IO failure or a size/digest mismatch.
pub fn fetch_verified(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<()> {
    let mut resp = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .with_context(|| format!("GET {url}"))?;

    if let Some(parent) = dest.parent() {
        fs_err::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("partial");
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    {
        let mut file = fs_err::File::create(&tmp)?;
        let mut buf = [0u8; 1 << 16];
        loop {
            let n = resp
                .read(&mut buf)
                .with_context(|| format!("read body of {url}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            written += n as u64;
        }
        file.flush()?;
    }

    let digest = hex::encode(hasher.finalize());
    if written != expected_size || digest != expected_sha256 {
        let _ = fs_err::remove_file(&tmp);
        bail!(
            "{url}: expected {expected_size} bytes sha256:{expected_sha256}, \
             got {written} bytes sha256:{digest}"
        );
    }
    fs_err::rename(&tmp, dest)?;
    Ok(())
}

/// Hex SHA-256 and size of a local file.
///
/// # Errors
/// Returns an error when the file cannot be read.
pub fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut file = fs_err::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1 << 16];
    let mut size: u64 = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}
