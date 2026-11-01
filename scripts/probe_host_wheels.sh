#!/usr/bin/env bash
# Probe each release wheel on the runner that produced it.
#
# A wheel that cannot install or import must not become a build artefact, so a
# build job runs this before it uploads. Every wheel is interpreter-specific, so
# the script starts a fresh environment from the matching CPython and runs the
# isolation probe there.
#
# Environment:
#   PROBE_VERSIONS   Space-separated CPython versions (default: 3.12 3.13 3.14).
#   DIST_DIRECTORY   Directory holding the built wheels (default: dist).
#   PROBE_SCRIPT     Probe run in each environment (default:
#                    scripts/release_wheel_probe.py).

set -euo pipefail

versions=${PROBE_VERSIONS:-3.12 3.13 3.14}
directory=${DIST_DIRECTORY:-dist}
probe_script=${PROBE_SCRIPT:-scripts/release_wheel_probe.py}
probe_path="${PWD}/${probe_script}"

# Resolve one interpreter for a CPython version. actions/setup-python installs
# every requested version into RUNNER_TOOL_CACHE, and only the last one keeps
# the plain `python` name, so the tool cache is searched first. The PATH lookup
# covers a host that exposes versioned names but no tool cache.
resolve_python() {
  local version=$1 candidate reported
  local -a candidates=()

  if [ -n "${RUNNER_TOOL_CACHE:-}" ]; then
    shopt -s nullglob
    candidates=(
      "${RUNNER_TOOL_CACHE}"/Python/"${version}".*/x64/bin/python"${version}"
      "${RUNNER_TOOL_CACHE}"/Python/"${version}".*/arm64/bin/python"${version}"
      "${RUNNER_TOOL_CACHE}"/Python/"${version}".*/x64/python.exe
      "${RUNNER_TOOL_CACHE}"/Python/"${version}".*/arm64/python.exe
    )
  fi
  if [ "${#candidates[@]}" -gt 0 ]; then
    printf '%s\n' "${candidates[0]}"
    return 0
  fi

  for candidate in "python${version}" python; do
    if ! command -v "${candidate}" >/dev/null 2>&1; then
      continue
    fi
    reported=$("${candidate}" -c 'import sys; print("%d.%d" % sys.version_info[:2])')
    if [ "${reported}" = "${version}" ]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  printf 'cannot find CPython %s to probe its wheel\n' "${version}" >&2
  return 1
}

probe_wheel() {
  local version=$1 tag venv probe_python scratch
  local -a wheels=()

  tag="cp${version//./}"
  shopt -s nullglob
  wheels=("${directory}"/*-"${tag}"-"${tag}"-*.whl)
  if [ "${#wheels[@]}" -ne 1 ]; then
    printf 'expected exactly one CPython %s wheel in %s, found %s\n' \
      "${version}" "${directory}" "${#wheels[@]}" >&2
    return 1
  fi

  # Scratch lives outside the checkout so a local run leaves no trace. The
  # probe runs from an empty directory inside it, so a checkout cannot satisfy
  # the import the probe is meant to prove. The process ID keeps concurrent
  # local runs apart without relying on a platform-specific mktemp template.
  scratch="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/femtologging-probe-${tag}-$$"
  rm -rf "${scratch}"
  venv="${scratch}/venv"
  mkdir -p "${scratch}/probe"
  "$(resolve_python "${version}")" -m venv "${venv}"
  case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) probe_python="${venv}/Scripts/python.exe" ;;
    *) probe_python="${venv}/bin/python" ;;
  esac
  "${probe_python}" -m pip install --quiet --no-deps --no-index "${wheels[0]}"

  (
    cd "${scratch}/probe"
    "${probe_python}" -I "${probe_path}"
  )
}

for version in ${versions}; do
  probe_wheel "${version}"
done