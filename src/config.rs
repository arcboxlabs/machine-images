//! Mirror configuration (`mirror.toml`).

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::upstream::ProductKey;

/// Top-level mirror configuration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub mirror: MirrorSettings,
    #[serde(default)]
    pub distros: Vec<DistroFilter>,
}

/// Global mirror settings.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorSettings {
    /// Upstream simplestreams server.
    pub upstream: String,
    /// Architectures to mirror, in upstream naming (`arm64`, `amd64`).
    pub arches: Vec<String>,
    /// Image variants to mirror (`default`, `cloud`).
    pub variants: Vec<String>,
    /// Number of newest versions listed per stream in the published index.
    /// Older objects stay on the CDN; they only drop out of the index.
    pub keep_versions: usize,
}

/// One distro to mirror.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistroFilter {
    /// Distro id as it appears in upstream product keys (e.g. `ubuntu`).
    pub name: String,
    /// Releases to mirror; omitted mirrors every release upstream publishes.
    pub releases: Option<Vec<String>>,
}

impl Config {
    /// Loads and validates a configuration file.
    ///
    /// # Errors
    /// Returns an error when the file cannot be read, parsed, or fails
    /// validation.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs_err::read_to_string(path)?;
        let config: Self =
            toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.mirror.keep_versions == 0 {
            bail!("mirror.keep_versions must be at least 1");
        }
        if self.mirror.arches.is_empty() {
            bail!("mirror.arches must not be empty");
        }
        if self.mirror.variants.is_empty() {
            bail!("mirror.variants must not be empty");
        }
        if self.distros.is_empty() {
            bail!("at least one [[distros]] entry is required");
        }
        Ok(())
    }

    /// Whether a product from the upstream catalog is part of this mirror.
    #[must_use]
    pub fn wants(&self, key: &ProductKey) -> bool {
        self.mirror.arches.contains(&key.arch)
            && self.mirror.variants.contains(&key.variant)
            && self.distros.iter().any(|d| {
                d.name == key.distro
                    && d.releases
                        .as_ref()
                        .is_none_or(|rs| rs.contains(&key.release))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml_str: &str) -> Config {
        let config: Config = toml::from_str(toml_str).unwrap();
        config.validate().unwrap();
        config
    }

    const BASE: &str = r#"
        [mirror]
        upstream = "https://images.linuxcontainers.org"
        arches = ["arm64"]
        variants = ["default"]
        keep_versions = 4

        [[distros]]
        name = "ubuntu"

        [[distros]]
        name = "alpine"
        releases = ["3.23"]
    "#;

    fn key(s: &str) -> ProductKey {
        ProductKey::parse(s).unwrap()
    }

    #[test]
    fn wants_filters_by_distro_release_arch_variant() {
        let c = config(BASE);
        assert!(c.wants(&key("ubuntu:noble:arm64:default")));
        assert!(c.wants(&key("alpine:3.23:arm64:default")));
        assert!(!c.wants(&key("alpine:3.22:arm64:default")), "release pin");
        assert!(!c.wants(&key("ubuntu:noble:amd64:default")), "arch");
        assert!(!c.wants(&key("ubuntu:noble:arm64:cloud")), "variant");
        assert!(!c.wants(&key("gentoo:current:arm64:default")), "distro");
    }

    #[test]
    fn validate_rejects_empty_sections() {
        for broken in [
            BASE.replace("keep_versions = 4", "keep_versions = 0"),
            BASE.replace(r#"arches = ["arm64"]"#, "arches = []"),
            BASE.replace(r#"variants = ["default"]"#, "variants = []"),
        ] {
            let config: Config = toml::from_str(&broken).unwrap();
            assert!(config.validate().is_err(), "should reject: {broken}");
        }
    }
}
