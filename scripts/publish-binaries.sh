#!/bin/sh
# Publishes the three tarballs a green `binaries` run built into the GitLab
# release, with a .sha256 beside each one.
#
#     sh scripts/publish-binaries.sh v0.2.0
#     sh scripts/publish-binaries.sh v0.2.0 --run 43779809277
#     sh scripts/publish-binaries.sh v0.2.0 --dry-run
#
# Why by hand: GitHub Actions only compiles, because no GitLab credential is
# stored on that side. The script trusts one thing, a run whose commit is the
# tag it was asked to publish, and refuses anything else. Signing is the next
# and separate act: scripts/sign-release.sh, with the passphrase typed by the
# person who holds the key.

set -eu

REPO="${STELE_GITHUB_REPO:-alexander-ae/stele}"
GL_REPO="${STELE_REPO:-__alexander__/stele}"
ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
TAG=""
RUN=""
DRY_RUN=0
ASSETS="stele-linux-x86_64.tar.gz stele-macos-arm64.tar.gz stele-macos-x86_64.tar.gz"

usage() {
    cat <<'FIN'
Usage: sh scripts/publish-binaries.sh <tag> [--run <id>] [--dry-run]

  --run <id>  use that GitHub Actions run instead of looking for one
  --dry-run   download and hash the tarballs, upload nothing
  --help      this

Variables: STELE_GITHUB_REPO (default alexander-ae/stele),
STELE_REPO (default __alexander__/stele).

The tag has to exist locally and on GitHub; the run has to be green and built
from the same commit. The release is created from the tag's CHANGELOG section
when it does not exist yet.
FIN
}

ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$1" >&2; }
die()  { printf '  \033[31m✗\033[0m %s\n' "$1" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --run)     [ $# -ge 2 ] || { usage >&2; exit 2; }; RUN="$2"; shift 2 ;;
        --dry-run) DRY_RUN=1; shift ;;
        -h|--help) usage; exit 0 ;;
        -*)        echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
        *)         [ -z "$TAG" ] || { echo "only one tag" >&2; exit 2; }; TAG="$1"; shift ;;
    esac
done
[ -n "$TAG" ] || { usage >&2; exit 2; }

command -v gh >/dev/null 2>&1 || die "gh is not on PATH"
command -v glab >/dev/null 2>&1 || die "glab is not on PATH"
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 --connect-timeout 20 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "curl or wget is required"
fi
if command -v sha256sum >/dev/null 2>&1; then
    hashfile() { ( cd "$1" && sha256sum "$2" > "$2.sha256" ); }
    checkfile() { ( cd "$1" && sha256sum -c "$2" >/dev/null ); }
else
    hashfile() { ( cd "$1" && shasum -a 256 "$2" > "$2.sha256" ); }
    checkfile() { ( cd "$1" && shasum -a 256 -c "$2" >/dev/null ); }
fi

SHA="$(git -C "$ROOT" rev-parse -q --verify "refs/tags/$TAG^{commit}" 2>/dev/null)" \
    || die "no local tag $TAG; create it, push it, and wait for the mirror"

if [ -z "$RUN" ]; then
    RUN="$(gh run list --repo "$REPO" --workflow binaries --limit 40 \
        --json databaseId,headSha,conclusion \
        --jq ".[] | select(.headSha == \"$SHA\" and .conclusion == \"success\") | .databaseId" \
        | head -n 1)"
    [ -n "$RUN" ] || die "no green binaries run for $TAG ($SHA); start it from the GitHub UI"
fi
ok "run $RUN built $TAG"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
gh run download "$RUN" --repo "$REPO" -D "$TMP/raw" >/dev/null \
    || die "cannot download the artifacts of run $RUN"
mkdir -p "$TMP/out" "$TMP/back"

echo "Hashing what the run built"
for f in $ASSETS; do
    src="$TMP/raw/${f%.tar.gz}/$f"
    [ -f "$src" ] || die "run $RUN did not produce $f"
    cp "$src" "$TMP/out/$f"
    hashfile "$TMP/out" "$f"
    ok "$f"
done

if glab release view "$TAG" -R "$GL_REPO" >/dev/null 2>&1; then
    ok "release $TAG exists"
else
    notes="$(awk -v t="$TAG" '
        index($0, "## " t) == 1 { inside = 1; next }
        inside && index($0, "## ") == 1 { exit }
        inside { print }
    ' "$ROOT/CHANGELOG.md")"
    [ -n "$notes" ] || die "no '## $TAG' section in CHANGELOG.md to describe the release"
    if [ "$DRY_RUN" -eq 1 ]; then
        warn "would create release $TAG"
    else
        glab release create "$TAG" -R "$GL_REPO" --name "$TAG" --notes "$notes" >/dev/null \
            || die "cannot create release $TAG"
        ok "release $TAG created"
    fi
fi

if [ "$DRY_RUN" -eq 1 ]; then
    echo
    echo "Dry run: the tarballs and their .sha256 are in $TMP/out and were not uploaded."
    exit 0
fi

echo "Uploading to the GitLab release"
for f in $ASSETS; do
    for asset in "$f" "$f.sha256"; do
        glab release upload -R "$GL_REPO" "$TAG" "$TMP/out/$asset" >/dev/null 2>&1 \
            || warn "$asset may already be in the release; check it"
    done
done
ok "upload done"

echo "Downloading them back and checking the hashes"
BASE="https://gitlab.com/$GL_REPO/-/releases/$TAG/downloads"
for f in $ASSETS; do
    fetch "$BASE/$f" "$TMP/back/$f" || die "$f does not come back from the release"
    fetch "$BASE/$f.sha256" "$TMP/back/$f.sha256" \
        || die "$f.sha256 does not come back from the release"
    checkfile "$TMP/back" "$f.sha256" || die "$f came back but does not match its sha256"
    cmp -s "$TMP/out/$f.sha256" "$TMP/back/$f.sha256" \
        || die "$f: the published sha256 is not the one computed here"
    ok "$f: published and matches"
done

echo
echo "Published $TAG. Next: sh scripts/sign-release.sh $TAG"
