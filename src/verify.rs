//! Post-sync verification of a staging directory.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::download::sha256_file;
use crate::index::{Index, Manifest};

/// Outcome of a verification pass.
#[derive(Debug)]
pub struct Report {
    /// Builds present locally and verified against their manifests.
    pub verified: usize,
    /// Index entries with no local files (retained history) — expected.
    pub retained: usize,
}

/// Verifies a staging directory produced by [`crate::sync::sync`]:
/// every index entry whose files exist locally must match its manifest's
/// digest and size, `latest` must resolve for every stream, and every local
/// manifest must be reachable from the index.
///
/// # Errors
/// Returns an error on any inconsistency.
pub fn verify(dir: &Path) -> Result<Report> {
    let index_path = dir.join("index.json");
    let index: Index = serde_json::from_str(&fs_err::read_to_string(&index_path)?)
        .with_context(|| format!("parse {}", index_path.display()))?;

    let mut verified = 0;
    let mut retained = 0;
    for (name, stream) in &index.images {
        if !stream.versions.contains_key(&stream.latest) {
            bail!(
                "stream '{name}': latest '{}' is not a listed version",
                stream.latest
            );
        }
        for (version, entry) in &stream.versions {
            let expected_manifest = format!("{name}/{version}/manifest.json");
            if entry.manifest != expected_manifest {
                bail!(
                    "stream '{name}': version '{version}' points at '{}', expected '{expected_manifest}'",
                    entry.manifest
                );
            }
            let manifest_path = dir.join(&entry.manifest);
            if !manifest_path.exists() {
                retained += 1;
                continue;
            }
            let manifest: Manifest = serde_json::from_str(&fs_err::read_to_string(&manifest_path)?)
                .with_context(|| format!("parse {}", manifest_path.display()))?;
            if manifest.name != *name || manifest.version != *version {
                bail!(
                    "{}: manifest identifies as {}@{}, expected {name}@{version}",
                    manifest_path.display(),
                    manifest.name,
                    manifest.version
                );
            }
            let rootfs_path = manifest_path
                .parent()
                .expect("manifest has a parent directory")
                .join(&manifest.rootfs.path);
            let (sha256, size) = sha256_file(&rootfs_path)?;
            if sha256 != manifest.rootfs.sha256 || size != manifest.rootfs.size {
                bail!(
                    "{}: expected {} bytes sha256:{}, got {size} bytes sha256:{sha256}",
                    rootfs_path.display(),
                    manifest.rootfs.size,
                    manifest.rootfs.sha256
                );
            }
            verified += 1;
        }
    }

    // Every locally staged manifest must be reachable from the index —
    // an orphan means the index merge lost an entry.
    for stream_dir in fs_err::read_dir(dir)? {
        let stream_dir = stream_dir?;
        if !stream_dir.file_type()?.is_dir() {
            continue;
        }
        let stream = stream_dir.file_name().to_string_lossy().into_owned();
        for version_dir in fs_err::read_dir(stream_dir.path())? {
            let version_dir = version_dir?;
            let version = version_dir.file_name().to_string_lossy().into_owned();
            if !version_dir.path().join("manifest.json").exists() {
                continue;
            }
            let listed = index
                .images
                .get(&stream)
                .is_some_and(|s| s.versions.contains_key(&version));
            if !listed {
                bail!("staged build {stream}/{version} is not listed in index.json");
            }
        }
    }

    Ok(Report { verified, retained })
}
