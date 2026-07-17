//! End-to-end tests: a local HTTP server plays upstream, the sync runs the
//! full plan→download→verify→stage pipeline against it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use machine_images::config::Config;
use machine_images::index::Index;
use machine_images::{sync, verify};
use sha2::{Digest, Sha256};

/// Serves a fixed path→body map on an ephemeral port; returns the base URL.
fn serve(routes: HashMap<String, Vec<u8>>) -> String {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let routes = Arc::new(routes);
    thread::spawn(move || {
        for request in server.incoming_requests() {
            let path = request.url().to_string();
            match routes.get(&path) {
                Some(body) => {
                    let response = tiny_http::Response::from_data(body.clone());
                    let _ = request.respond(response);
                }
                None => {
                    let _ = request.respond(tiny_http::Response::empty(404));
                }
            }
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Builds a products:1.0 catalog + blob routes for `(product_key, stamp, body)`.
fn upstream_routes(builds: &[(&str, &str, &[u8])]) -> HashMap<String, Vec<u8>> {
    let mut products = serde_json::Map::new();
    let mut routes = HashMap::new();
    for (key, stamp, body) in builds {
        let segments: Vec<&str> = key.split(':').collect();
        let blob_path = format!(
            "images/{}/{}/{}/{}/{stamp}/rootfs.squashfs",
            segments[0], segments[1], segments[2], segments[3]
        );
        routes.insert(format!("/{blob_path}"), body.to_vec());

        let product = products
            .entry((*key).to_string())
            .or_insert_with(|| {
                serde_json::json!({
                    "release_title": segments[1],
                    "versions": {}
                })
            })
            .as_object_mut()
            .unwrap();
        product["versions"][stamp] = serde_json::json!({
            "items": {
                "root.squashfs": {
                    "ftype": "squashfs",
                    "sha256": sha256_hex(body),
                    "size": body.len(),
                    "path": blob_path,
                }
            }
        });
    }
    let catalog = serde_json::json!({
        "format": "products:1.0",
        "content_id": "images",
        "datatype": "image-downloads",
        "products": products,
    });
    routes.insert(
        "/streams/v1/images.json".to_string(),
        serde_json::to_vec(&catalog).unwrap(),
    );
    routes
}

fn config_for(upstream: &str, distros: &str, keep_versions: usize) -> Config {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mirror.toml");
    fs_err::write(
        &path,
        format!(
            r#"
            [mirror]
            upstream = "{upstream}"
            arches = ["arm64"]
            variants = ["default"]
            keep_versions = {keep_versions}
            {distros}
            "#
        ),
    )
    .unwrap();
    Config::load(&path).unwrap()
}

const TWO_DISTROS: &str = r#"
    [[distros]]
    name = "ubuntu"
    [[distros]]
    name = "alpine"
"#;

fn read_index(dir: &Path) -> Index {
    serde_json::from_str(&fs_err::read_to_string(dir.join("index.json")).unwrap()).unwrap()
}

#[test]
fn bootstrap_sync_stages_builds_and_index() {
    let base = serve(upstream_routes(&[
        ("ubuntu:noble:arm64:default", "20260101_01:01", b"ubuntu-v1"),
        ("alpine:3.23:arm64:default", "20260101_02:02", b"alpine-v1"),
        ("gentoo:current:arm64:default", "20260101_03:03", b"gentoo"),
    ]));
    let config = config_for(&base, TWO_DISTROS, 4);
    let out = tempfile::tempdir().unwrap();

    let summary = sync::sync(
        &config,
        Index::default(),
        out.path(),
        "2026-07-17T00:00:00Z",
    )
    .unwrap();
    assert_eq!(summary.new_versions, 2, "gentoo is not configured");

    let rootfs = out
        .path()
        .join("ubuntu-noble-arm64/20260101_0101/rootfs.squashfs");
    assert_eq!(fs_err::read(&rootfs).unwrap(), b"ubuntu-v1");

    let index = read_index(out.path());
    assert_eq!(index.updated_at.as_deref(), Some("2026-07-17T00:00:00Z"));
    let stream = &index.images["ubuntu-noble-arm64"];
    assert_eq!(stream.latest, "20260101_0101");
    assert_eq!(stream.distro, "ubuntu");
    assert_eq!(
        stream.versions["20260101_0101"].manifest,
        "ubuntu-noble-arm64/20260101_0101/manifest.json"
    );
    assert!(index.images.contains_key("alpine-3.23-arm64"));
    assert!(!index.images.contains_key("gentoo-current-arm64"));

    let report = verify::verify(out.path()).unwrap();
    assert_eq!(report.verified, 2);
    assert_eq!(report.retained, 0);
}

#[test]
fn incremental_sync_downloads_only_new_builds_and_retains_dropped_streams() {
    // First sync: ubuntu + alpine.
    let base = serve(upstream_routes(&[
        ("ubuntu:noble:arm64:default", "20260101_01:01", b"ubuntu-v1"),
        ("alpine:3.23:arm64:default", "20260101_02:02", b"alpine-v1"),
    ]));
    let config = config_for(&base, TWO_DISTROS, 4);
    let out1 = tempfile::tempdir().unwrap();
    sync::sync(&config, Index::default(), out1.path(), "t1").unwrap();
    let state = read_index(out1.path());

    // Second sync: ubuntu has a newer build; alpine unchanged; alpine also
    // dropped from the config.
    let base2 = serve(upstream_routes(&[
        ("ubuntu:noble:arm64:default", "20260101_01:01", b"ubuntu-v1"),
        ("ubuntu:noble:arm64:default", "20260102_01:01", b"ubuntu-v2"),
    ]));
    let ubuntu_only = "[[distros]]\nname = \"ubuntu\"";
    let config2 = config_for(&base2, ubuntu_only, 4);
    let out2 = tempfile::tempdir().unwrap();
    let summary = sync::sync(&config2, state, out2.path(), "t2").unwrap();

    assert_eq!(summary.new_versions, 1);
    assert_eq!(summary.staged[0].stream, "ubuntu-noble-arm64");
    assert_eq!(summary.staged[0].version, "20260102_0101");

    let index = read_index(out2.path());
    let ubuntu = &index.images["ubuntu-noble-arm64"];
    assert_eq!(ubuntu.latest, "20260102_0101");
    assert_eq!(ubuntu.versions.len(), 2, "old version stays listed");
    let alpine = &index.images["alpine-3.23-arm64"];
    assert_eq!(alpine.latest, "20260101_0202", "dropped stream retained");

    // Only the new build exists locally; retained entries are index-only.
    let report = verify::verify(out2.path()).unwrap();
    assert_eq!(report.verified, 1);
    assert_eq!(report.retained, 2);

    // Re-running against the new state is a no-op.
    let state2 = read_index(out2.path());
    let out3 = tempfile::tempdir().unwrap();
    let summary = sync::sync(&config2, state2, out3.path(), "t3").unwrap();
    assert_eq!(summary.new_versions, 0);
}

#[test]
fn digest_mismatch_fails_and_leaves_no_file() {
    let mut routes = upstream_routes(&[(
        "ubuntu:noble:arm64:default",
        "20260101_01:01",
        b"correct-bytes".as_slice(),
    )]);
    // Corrupt the blob after the catalog was built.
    routes.insert(
        "/images/ubuntu/noble/arm64/default/20260101_01:01/rootfs.squashfs".to_string(),
        b"tampered-bytes!".to_vec(),
    );
    let base = serve(routes);
    let config = config_for(&base, "[[distros]]\nname = \"ubuntu\"", 4);
    let out = tempfile::tempdir().unwrap();

    let err = sync::sync(&config, Index::default(), out.path(), "t").unwrap_err();
    assert!(
        err.to_string().contains("sha256"),
        "unexpected error: {err}"
    );

    let build_dir = out.path().join("ubuntu-noble-arm64/20260101_0101");
    assert!(!build_dir.join("rootfs.squashfs").exists());
    assert!(!build_dir.join("rootfs.squashfs.partial").exists());
}

#[test]
fn verify_catches_corrupted_staging() {
    let base = serve(upstream_routes(&[(
        "ubuntu:noble:arm64:default",
        "20260101_01:01",
        b"ubuntu-v1",
    )]));
    let config = config_for(&base, "[[distros]]\nname = \"ubuntu\"", 4);
    let out = tempfile::tempdir().unwrap();
    sync::sync(&config, Index::default(), out.path(), "t").unwrap();

    let rootfs = out
        .path()
        .join("ubuntu-noble-arm64/20260101_0101/rootfs.squashfs");
    fs_err::write(&rootfs, b"flipped").unwrap();
    assert!(verify::verify(out.path()).is_err());
}

/// Mirrors one real (tiny) busybox build from images.linuxcontainers.org.
/// Network-dependent; run with `cargo test -- --ignored`.
#[test]
#[ignore = "downloads ~1 MB from images.linuxcontainers.org"]
fn live_mirror_busybox() {
    let config = config_for(
        "https://images.linuxcontainers.org",
        "[[distros]]\nname = \"busybox\"",
        1,
    );
    let out = tempfile::tempdir().unwrap();
    let summary = sync::sync(&config, Index::default(), out.path(), "live").unwrap();
    assert!(summary.new_versions >= 1);
    let report = verify::verify(out.path()).unwrap();
    assert_eq!(report.verified, summary.new_versions);
}
