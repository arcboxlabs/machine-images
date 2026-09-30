# ArcBox Machine Images

Mirror pipeline for ArcBox Linux machine images: a curated subset of the
[Linux Containers community image server](https://images.linuxcontainers.org)
(Incus images, squashfs rootfs, no kernel — ArcBox machines boot the
[`arcboxlabs/kernel`](https://github.com/arcboxlabs/kernel) kernel), re-hosted
on ArcBox's CDN with verified digests and add-only retention.

## CDN layout

Published to the `linux/` namespace of the `arcboxcdn-image` bucket, fronted
by `image.arcboxcdn.com` — the same index/manifest scheme as the `darwin/`
namespace (macOS runner images), consumable by `arcbox-core`'s `RemoteIndex`:

```
linux/
├── index.json                          # mutable pointer, max-age=60
└── {stream}/{version}/                 # immutable, max-age=1y
    ├── manifest.json
    └── rootfs.squashfs
```

- **stream** — `{distro}-{release}-{arch}` (e.g. `ubuntu-noble-arm64`), plus a
  `-{variant}` suffix for non-default variants.
- **version** — the upstream build stamp with the `:` dropped
  (`20260716_09:47` → `20260716_0947`).
- `index.json` lists every stream with its `latest` pointer and the newest
  `keep_versions` builds; each entry denormalizes `distro`/`release`/`arch`
  so clients can render a picker from the index alone.
- `manifest.json` records the rootfs digest/size and full upstream
  provenance (server, product key, verbatim build stamp).

## Retention

Add-only, following the OrbStack precedent: a sync never deletes published
objects. Builds that upstream prunes stay available for pinned references;
streams removed from `mirror.toml` stop syncing but remain in the index so
existing machines can always re-resolve their image.

## Configuration

`mirror.toml` picks what to mirror:

```toml
[mirror]
upstream = "https://images.linuxcontainers.org"
arches = ["arm64", "amd64"]
variants = ["default"]
keep_versions = 4

[[distros]]
name = "ubuntu"
# releases = ["noble"]   # optional pin; omitted mirrors all upstream releases
```

## Usage

```bash
# Mirror new builds into dist/ against the published index
curl -fsSo state.json https://image.arcboxcdn.com/linux/index.json
cargo run --release -- sync --config mirror.toml --output dist --state-file state.json

# Re-verify staged artifacts against their manifests
cargo run --release -- verify --dir dist
```

Every downloaded rootfs is verified against the upstream catalog's SHA-256
before staging; `verify` re-hashes the staged files and cross-checks the
merged index.

## CI

- `ci.yml` — fmt, clippy, tests, plus a live smoke test that mirrors one tiny
  busybox build from the real upstream.
- `sync.yml` — daily cron (plus `mirror.toml` pushes and manual dispatch):
  builds the tool, plans against the live index, stages new builds, and
  publishes via [`arcboxlabs/actions/r2-publish`](https://github.com/arcboxlabs/actions).
  Uploads are ordered blobs-first, index-last, so a fresh index never points
  at missing objects. Seeding an empty CDN: dispatch it with
  `keep_versions = 1` so the first run mirrors only the newest build per
  stream; the daily syncs fill the index back up to `keep_versions`.

Required repository configuration (an R2 API token with Object Read & Write
on the `arcboxcdn-image` bucket):

| Name | Kind | Value |
| --- | --- | --- |
| `R2_ACCOUNT_ID` | variable | Cloudflare account ID that owns the bucket |
| `R2_ACCESS_KEY_ID` | secret | R2 S3 Access Key ID |
| `R2_SECRET_ACCESS_KEY` | secret | R2 S3 Secret Access Key |
