#!/usr/bin/env bash
# Publish one FlowShot AUR package. One script for hand AND CI runs:
# same steps, same guards.
#
# Usage: publish.sh <pkgname> <prepared-dir> "<commit message>"
#   <prepared-dir> must contain the final PKGBUILD and .SRCINFO.
#
# Auth: AUR uses ACCOUNT-LEVEL ssh keys (no per-package deploy keys), so CI
# must use a DEDICATED AUR account owning only the FlowShot packages - a
# leaked key then cannot reach unrelated packages. Provide the private key
# via AUR_SSH_PRIVATE_KEY (CI) or a preconfigured ~/.ssh/aur (manual).
#
# Refuses to publish while checksums still say SKIP, regenerates nothing
# itself (the caller renders PKGBUILD/.SRCINFO), no-ops when the AUR copy is
# already identical, and pushes to master only (the AUR accepts nothing else).
set -euo pipefail

pkgname="${1:?usage: publish.sh <pkgname> <prepared-dir> <message>}"
prepared="${2:?missing prepared directory}"
message="${3:?missing commit message}"

[[ -f "$prepared/PKGBUILD" && -f "$prepared/.SRCINFO" ]] || {
  echo "prepared dir must contain PKGBUILD and .SRCINFO" >&2; exit 1;
}
# VCS packages legitimately carry SKIP checksums (their source is a moving
# git ref; the user's checkout is checksummed by makepkg locally) - every
# other package must carry real checksums.
if [[ "$pkgname" != *-git ]] && grep -qE "sums=\('SKIP'\)" "$prepared/PKGBUILD"; then
  echo "refusing to publish a non-VCS package with SKIP checksums" >&2; exit 1
fi

SSH_DIR="${AUR_SSH_DIR:-$HOME/.ssh}"
if [[ -n "${AUR_SSH_PRIVATE_KEY:-}" ]]; then
  SSH_DIR="$(mktemp -d)"
  trap 'rm -rf "$SSH_DIR"' EXIT
  install -d -m 700 "$SSH_DIR"
  install -m 600 /dev/null "$SSH_DIR/aur"
  printf '%s\n' "$AUR_SSH_PRIVATE_KEY" > "$SSH_DIR/aur"
  # Host keys are PINNED, never trust-on-first-use. Fingerprints re-derived
  # 2026-10-09 via: ssh-keygen -lf <(ssh-keyscan -t ed25519,ecdsa,rsa aur.archlinux.org)
  # If Arch rotates keys, update both this file and the verification below.
  cat > "$SSH_DIR/known_hosts" <<'EOF'
aur.archlinux.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEuBKrPzbawxA/k2g6NcyV5jmqwJ2s+zpgZGZ7tpLIcN
EOF
  cat > "$SSH_DIR/config" <<EOF
Host aur.archlinux.org
  User aur
  IdentityFile $SSH_DIR/aur
  IdentitiesOnly yes
  StrictHostKeyChecking yes
  UserKnownHostsFile $SSH_DIR/known_hosts
EOF
  chmod 600 "$SSH_DIR/config" "$SSH_DIR/known_hosts"
  # Fail loudly if the pinned key is not what the host offers.
  scanned="$(mktemp)"
  ssh-keyscan -t ed25519 aur.archlinux.org 2>/dev/null > "$scanned"
  fp="$(ssh-keygen -lf "$scanned" | awk '{print $2}')"
  rm -f "$scanned"
  [[ "$fp" == "SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4" ]] || {
    echo "unexpected AUR host key fingerprint: $fp" >&2; exit 1; }
fi

export GIT_SSH_COMMAND="ssh -F $SSH_DIR/config"
work="$(mktemp -d)"
trap 'rm -rf "$work"; [[ -n "${AUR_SSH_PRIVATE_KEY:-}" ]] && rm -rf "$SSH_DIR"' EXIT

first_publish=0
if ! git clone -q "ssh://aur@aur.archlinux.org/$pkgname.git" "$work/aur" 2>/dev/null; then
  # The pkgbase does not exist yet: AUR creates it on the first push to
  # ssh://aur@aur.archlinux.org/<pkgbase>.git - clone cannot work, so init
  # locally and push into the empty repository instead.
  echo "$pkgname: not on the AUR yet - first publish"
  first_publish=1
  mkdir -p "$work/aur"
  cd "$work/aur"
  git init -q -b master .
  git remote add origin "ssh://aur@aur.archlinux.org/$pkgname.git"
else
  cd "$work/aur"
fi
install -m644 "$prepared/PKGBUILD" "$prepared/.SRCINFO" .
git add PKGBUILD .SRCINFO
# Staged, not working-tree: on a first publish the repo is empty, so a
# working-tree diff sees nothing while the files are merely untracked.
if [[ "$first_publish" -eq 0 ]] && git diff --cached --quiet; then
  echo "$pkgname: already published, nothing to do"
  exit 0
fi
git -c user.name="${AUR_GIT_NAME:-FlowOSS}" \
    -c user.email="${AUR_GIT_EMAIL:-git@flowhost.io}" \
    commit -q -m "$message"
git push -q origin HEAD:master
echo "$pkgname: published"
