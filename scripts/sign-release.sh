#!/bin/sh
# Signs the assets of a published stele release with minisign, and uploads
# the .minisig files beside them.
#
# Why by hand and why here: the signature says "the person who maintains
# stele saw these exact bytes". A key living in CI would sign whatever the
# pipeline built, which the .sha256 already attests. The private key stays on
# this machine and asks for its passphrase once.
#
#     sh scripts/sign-release.sh v0.2.0
#     sh scripts/sign-release.sh v0.2.0 --dry-run
#
# The first time, generate the key pair and publish the public half:
#
#     mkdir -p ~/.minisign
#     minisign -G -p stele.pub -s ~/.minisign/stele.key
#
# Put the one-line public key in stele.pub, the README, the GitLab project
# description and install.sh (as the verification key).
set -eu

REPO="${STELE_REPO:-__alexander__/stele}"
KEY="${STELE_SIGNING_KEY:-$HOME/.minisign/stele.key}"
ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
PUB="${STELE_PUB:-$ROOT/stele.pub}"
TAG=""
DRY_RUN=0

ASSETS="stele-linux-x86_64.tar.gz stele-macos-arm64.tar.gz stele-macos-x86_64.tar.gz"

usage() {
    cat <<'FIN'
Usage: sh scripts/sign-release.sh <tag> [--dry-run]

  --dry-run   sign locally and verify, but upload nothing
  --help      this

Variables: STELE_SIGNING_KEY (default ~/.minisign/stele.key),
STELE_PUB (default stele.pub in the repository root), STELE_REPO.

The private key needs a passphrase; minisign asks for it once, here.
FIN
}

ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$1" >&2; }
die()  { printf '  \033[31m✗\033[0m %s\n' "$1" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=1; shift ;;
        -h|--help) usage; exit 0 ;;
        -*)        echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
        *)         [ -z "$TAG" ] || { echo "only one tag" >&2; exit 2; }; TAG="$1"; shift ;;
    esac
done
[ -n "$TAG" ] || { usage >&2; exit 2; }

if ! command -v minisign >/dev/null 2>&1 && [ -x "$HOME/.local/bin/minisign" ]; then
    PATH="$HOME/.local/bin:$PATH"
    export PATH
fi
command -v minisign >/dev/null 2>&1 || die "minisign is not on PATH"
command -v glab >/dev/null 2>&1 || die "glab is not on PATH"

[ -f "$KEY" ] || die "no signing key at $KEY (generate it with: minisign -G -p \"$PUB\" -s \"$KEY\")"
[ -f "$PUB" ] || die "no public key at $PUB (it is created by the -G above)"

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 --connect-timeout 20 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "curl or wget is required"
fi

BASE="https://gitlab.com/$REPO/-/releases/$TAG/downloads"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading $TAG from $REPO"
for f in $ASSETS; do
    fetch "$BASE/$f" "$TMP/$f" || die "cannot download $BASE/$f"
    fetch "$BASE/$f.sha256" "$TMP/$f.sha256" || die "no .sha256 for $f"
    if command -v sha256sum >/dev/null 2>&1; then
        ( cd "$TMP" && sha256sum -c "$f.sha256" >/dev/null ) || die "$f does not match its sha256"
    else
        ( cd "$TMP" && shasum -a 256 -c "$f.sha256" >/dev/null ) || die "$f does not match its sha256"
    fi
    ok "$f matches its sha256"
done

echo "Signing (minisign will ask for the key passphrase once)"
# Word splitting is the point: minisign signs every asset in one call, so it
# asks for the passphrase once.
# shellcheck disable=SC2086
( cd "$TMP" && minisign -S -s "$KEY" -m $ASSETS ) || die "minisign failed"
for f in $ASSETS; do
    [ -f "$TMP/$f.minisig" ] || die "$f was not signed"
    ok "$f.minisig"
done

echo "Verifying against the public key"
for f in $ASSETS; do
    minisign -V -p "$PUB" -m "$TMP/$f" -x "$TMP/$f.minisig" >/dev/null \
        || die "$f does not verify against $PUB"
    ok "$f verifies"
done

if [ "$DRY_RUN" -eq 1 ]; then
    echo
    echo "Dry run: signatures are in $TMP and were not uploaded."
    exit 0
fi

echo "Uploading the signatures to the GitLab release"
( cd "$ROOT" && for f in $ASSETS; do
    glab release upload -R "$REPO" "$TAG" "$TMP/$f.minisig" >/dev/null 2>&1 \
        || warn "$f.minisig may already be in the release; check it"
done )
ok "upload done"

echo "Downloading the signatures back and verifying"
for f in $ASSETS; do
    fetch "$BASE/$f.minisig" "$TMP/back-$f.minisig" \
        || die "$f.minisig does not come back from the release"
    minisign -V -p "$PUB" -m "$TMP/$f" -x "$TMP/back-$f.minisig" >/dev/null \
        || die "$f.minisig came back but does not verify"
    ok "$f: published and verifies"
done

echo
echo "Signed $TAG. Next: put the public key in the README, the repository"
echo "stele.pub, the GitLab project description, and install.sh."
