#!/usr/bin/env bash
# Verify the prerequisites of the repository's development build standard.
#
# Runs before every target that compiles Rust, so a missing linker or toolchain
# fails with an actionable message instead of an opaque linker error deep
# inside Cargo. Exits non-zero when a required component is absent or unusable.
#
# The OS can be overridden with BUILD_TOOLS_OS so the contract tests can
# exercise both the Linux and the non-Linux route on one host.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)

note() {
  printf 'build-tools: %s\n' "$1" >&2
}

host_os() {
  printf '%s\n' "${BUILD_TOOLS_OS:-$(uname -s)}"
}

# Read the channel from rust-toolchain.toml rather than repeating the pin here.
# A second copy would silently drift from the toolchain Cargo actually selects.
pinned_toolchain() {
  local channel
  channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$repo_root/rust-toolchain.toml")
  if [ -z "$channel" ]; then
    note 'no channel found in rust-toolchain.toml'
    return 1
  fi
  printf '%s\n' "$channel"
}

# Enumerate rustup's installed toolchains from a neutral directory.
#
# `rustup toolchain list` honours a `rust-toolchain.toml` override discovered
# from the working directory. Running it at the repository root would let
# rustup resolve -- and potentially provision -- the pinned channel, which both
# defeats the capability check and can mutate the host. `/` lies outside the
# repository, so the listing reflects only what is genuinely installed.
installed_toolchains() {
  ( cd -- / && rustup toolchain list )
}

# Report whether `channel` is installed, matching its name literally.
#
# rustup prints one line per toolchain as `<name>[ (<status>)]`, so the first
# whitespace-delimited field is the name. Literal `case` patterns accept only
# the exact channel or that channel with a host-triple suffix; a prefix regex
# would wrongly accept `1.99.1` when the host only has `1.99.10`.
toolchain_is_installed() {
  local channel=$1 line name
  while IFS= read -r line; do
    name=${line%%[[:space:]]*}
    case "$name" in
      "$channel" | "$channel"-*) return 0 ;;
    esac
  done < <(installed_toolchains)
  return 1
}

# Report on the toolchain half: rustup must be present, and the pinned channel
# must already be installed. A stable/MSRV promise does not remove the need for
# the pinned toolchain to be materialized on the host.
check_toolchain() {
  local channel=$1
  if ! command -v rustup >/dev/null 2>&1; then
    note 'rustup not found on PATH; it selects the pinned toolchain'
    note 'install it from https://rustup.rs'
    return 1
  fi
  if ! toolchain_is_installed "$channel"; then
    note "toolchain ${channel} is not installed"
    note 'install it with: make install-build-tools'
    return 1
  fi
  note "toolchain ${channel} available"
}

# Report on the linker half. mold ships for Linux only, so every other host
# legitimately falls back to its platform linker; that is a note, not a failure.
check_mold() {
  local resolved
  if [ "$(host_os)" != Linux ]; then
    note "mold is Linux-only; using the default $(host_os) linker"
    return 0
  fi
  if ! resolved=$(command -v mold 2>/dev/null); then
    note 'mold not found on PATH; the Linux development build links with it'
    note 'install it with: make install-build-tools'
    return 1
  fi
  # A mold that cannot report its version is broken (a truncated download or an
  # unresolved shared library), so treat it as missing rather than letting the
  # opaque failure surface later at link time.
  if ! mold --version >/dev/null 2>&1; then
    note "mold at ${resolved} is on PATH but cannot run"
    note 'reinstall it with: make install-build-tools'
    return 1
  fi
  note "mold at ${resolved}"
}

# Run both checks unconditionally, so one run reports every missing piece.
main() {
  local channel status=0
  channel=$(pinned_toolchain) || return 1
  check_toolchain "$channel" || status=1
  check_mold || status=1
  if [ "$status" -ne 0 ]; then
    note 'capability check failed; see the messages above'
  fi
  return "$status"
}

main "$@"