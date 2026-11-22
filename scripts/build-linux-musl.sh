#!/usr/bin/env bash
set -euo pipefail

# Build a zero-dependency static Linux musl single binary (embeds web/dist)
# Supports Debian, Ubuntu, Alpine Linux (x86_64 / aarch64)
TARGET="${1:-x86_64-unknown-linux-musl}"
OUT_DIR="${2:-./dist-release}"
VERSION="$(grep '^version = ' Cargo.toml | head -n 1 | cut -d'"' -f2)"

echo "=== [1/4] Building Vue 3 frontend (web/dist) ==="
(cd web && pnpm install --frozen-lockfile && pnpm build)

echo "=== [2/4] Ensuring Rust musl target: $TARGET ==="
rustup target add "$TARGET"

echo "=== [3/4] Compiling static single binary ($TARGET, release opt-level=z LTO) ==="
cargo build --release --target "$TARGET" -p zmpanel-server

echo "=== [4/4] Packaging release artifact & SHA256 manifest ==="
mkdir -p "$OUT_DIR"
BIN_SRC="target/$TARGET/release/zmpanel"
BIN_DST="$OUT_DIR/zmpanel-${VERSION}-${TARGET}"
cp "$BIN_SRC" "$BIN_DST"
chmod +x "$BIN_DST"

SHA256="$(sha256sum "$BIN_DST" | awk '{print $1}')"
echo "$SHA256  $(basename "$BIN_DST")" > "${BIN_DST}.sha256"

cat > "$OUT_DIR/release-manifest-${TARGET}.json" <<EOF
{
  "version": "${VERSION}",
  "changelog": "zmpanel v${VERSION} (${TARGET} static musl single binary)",
  "download_url": "https://YOUR_MIRROR_OR_RELEASE_URL/$(basename "$BIN_DST")",
  "sha256": "${SHA256}"
}
EOF

ls -lh "$BIN_DST"
echo "SHA256: $SHA256"
echo "Done! Artifacts in $OUT_DIR"
