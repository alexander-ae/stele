#!/bin/sh
# stele's installer. POSIX sh on purpose: macOS ships bash 3.2 from 2007.
#
# Downloads a prebuilt binary from the GitLab release and verifies its sha256.
# With --build it compiles from source instead, which is the path for a
# platform with no published binary (and the only one that needs Rust).
#
# It installs the binary and nothing else: no token, no service, no config.
set -eu

REPO="${STELE_REPO:-__alexander__/stele}"
PREFIX="${STELE_PREFIX:-$HOME/.local/bin}"
TAG="latest"
BUILD=0
VERIFY=1

usage() {
    cat <<'FIN'
Usage: sh scripts/install.sh [options]

  --tag <vX.Y.Z>     install that release instead of the latest one
  --prefix <dir>     where to install the binary (default ~/.local/bin)
  --build            compile from source instead of downloading a binary
  --no-verify        carry on even if the sha256 cannot be checked
  -h, --help         this

Variables: STELE_REPO (where to download from), STELE_PREFIX.

Without --build, this needs curl or wget, tar, and sha256sum or shasum.
--build needs Rust 1.89 or newer and a C compiler.
FIN
}

ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$1" >&2; }
die()  { printf '  \033[31m✗\033[0m %s\n' "$1" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --tag)       TAG="${2:?--tag needs a version}"; shift 2 ;;
        --prefix)    PREFIX="${2:?--prefix needs a directory}"; shift 2 ;;
        --build)     BUILD=1; shift ;;
        --no-verify) VERIFY=0; shift ;;
        -h|--help)   usage; exit 0 ;;
        *)           echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [ "$BUILD" -eq 1 ]; then
    # rustup may have been installed with --no-modify-path; the binary is
    # there even when the shell does not know it.
    if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
        PATH="$HOME/.cargo/bin:$PATH"
        export PATH
    fi
    command -v cargo >/dev/null 2>&1 \
        || die "cargo is not on PATH; install Rust 1.89+ or use a prebuilt release"
    command -v cc >/dev/null 2>&1 || command -v gcc >/dev/null 2>&1 \
        || die "a C compiler is needed for the bundled SQLite"
    cargo build --release --locked
    install -d "$PREFIX"
    install -m 755 target/release/stele "$PREFIX/stele"
    ok "installed $PREFIX/stele (built from source)"
    "$PREFIX/stele" --version
    exit 0
fi

SYSTEM="$(uname -s)"
ARCH="$(uname -m)"
case "$SYSTEM/$ARCH" in
    Linux/x86_64)   PLATFORM="linux-x86_64" ;;
    Darwin/arm64)   PLATFORM="macos-arm64" ;;
    Darwin/x86_64)  PLATFORM="macos-x86_64" ;;
    *)              PLATFORM="" ;;
esac

if [ -z "$PLATFORM" ]; then
    die "no prebuilt binary for $SYSTEM/$ARCH; rerun with --build if Rust is installed"
fi

command -v tar >/dev/null 2>&1 || die "tar is required"
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 --connect-timeout 20 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "curl or wget is required"
fi

ASSET="stele-$PLATFORM.tar.gz"
if [ "$TAG" = "latest" ]; then
    BASE="https://gitlab.com/$REPO/-/releases/permalink/latest/downloads"
else
    BASE="https://gitlab.com/$REPO/-/releases/$TAG/downloads"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fetch "$BASE/$ASSET" "$TMP/$ASSET" \
    || die "cannot download $BASE/$ASSET (is the release published?)"
ok "downloaded $ASSET"

if [ "$VERIFY" -eq 1 ]; then
    fetch "$BASE/$ASSET.sha256" "$TMP/$ASSET.sha256" \
        || die "the release has no .sha256 for $ASSET; refusing an unchecked download"
    if command -v sha256sum >/dev/null 2>&1; then
        ( cd "$TMP" && sha256sum -c "$ASSET.sha256" >/dev/null ) \
            || die "the download does not match its sha256"
    elif command -v shasum >/dev/null 2>&1; then
        ( cd "$TMP" && shasum -a 256 -c "$ASSET.sha256" >/dev/null ) \
            || die "the download does not match its sha256"
    else
        die "no sha256sum or shasum; rerun with --no-verify if you accept that"
    fi
    ok "sha256 verified"
else
    warn "sha256 not checked (--no-verify)"
fi

tar -xzf "$TMP/$ASSET" -C "$TMP"
[ -f "$TMP/stele" ] || die "the archive does not contain stele"

install -d "$PREFIX"
install -m 755 "$TMP/stele" "$PREFIX/stele"
ok "installed $PREFIX/stele"
"$PREFIX/stele" --version

case ":$PATH:" in
    *":$PREFIX:"*) ;;
    *) warn "add $PREFIX to PATH to run stele by name" ;;
esac

cat <<'FIN'

Next: on the mailbox machine, create the tokens and start `stele serve`; on
this machine, `stele login --url http://<private-ip>:3900 --token -`.
The full guide is docs/installation.md.
FIN
