#!/usr/bin/env bash
# Packs a built LisekMovie into the release archive that install-linux.sh and
# update-linux.sh download (method "release"):
#
#   lisekmovie-<version>-linux-<arch>.tar.gz
#     lisekmovie-<version>-linux-<arch>/
#       bin/lisek-server              release build of the backend
#       web/                          SvelteKit build + server.js + production node_modules
#       .env.example  VERSION  LICENSE  README.md  install-linux.sh  update-linux.sh
#   lisekmovie-<version>-linux-<arch>.tar.gz.sha256
#
# Build first (from the folder that holds LisekMovie-app and the plugins):
#   (cd LisekMovie-app/LisekMovie-Backend && SQLX_OFFLINE=true cargo build --release --locked -p lisek-server)
#   (cd LisekMovie-app/LisekMovie-Frontend && npm ci && npm run build)
# then:
#   bash LisekMovie-app/tools/package-release.sh --version v1.0.0-beta --out dist
#
# Options: --version TAG (required), --arch x86_64|aarch64 (default: this
# machine), --binary PATH (default: LisekMovie-Backend/target/release/lisek-server),
# --out DIR (default: dist).

set -Eeuo pipefail

APP=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
VERSION=""
ARCH=""
BINARY="$APP/LisekMovie-Backend/target/release/lisek-server"
OUT="dist"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) VERSION=$2; shift 2 ;;
        --arch) ARCH=$2; shift 2 ;;
        --binary) BINARY=$2; shift 2 ;;
        --out) OUT=$2; shift 2 ;;
        -h | --help) sed -n '2,21p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

[[ -n "$VERSION" ]] || { echo "--version is required" >&2; exit 2; }
if [[ -z "$ARCH" ]]; then
    case "$(uname -m)" in
        x86_64 | amd64) ARCH=x86_64 ;;
        aarch64 | arm64) ARCH=aarch64 ;;
        *) echo "unsupported architecture: $(uname -m)" >&2; exit 2 ;;
    esac
fi

FRONTEND="$APP/LisekMovie-Frontend"
[[ -x "$BINARY" ]] || { echo "missing backend binary: $BINARY" >&2; exit 1; }
[[ -f "$FRONTEND/build/index.js" ]] || { echo "missing web build: run npm run build in $FRONTEND" >&2; exit 1; }

NAME="lisekmovie-${VERSION#v}-linux-${ARCH}"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
ROOT="$WORK/$NAME"

install -d "$ROOT/bin" "$ROOT/web"
install -m 0755 "$BINARY" "$ROOT/bin/lisek-server"
cp -a "$FRONTEND/build" "$FRONTEND/server.js" "$FRONTEND/package.json" "$FRONTEND/package-lock.json" "$ROOT/web/"
(cd "$ROOT/web" && npm ci --omit=dev --ignore-scripts --no-audit --no-fund --loglevel=error)
install -m 0644 "$APP/.env.example" "$APP/LICENSE" "$APP/README.md" "$ROOT/"
install -m 0755 "$APP/install-linux.sh" "$APP/update-linux.sh" "$ROOT/"
printf '%s\n' "$VERSION" >"$ROOT/VERSION"

# Reproducible-ish archive: sorted entries, no owner names.
tar --sort=name --owner=0 --group=0 --numeric-owner -C "$WORK" -czf "$OUT/$NAME.tar.gz" "$NAME"
(cd "$OUT" && sha256sum "$NAME.tar.gz" >"$NAME.tar.gz.sha256")
echo "$OUT/$NAME.tar.gz"
cat "$OUT/$NAME.tar.gz.sha256"
