//! Mirror pipeline for ArcBox Linux machine images.
//!
//! Mirrors a curated subset of the Linux Containers (Incus) community image
//! server into the `linux/` namespace of the `arcboxcdn-image` bucket
//! (`image.arcboxcdn.com`), using the same index/manifest layout as the
//! `darwin/` namespace published by `macos-runner-image-builder`.

pub mod config;
pub mod download;
pub mod index;
pub mod sync;
pub mod upstream;
pub mod verify;
