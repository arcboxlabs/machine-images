//! Sync planning and staging.
//!
//! Reads the upstream catalog, mirrors every configured product whose newest
//! build is not yet in the published index, and writes a staging directory
//! ready for `b2-publish`:
//!
//! ```text
//! out/
//! ├── index.json                        # merged: published state ∪ new builds
//! └── {stream}/{version}/
//!     ├── manifest.json
//!     └── rootfs.squashfs
//! ```
//!
//! Retention is add-only: streams present in the published index but no
//! longer configured (or pruned upstream) are carried over untouched, so
//! existing machines can always re-resolve their image.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::download;
use crate::index::{
    Index, MANIFEST_SCHEMA_VERSION, Manifest, RootfsFile, StreamMeta, UpstreamRef,
    normalize_version, stream_name,
};
use crate::upstream::{CATALOG_PATH, Catalog, ProductKey};

/// One build staged by a sync run.
#[derive(Debug, Serialize)]
pub struct StagedBuild {
    pub stream: String,
    pub version: String,
    pub bytes: u64,
}

/// Machine-readable outcome of a sync run.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub new_versions: usize,
    pub staged: Vec<StagedBuild>,
}

/// Runs a sync: plan against `state`, download new builds, stage the merged
/// index. Fails fast on the first download or digest error.
///
/// # Errors
/// Returns an error on catalog fetch/parse failure, download failure, digest
/// mismatch, or IO error.
pub fn sync(config: &Config, mut state: Index, out_dir: &Path, now: &str) -> Result<Summary> {
    let client = download::client()?;
    let upstream = config.mirror.upstream.trim_end_matches('/');

    let catalog_url = format!("{upstream}/{CATALOG_PATH}");
    let catalog_body = client
        .get(&catalog_url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .with_context(|| format!("GET {catalog_url}"))?
        .text()
        .with_context(|| format!("read {catalog_url}"))?;
    let catalog: Catalog =
        serde_json::from_str(&catalog_body).with_context(|| format!("parse {catalog_url}"))?;
    catalog.validate()?;

    let mut staged = Vec::new();
    for (key_str, product) in &catalog.products {
        let key = ProductKey::parse(key_str)?;
        if !config.wants(&key) {
            continue;
        }

        let Some((stamp, build)) = product.latest_version() else {
            eprintln!("warning: {key_str} has no versions; skipping");
            continue;
        };
        let Some(rootfs) = build.squashfs() else {
            eprintln!("warning: {key_str} {stamp} has no squashfs item; skipping");
            continue;
        };

        let meta = StreamMeta {
            distro: key.distro.clone(),
            release: key.release.clone(),
            arch: key.arch.clone(),
            variant: key.variant.clone(),
        };
        let stream = stream_name(&meta);
        let version = normalize_version(stamp);
        if state.contains(&stream, &version) {
            continue;
        }

        let build_dir = out_dir.join(&stream).join(&version);
        let rootfs_url = format!("{upstream}/{}", rootfs.path);
        println!("mirroring {stream}@{version} ({} bytes)...", rootfs.size);
        download::fetch_verified(
            &client,
            &rootfs_url,
            &build_dir.join("rootfs.squashfs"),
            &rootfs.sha256,
            rootfs.size,
        )?;

        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            name: stream.clone(),
            version: version.clone(),
            distro: key.distro.clone(),
            release: key.release.clone(),
            release_title: product
                .release_title
                .clone()
                .unwrap_or_else(|| key.release.clone()),
            arch: key.arch.clone(),
            variant: key.variant.clone(),
            upstream: UpstreamRef {
                server: upstream.to_string(),
                product: key_str.clone(),
                version: stamp.clone(),
            },
            rootfs: RootfsFile {
                path: "rootfs.squashfs".to_string(),
                format: "squashfs".to_string(),
                size: rootfs.size,
                sha256: rootfs.sha256.clone(),
            },
        };
        write_json(&build_dir.join("manifest.json"), &manifest)?;

        state.add_version(&stream, &meta, &version, config.mirror.keep_versions);
        staged.push(StagedBuild {
            stream,
            version,
            bytes: rootfs.size,
        });
    }

    state.updated_at = Some(now.to_string());
    write_json(&out_dir.join("index.json"), &state)?;

    Ok(Summary {
        new_versions: staged.len(),
        staged,
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs_err::create_dir_all(parent)?;
    }
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    fs_err::write(path, json)?;
    Ok(())
}
