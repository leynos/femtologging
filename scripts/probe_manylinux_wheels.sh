#!/usr/bin/env bash
# Probe each manylinux wheel inside a matching manylinux container.
#
# The container carries the CPython interpreters the wheels were built for, so
# installing and importing run on the target platform rather than on the runner's
# host libc. An aarch64 job relies on the QEMU emulation it sets up.
#
# Environment:
#   PROBE_IMAGE      manylinux image to probe in (required).
#   PROBE_VERSIONS   Space-separated CPython versions (default: 3.12 3.13 3.14).
#   DIST_DIRECTORY   Directory holding the built wheels (default: dist).
#   PROBE_PLATFORM   Value passed to docker --platform (default: linux/amd64).

set -euo pipefail

image=${PROBE_IMAGE:?PROBE_IMAGE must name the manylinux image}
versions=${PROBE_VERSIONS:-3.12 3.13 3.14}
directory=${DIST_DIRECTORY:-dist}
platform=${PROBE_PLATFORM:-linux/amd64}
probe_script=/scripts/release_wheel_probe.py

for version in ${versions}; do
  tag="cp${version//./}"
  shopt -s nullglob
  wheels=("${directory}"/*-"${tag}"-"${tag}"-*.whl)
  if [ "${#wheels[@]}" -ne 1 ]; then
    printf 'expected exactly one CPython %s wheel in %s, found %s\n' \
      "${version}" "${directory}" "${#wheels[@]}" >&2
    exit 1
  fi

  wheel_name=$(basename "${wheels[0]}")
  python=/opt/python/"${tag}-${tag}"/bin/python
  # The probe runs from an empty directory so the mounted checkout cannot
  # satisfy the import it is meant to prove.
  docker run --rm --platform "${platform}" \
    -v "${PWD}/${directory}:/dist:ro" \
    -v "${PWD}/scripts:/scripts:ro" \
    "${image}" /bin/bash -c "set -euo pipefail
'${python}' -m pip install --quiet --no-deps --no-index '/dist/${wheel_name}'
cd \"\$(mktemp -d)\"
'${python}' -I '${probe_script}'"
done