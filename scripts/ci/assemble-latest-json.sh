#!/usr/bin/env bash
# Assemble Tauri updater latest.json from existing GitHub Release assets: download each
# bundle, sign with TAURI_SIGNING_PRIVATE_KEY, upload combined latest.json.
# Used when tauri-action skips latest.json (no matching .sig artifacts during matrix upload).
set -euo pipefail

# RELEASE_TAG is set by release.yml for tag + manual dispatch; fall back to GITHUB_REF_NAME.
TAG="${RELEASE_TAG:-${GITHUB_REF_NAME:?}}"
REPO="${GITHUB_REPOSITORY:?}"

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "::error::TAURI_SIGNING_PRIVATE_KEY is not set."
  echo "::error::Add the minisign private key as a repository secret (TAURI_SIGNING_PRIVATE_KEY)."
  echo "::error::Generate a keypair with: cargo tauri signer generate -w ~/.armaraos/tauri.key"
  echo "::error::Then add the private key content and TAURI_SIGNING_PUBLIC_KEY to GitHub secrets."
  exit 1
fi

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

gh api "repos/${REPO}/releases/tags/${TAG}" >"${WORKDIR}/rel.json"

if [[ "$(jq -r '.message // empty' "${WORKDIR}/rel.json")" == *Not\ Found* ]]; then
  echo "::error::Release for tag ${TAG} not found."
  exit 1
fi

PUB_DATE=$(jq -r '.published_at // empty' "${WORKDIR}/rel.json")
VERSION="${TAG#v}"

pick_url() {
  local pattern="$1"
  jq -r --arg p "$pattern" '.assets[] | select(.name | test($p)) | .browser_download_url' "${WORKDIR}/rel.json" | head -1
}

sign_and_add() {
  local platform_key="$1"
  local url="$2"
  local fname="$3"

  if [[ -z "$url" || "$url" == "null" ]]; then
    echo "::warning::No asset URL for ${platform_key} (expected ${fname} pattern); skipping platform."
    return 0
  fi

  local fpath="${WORKDIR}/${fname}"
  echo "Downloading ${fname}..."
  curl -fsSL "$url" -o "$fpath"

  echo "Signing ${fname}..."
  local -a sign_cmd=(cargo tauri signer sign -k "${TAURI_SIGNING_PRIVATE_KEY}" "$fpath")
  if [[ -n "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]]; then
    sign_cmd+=(-p "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD}")
  fi
  "${sign_cmd[@]}"

  local sigpath=""
  if [[ -f "${fpath}.sig" ]]; then
    sigpath="${fpath}.sig"
  elif [[ -f "${fpath}.minisig" ]]; then
    sigpath="${fpath}.minisig"
  else
    echo "::error::No .sig next to ${fpath}"
    ls -la "$(dirname "$fpath")"
    exit 1
  fi

  jq --arg k "$platform_key" --arg u "$url" --rawfile s "$sigpath" \
    '.platforms[$k] = {url: $u, signature: ($s | rtrimstr("\n"))}' "${WORKDIR}/latest.partial.json" >"${WORKDIR}/latest.next.json"
  mv "${WORKDIR}/latest.next.json" "${WORKDIR}/latest.partial.json"
}

# macOS updater bundles (.app.tar.gz); Windows NSIS; Linux .deb
URL_DARWIN_AARCH=$(pick_url 'ArmaraOS_aarch64\.app\.tar\.gz$')
URL_DARWIN_X64=$(pick_url 'ArmaraOS_x64\.app\.tar\.gz$')
URL_WIN_X64=$(pick_url 'ArmaraOS_.*_x64-setup\.exe$')
URL_WIN_ARM64=$(pick_url 'ArmaraOS_.*_arm64-setup\.exe$')
URL_LINUX_DEB=$(pick_url 'ArmaraOS_.*_amd64\.deb$')

jq -n \
  --arg v "$VERSION" \
  --arg notes "ArmaraOS ${TAG} — https://github.com/${REPO}/releases/tag/${TAG}" \
  --arg pub "$PUB_DATE" \
  '{version: $v, notes: $notes, pub_date: $pub, platforms: {}}' \
  >"${WORKDIR}/latest.partial.json"

sign_and_add "darwin-aarch64" "$URL_DARWIN_AARCH" "ArmaraOS_aarch64.app.tar.gz"
sign_and_add "darwin-x86_64" "$URL_DARWIN_X64" "ArmaraOS_x64.app.tar.gz"
sign_and_add "windows-x86_64" "$URL_WIN_X64" "win-x64-setup.exe"
sign_and_add "windows-aarch64" "$URL_WIN_ARM64" "win-arm64-setup.exe"
sign_and_add "linux-x86_64" "$URL_LINUX_DEB" "linux-amd64.deb"

if [[ "$(jq '.platforms | length' "${WORKDIR}/latest.partial.json")" -eq 0 ]]; then
  echo "::error::No platforms were added to latest.json (asset URLs missing?)."
  exit 1
fi

cp "${WORKDIR}/latest.partial.json" "${WORKDIR}/latest.json"
echo "latest.json:"
jq '.' "${WORKDIR}/latest.json"

echo "Uploading latest.json to ${TAG}..."
gh release upload "$TAG" "${WORKDIR}/latest.json" --clobber

echo "Done."
