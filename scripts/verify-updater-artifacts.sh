#!/usr/bin/env bash
# Checks a published release's updater artifacts (v0.7 plan U5):
# - latest.json names this version and only URLs inside this release;
# - every installer has a `.sig`, and latest.json carries exactly that signature;
# - every `.sig` verifies with minisign against the public key in tauri.conf.json.
#
# Usage: verify-updater-artifacts.sh <tag> <tauri.conf.json> [asset dir]
# Without an asset dir the assets are downloaded with `gh release download`
# (needs GH_TOKEN and GITHUB_REPOSITORY). Needs jq, base64 and minisign.
set -euo pipefail

tag="${1:?tag}"
config="${2:?tauri.conf.json}"
dir="${3:-}"
version="${tag#v}"
repo="${GITHUB_REPOSITORY:-bs135/xpass}"
work="$(mktemp -d)"

if [ -z "$dir" ]; then
  dir="$work/assets"
  mkdir -p "$dir"
  gh release download "$tag" --repo "$repo" --dir "$dir" \
    --pattern 'latest.json' --pattern '*.sig' --pattern '*.exe' --pattern '*.msi'
fi

# The committed key: base64 of the minisign public key file.
jq -r '.plugins.updater.pubkey' "$config" | base64 -d > "$work/updater.pub"

cd "$dir"
fail() { echo "::error::$*"; exit 1; }

[ -f latest.json ] || fail "no latest.json in the release"
jq -e --arg v "$version" '.version == $v or .version == ("v" + $v)' latest.json > /dev/null ||
  fail "latest.json does not name version $version"

prefix="https://github.com/$repo/releases/download/$tag/"
entries="$(jq -r '.platforms | to_entries[] | "\(.key) \(.value.url) \(.value.signature)"' latest.json)"
[ -n "$entries" ] || fail "latest.json lists no platform"
while read -r key url signature; do
  case "$url" in
    "$prefix"*) ;;
    *) fail "$key points outside this release: $url" ;;
  esac
  name="${url##*/}"
  [ -f "$name.sig" ] || fail "$key: no $name.sig in the release"
  [ "$signature" = "$(cat "$name.sig")" ] || fail "$key: signature differs from $name.sig"
  echo "ok: $key -> $name"
done <<< "$entries"

count=0
for installer in *.exe *.msi; do
  [ -f "$installer" ] || continue
  [ -f "$installer.sig" ] || fail "no signature for $installer"
  base64 -d "$installer.sig" > "$work/$installer.minisig"
  minisign -V -q -m "$installer" -x "$work/$installer.minisig" -p "$work/updater.pub" ||
    fail "$installer: signature does not verify with the committed public key"
  echo "verified: $installer"
  count=$((count + 1))
done
[ "$count" -ge 2 ] || fail "expected the NSIS and MSI installers, found $count"
echo "updater artifacts OK for $tag"
