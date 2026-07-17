//! The published CDN schema: a mutable `index.json` pointer plus one immutable
//! `manifest.json` per version, matching the layout the `darwin/` namespace of
//! `image.arcboxcdn.com` already uses (consumed by `arcbox-core`'s
//! `RemoteIndex`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const INDEX_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// The discovery index (`index.json` at the namespace root).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub images: BTreeMap<String, Stream>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            schema_version: INDEX_SCHEMA_VERSION,
            updated_at: None,
            images: BTreeMap::new(),
        }
    }
}

/// One image stream (`{distro}-{release}-{arch}`) in the index.
///
/// `distro`/`release`/`arch`/`variant` are denormalized here so clients can
/// render a distro picker from the index alone; the base `RemoteIndex`
/// consumer ignores them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stream {
    pub distro: String,
    pub release: String,
    pub arch: String,
    pub variant: String,
    pub latest: String,
    pub versions: BTreeMap<String, VersionEntry>,
}

/// One published version of a stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    /// Manifest path relative to the index location.
    pub manifest: String,
}

/// A published image manifest (one immutable version of a stream).
#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    /// Stream name.
    pub name: String,
    /// Version label (normalized upstream build stamp).
    pub version: String,
    pub distro: String,
    pub release: String,
    pub release_title: String,
    pub arch: String,
    pub variant: String,
    /// Where this build was mirrored from.
    pub upstream: UpstreamRef,
    /// The rootfs image, stored next to the manifest.
    pub rootfs: RootfsFile,
}

/// Provenance of a mirrored build.
#[derive(Debug, Serialize, Deserialize)]
pub struct UpstreamRef {
    pub server: String,
    /// Upstream product key (`distro:release:arch:variant`).
    pub product: String,
    /// Verbatim upstream build stamp (`YYYYMMDD_HH:MM`).
    pub version: String,
}

/// A rootfs file entry in a manifest.
#[derive(Debug, Serialize, Deserialize)]
pub struct RootfsFile {
    /// File name relative to the manifest's directory.
    pub path: String,
    /// Image format (`squashfs`).
    pub format: String,
    /// Size in bytes.
    pub size: u64,
    /// Hex SHA-256 of the file.
    pub sha256: String,
}

/// Stream identity fields carried into the index.
#[derive(Debug, Clone)]
pub struct StreamMeta {
    pub distro: String,
    pub release: String,
    pub arch: String,
    pub variant: String,
}

/// Stream name for a product: `{distro}-{release}-{arch}`, with a `-{variant}`
/// suffix for non-default variants. Sanitized to the `[a-z0-9.-]` charset
/// image references accept.
#[must_use]
pub fn stream_name(meta: &StreamMeta) -> String {
    let mut name = format!(
        "{}-{}-{}",
        sanitize(&meta.distro),
        sanitize(&meta.release),
        sanitize(&meta.arch)
    );
    if meta.variant != "default" {
        name.push('-');
        name.push_str(&sanitize(&meta.variant));
    }
    name
}

fn sanitize(s: &str) -> String {
    s.to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Path- and reference-safe version label: upstream build stamps keep their
/// ordering but drop the `:` (`20260716_09:47` → `20260716_0947`).
#[must_use]
pub fn normalize_version(stamp: &str) -> String {
    stamp.chars().filter(|c| *c != ':').collect()
}

impl Index {
    /// Inserts a version into a stream (creating the stream when new),
    /// recomputes `latest`, and trims the listed versions to the newest
    /// `keep`. Trimmed objects stay on the CDN; they only leave the index.
    pub fn add_version(&mut self, name: &str, meta: &StreamMeta, version: &str, keep: usize) {
        let stream = self
            .images
            .entry(name.to_string())
            .or_insert_with(|| Stream {
                distro: meta.distro.clone(),
                release: meta.release.clone(),
                arch: meta.arch.clone(),
                variant: meta.variant.clone(),
                latest: String::new(),
                versions: BTreeMap::new(),
            });
        stream.versions.insert(
            version.to_string(),
            VersionEntry {
                manifest: format!("{name}/{version}/manifest.json"),
            },
        );
        while stream.versions.len() > keep {
            let oldest = stream
                .versions
                .keys()
                .next()
                .expect("non-empty after insert")
                .clone();
            stream.versions.remove(&oldest);
        }
        // The newest listed version; the insert above can never be trimmed
        // away because trimming removes from the oldest end.
        stream.latest = stream
            .versions
            .keys()
            .next_back()
            .expect("non-empty after insert")
            .clone();
    }

    /// Whether `stream` already lists `version`.
    #[must_use]
    pub fn contains(&self, stream: &str, version: &str) -> bool {
        self.images
            .get(stream)
            .is_some_and(|s| s.versions.contains_key(version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> StreamMeta {
        StreamMeta {
            distro: "ubuntu".into(),
            release: "noble".into(),
            arch: "arm64".into(),
            variant: "default".into(),
        }
    }

    #[test]
    fn stream_name_includes_variant_only_when_not_default() {
        assert_eq!(stream_name(&meta()), "ubuntu-noble-arm64");
        let cloud = StreamMeta {
            variant: "cloud".into(),
            ..meta()
        };
        assert_eq!(stream_name(&cloud), "ubuntu-noble-arm64-cloud");
    }

    #[test]
    fn stream_name_sanitizes_to_reference_charset() {
        let odd = StreamMeta {
            distro: "OpenEuler".into(),
            release: "20.03_LTS".into(),
            ..meta()
        };
        assert_eq!(stream_name(&odd), "openeuler-20.03-lts-arm64");
    }

    #[test]
    fn normalize_version_drops_colon_and_keeps_order() {
        assert_eq!(normalize_version("20260716_09:47"), "20260716_0947");
        assert_eq!(normalize_version("20260716_0947"), "20260716_0947");
        let (a, b) = ("20260101_23:59", "20260102_00:01");
        assert!(normalize_version(a) < normalize_version(b));
    }

    #[test]
    fn add_version_tracks_latest_and_trims_oldest() {
        let mut index = Index::default();
        index.add_version("ubuntu-noble-arm64", &meta(), "20260101_0101", 2);
        index.add_version("ubuntu-noble-arm64", &meta(), "20260103_0101", 2);
        // Out-of-order insert must not disturb `latest`.
        index.add_version("ubuntu-noble-arm64", &meta(), "20260102_0101", 2);

        let stream = &index.images["ubuntu-noble-arm64"];
        assert_eq!(stream.latest, "20260103_0101");
        assert_eq!(
            stream.versions.keys().collect::<Vec<_>>(),
            ["20260102_0101", "20260103_0101"],
            "oldest trimmed at keep=2"
        );
        assert_eq!(
            stream.versions["20260103_0101"].manifest,
            "ubuntu-noble-arm64/20260103_0101/manifest.json"
        );
        assert!(index.contains("ubuntu-noble-arm64", "20260102_0101"));
        assert!(!index.contains("ubuntu-noble-arm64", "20260101_0101"));
    }

    #[test]
    fn index_round_trips_and_tolerates_missing_optionals() {
        let json = r#"{"schema_version": 1, "images": {}}"#;
        let index: Index = serde_json::from_str(json).unwrap();
        assert!(index.updated_at.is_none());
        let out = serde_json::to_string(&index).unwrap();
        assert!(!out.contains("updated_at"), "None must not serialize");
    }
}
