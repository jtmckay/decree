#!/bin/sh
# Install decree from a GitHub release:
#   curl -fsSL https://raw.githubusercontent.com/jtmckay/decree/main/install.sh | sh
# The newest stable release by default (a beta only while there is no stable 0.5 or later);
# a version with `| sh -s -- v0.5.0`.
# Installs to $XDG_BIN_HOME, default ~/.local/bin (as uv does), with no sudo. Run it again to update.
set -eu
repo=jtmckay/decree
asset=decree-x86_64-linux

fail() { echo "decree install: $*" >&2; exit 1; }

[ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] ||
  fail "a binary is published for Linux x86-64 only; elsewhere: cargo install --git https://github.com/${repo} --tag <tag>"

# Without a version: the newest stable release from 0.5.0 on, however many betas follow it;
# while there is none, the newest beta from 0.5.0 on. Older releases are never picked.
min=0.5.0
at_least_min() { [ "$(printf '%s\n%s\n' "${min}" "${1%%-*}" | sort -V | head -n 1)" = "${min}" ]; }
pick() {
  stable= beta=
  for v in $(sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p'); do
    at_least_min "${v}" || continue
    case "${v}" in
      *-*) beta="${beta}${v}\n" ;;
      *) stable="${stable}${v}\n" ;;
    esac
  done
  v=$(printf "%b" "${stable:-${beta}}" | sort -V | tail -n 1)
  [ -z "${v}" ] || echo "v${v}"
}
tag=${1:-$(curl -fsSL "https://api.github.com/repos/${repo}/releases?per_page=100" | pick)}
[ -n "${tag}" ] || fail "no release of ${min} or later found"

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
