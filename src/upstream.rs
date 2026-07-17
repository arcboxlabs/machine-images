//! Upstream simplestreams (`products:1.0`) catalog types.
//!
//! Only the fields this mirror consumes are modeled; the catalog carries more
//! (aliases, deltas, combined digests) that we deliberately ignore. Product
//! identity comes from the product *key* (`distro:release:arch:variant`) —
//! the body's `os` field is a display name.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::Deserialize;

/// Catalog location relative to the upstream server root.
pub const CATALOG_PATH: &str = "streams/v1/images.json";

/// The upstream image catalog.
#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub format: String,
    #[serde(default)]
    pub products: BTreeMap<String, Product>,
}

/// One product (a distro/release/arch/variant tuple).
#[derive(Debug, Deserialize)]
pub struct Product {
    #[serde(default)]
    pub release_title: Option<String>,
    #[serde(default)]
    pub versions: BTreeMap<String, Version>,
}

/// One build of a product.
#[derive(Debug, Deserialize)]
pub struct Version {
    #[serde(default)]
    pub items: BTreeMap<String, Item>,
}

/// One downloadable file of a build.
#[derive(Debug, Deserialize)]
pub struct Item {
    pub ftype: String,
    pub sha256: String,
    pub size: u64,
    pub path: String,
}

/// A parsed product key: `distro:release:arch:variant`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductKey {
    pub distro: String,
    pub release: String,
    pub arch: String,
    pub variant: String,
}

impl ProductKey {
    /// Parses a `distro:release:arch:variant` product key.
    ///
    /// # Errors
    /// Returns an error when the key does not have exactly four segments.
    pub fn parse(key: &str) -> Result<Self> {
        let parts: Vec<&str> = key.split(':').collect();
        let [distro, release, arch, variant] = parts.as_slice() else {
            bail!("product key '{key}' is not distro:release:arch:variant");
        };
        Ok(Self {
            distro: (*distro).to_string(),
            release: (*release).to_string(),
            arch: (*arch).to_string(),
            variant: (*variant).to_string(),
        })
    }
}

impl Catalog {
    /// Rejects catalogs in a format other than `products:1.0`.
    ///
    /// # Errors
    /// Returns an error on an unknown format.
    pub fn validate(&self) -> Result<()> {
        if self.format != "products:1.0" {
            bail!("unsupported catalog format '{}'", self.format);
        }
        Ok(())
    }
}

impl Product {
    /// The newest build. Upstream stamps are fixed-width `YYYYMMDD_HH:MM`, so
    /// the lexicographically greatest key is the newest.
    #[must_use]
    pub fn latest_version(&self) -> Option<(&String, &Version)> {
        self.versions.last_key_value()
    }
}

impl Version {
    /// The squashfs rootfs item of this build, if published.
    #[must_use]
    pub fn squashfs(&self) -> Option<&Item> {
        self.items.values().find(|i| i.ftype == "squashfs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_key_parses_four_segments() {
        let key = ProductKey::parse("ubuntu:noble:arm64:default").unwrap();
        assert_eq!(key.distro, "ubuntu");
        assert_eq!(key.release, "noble");
        assert_eq!(key.arch, "arm64");
        assert_eq!(key.variant, "default");

        assert!(ProductKey::parse("ubuntu:noble:arm64").is_err());
        assert!(ProductKey::parse("a:b:c:d:e").is_err());
    }

    #[test]
    fn latest_version_is_greatest_stamp() {
        let catalog: Catalog = serde_json::from_str(
            r#"{
                "format": "products:1.0",
                "products": {
                    "ubuntu:noble:arm64:default": {
                        "release_title": "24.04",
                        "versions": {
                            "20260101_01:01": {"items": {}},
                            "20260102_01:01": {"items": {}},
                            "20251231_23:59": {"items": {}}
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        catalog.validate().unwrap();

        let product = &catalog.products["ubuntu:noble:arm64:default"];
        assert_eq!(product.latest_version().unwrap().0, "20260102_01:01");
    }

    #[test]
    fn squashfs_item_found_by_ftype() {
        let version: Version = serde_json::from_str(
            r#"{
                "items": {
                    "root.tar.xz": {"ftype": "root.tar.xz", "sha256": "a", "size": 1, "path": "p1"},
                    "root.squashfs": {"ftype": "squashfs", "sha256": "b", "size": 2, "path": "p2"}
                }
            }"#,
        )
        .unwrap();
        assert_eq!(version.squashfs().unwrap().path, "p2");

        let empty = Version {
            items: BTreeMap::new(),
        };
        assert!(empty.squashfs().is_none());
    }

    #[test]
    fn validate_rejects_unknown_format() {
        let catalog = Catalog {
            format: "products:2.0".to_string(),
            products: BTreeMap::new(),
        };
        assert!(catalog.validate().is_err());
    }
}
