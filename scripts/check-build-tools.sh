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
# `RUSTUP_AUTO_INSTALL=0` removes the remaining route to provisioning: an
# inherited `RUSTUP_TOOLCHAIN` naming an absent toolchain, which changing
# directory does not affect. A capability probe must never install what it is
# meant to find missing.
installed_toolchains() {
  ( cd -- / && RUSTUP_AUTO_INSTALL=0 rustup toolchain list )
}

# Resolve rustup's default host triple from a neutral directory.
#
# As with the enumeration, `/` lies outside the repository, so the pinned
# `rust-toolchain.toml` cannot override the answer, and `RUSTUP_AUTO_INSTALL=0`
# keeps a probe from provisioning whatever `RUSTUP_TOOLCHAIN` names. The host
# line is read even when `rustup show` exits non-zero: with an absent default
# toolchain the command prints the host it resolved and then fails on the
# toolchain it could not activate. Discarding that host would fall back to the
# bare-channel match, which rejects a pin that is installed for this host. A
# probe that failed is still worth naming, so the recovery is reported rather
# than passed over. `note` writes to stderr, which the caller's command
# substitution does not capture, so the diagnostic reaches the operator.
#
# Both failure shapes name `rustup show`: a probe that ran and failed is a
# different operator problem from one that answered without a host triple.
host_triple() {
  local shown status=0 host
  shown=$( cd -- / && RUSTUP_AUTO_INSTALL=0 rustup show ) || status=$?
  host=$(printf '%s\n' "$shown" | sed -n 's/^Default host: //p')
  if [ -z "$host" ]; then
    if [ "$status" -ne 0 ]; then
      note 'rustup show failed; cannot determine the default host triple'
    else
      note 'rustup show reported no default host triple'
    fi
    return 1
  fi
  if [ "$status" -ne 0 ]; then
    note "rustup show exited ${status}; using the host it reported"
  fi
  printf '%s\n' "$host"
}

# Report whether `channel` is installed for this host, matching its name
# literally.
#
# rustup prints one line per toolchain as `<name>[ (<status>)]`, so the first
# whitespace-delimited field is the name. The pinned channel is materialised
# as `<channel>-<host>`; accepting any host-triple suffix would let another
# architecture's toolchain satisfy a preflight whose linker and binaries are
# host-specific. Only when rustup cannot report a host triple is the exact
# bare `<channel>` accepted, as a fallback rather than a general pattern.
#
# Returns 0 when installed, 1 when the listing does not contain the channel,
# and 2 when rustup could not produce a listing at all. The caller keeps those
# apart: a failed probe must not be reported as an absent toolchain.
toolchain_is_installed() {
  local channel=$1 host listing line name
  host=$(host_triple) || host=
  listing=$(installed_toolchains) || return 2
  while IFS= read -r line; do
    name=${line%%[[:space:]]*}
    if [ -n "$host" ]; then
      if [ "$name" = "$channel-$host" ]; then
        return 0
      fi
    elif [ "$name" = "$channel" ]; then
      return 0
    fi
  done <<< "$listing"
  return 1
}

# Report on the toolchain half: rustup must be present, and the pinned channel
# must already be installed. A stable/MSRV promise does not remove the need for
# the pinned toolchain to be materialized on the host.
check_toolchain() {
  local channel=$1 status=0
  if ! command -v rustup >/dev/null 2>&1; then
    note 'rustup not found on PATH; it selects the pinned toolchain'
    note 'install it from https://rustup.rs'
    return 1
  fi
  # The `|| status=$?` form keeps a failing probe from tripping `set -e` while
  # preserving the distinction between "not installed" and "could not list".
  toolchain_is_installed "$channel" || status=$?
  if [ "$status" -eq 0 ]; then
    note "toolchain ${channel} available"
    return 0
  fi
  if [ "$status" -eq 1 ]; then
    note "toolchain ${channel} is not installed"
    note 'install it with: make install-build-tools'
    return 1
  fi
  note 'rustup could not list installed toolchains'
  return 1
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