#!/bin/sh
# Install decree from a GitHub release:
#   curl -fsSL https://raw.githubusercontent.com/jtmckay/decree/v0.5/install.sh | sh
# The newest release by default (betas included); a version with `| sh -s -- v0.5.0-beta.11`.
# Installs to $XDG_BIN_HOME, default ~/.local/bin (as uv does), with no sudo. Run it again to update.
set -eu
repo=jtmckay/decree
asset=decree-x86_64-linux

fail() { echo "decree install: $*" >&2; exit 1; }

[ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] ||
  fail "a binary is published for Linux x86-64 only; elsewhere: cargo install --git https://github.com/${repo} --tag <tag>"

# The releases list puts the newest first, prereleases included (`releases/latest` skips them).
tag=${1:-$(curl -fsSL "https://api.github.com/repos/${repo}/releases?per_page=1" |
  sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)}
[ -n "${tag}" ] || fail "no release found"

dir=${XDG_BIN_HOME:-${HOME}/.local/bin}
tmp=$(mktemp -d)
trap 'rm -rf "${tmp}"' EXIT
base="https://github.com/${repo}/releases/download/${tag}"
curl -fsSL -o "${tmp}/${asset}" "${base}/${asset}" || fail "${tag} has no ${asset}"
curl -fsSL -o "${tmp}/${asset}.sha256" "${base}/${asset}.sha256" || fail "${tag} has no ${asset}.sha256"
(cd "${tmp}" && sha256sum -c --status "${asset}.sha256") || fail "checksum mismatch for ${tag}"

mkdir -p "${dir}"
install -m 755 "${tmp}/${asset}" "${dir}/decree"
echo "installed $("${dir}/decree" --version) to ${dir}/decree"
case ":${PATH}:" in
  *":${dir}:"*) ;;
  *) echo "add ${dir} to PATH" ;;
esac
found=$(command -v decree || true)
[ -z "${found}" ] || [ "${found}" = "${dir}/decree" ] ||
  echo "note: ${found} comes first on PATH; remove it (cargo uninstall decree) or reorder PATH"
