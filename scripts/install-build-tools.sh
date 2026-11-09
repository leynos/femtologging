#!/usr/bin/env bash
# Install the prerequisites of the repository's development build standard.
#
# Installs the toolchain rust-toolchain.toml pins, including the components the
# gates use, and the mold linker on Linux. Nothing here touches the release,
# packaging, coverage, or Whitaker toolchains, which deliberately exclude the
# development standard.
#
# The OS can be overridden with BUILD_TOOLS_OS so the contract tests can
# exercise both the Linux and the non-Linux route on one host.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)

note() {
  printf 'build-tools: %s\n' "$1" >&2
}

fail() {
  note "$1"
  exit 1
}

host_os() {
  printf '%s\n' "${BUILD_TOOLS_OS:-$(uname -s)}"
}

pinned_toolchain() {
  local channel
  channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$repo_root/rust-toolchain.toml")
  if [ -z "$channel" ]; then
    note 'no channel found in rust-toolchain.toml'
    return 1
  fi
  printf '%s\n' "$channel"
}

install_toolchain() {
  local channel=$1
  command -v rustup >/dev/null 2>&1 ||
    fail 'rustup not found on PATH; install it from https://rustup.rs'
  note "installing toolchain ${channel}"
  rustup toolchain install "$channel" --profile minimal \
    --component rustfmt --component clippy --component rust-analyzer ||
    fail "failed to install toolchain ${channel}"
}

# Install mold through the host package manager when that can be done without
# an interactive prompt. Exporting the command instead of hiding it keeps the
# failure actionable on a host where unattended installation is unavailable.
install_mold_command() {
  if command -v dnf >/dev/null 2>&1; then
    printf 'dnf install -y mold\n'
  elif command -v apt-get >/dev/null 2>&1; then
    printf 'apt-get install -y mold\n'
  elif command -v pacman >/dev/null 2>&1; then
    printf 'pacman -S --noconfirm mold\n'
  else
    printf 'install the mold linker with your package manager\n'
  fi
}

run_package_install() {
  local command=$1
  if [ "$(id -u)" -eq 0 ]; then
    # shellcheck disable=SC2086  # the command is a deliberately simple word list
    $command
  elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    # shellcheck disable=SC2086  # as above
    sudo -n $command
  else
    return 1
  fi
}

install_mold() {
  local command
  if [ "$(host_os)" != Linux ]; then
    note "mold is Linux-only; skipping on $(host_os), the platform linker is used"
    return 0
  fi
  if command -v mold >/dev/null 2>&1 && mold --version >/dev/null 2>&1; then
    note 'mold already installed'
    return 0
  fi
  command=$(install_mold_command)
  note "installing mold: ${command}"
  if ! run_package_install "$command"; then
    note 'automatic installation is unavailable; run this command as an administrator:'
    note "  ${command}"
    return 1
  fi
  if ! { command -v mold >/dev/null 2>&1 && mold --version >/dev/null 2>&1; }; then
    fail 'mold is still unavailable after installation'
  fi
  note 'mold installed'
}

# Install the toolchain first: it is the larger download, and a linker failure
# after it leaves the developer with a usable Rust installation either way.
main() {
  local channel
  channel=$(pinned_toolchain) || return 1
  install_toolchain "$channel"
  install_mold
  note 'ready; verify with: make check-build-tools'
}

main "$@"