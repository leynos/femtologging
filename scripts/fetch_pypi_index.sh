#!/usr/bin/env bash
set -euo pipefail

: "${PYPI_SIMPLE_URL:?PYPI_SIMPLE_URL must be set}"
: "${PYPI_INDEX_PATH:?PYPI_INDEX_PATH must be set}"
allow_missing="${PYPI_ALLOW_MISSING:-false}"

fetch_index() {
  local attempt=0
  local status=000
  while [ "${attempt}" -lt 6 ]; do
    attempt=$((attempt + 1))
    status=$(curl --silent --show-error --location \
      --connect-timeout 10 --max-time 60 \
      --output "${PYPI_INDEX_PATH}" --write-out '%{http_code}' \
      --header 'Accept: application/vnd.pypi.simple.v1+json' \
      "${PYPI_SIMPLE_URL}") || status=000
    case "${status}" in
      000|429|5??)
        if [ "${attempt}" -lt 6 ]; then
          sleep 2
        fi
        ;;
      *)
        break
        ;;
    esac
  done

  case "${status}" in
    200) ;;
    404)
      if [ "${allow_missing}" = true ]; then
        printf '{"files": []}\n' > "${PYPI_INDEX_PATH}"
      else
        echo "PyPI project is unpublished; no index is available." >&2
        exit 1
      fi
      ;;
    *)
      echo "PyPI Simple index returned HTTP ${status}." >&2
      exit 1
      ;;
  esac
}

fetch_index

expected_directory="${PYPI_EXPECTED_DIRECTORY:-}"
if [ -n "${expected_directory}" ]; then
  if [ ! -d "${expected_directory}" ]; then
    echo "Expected PyPI upload directory does not exist: ${expected_directory}" >&2
    exit 1
  fi

  attempt=1
  while [ "${attempt}" -le 10 ]; do
    missing=$(python3 - "${PYPI_INDEX_PATH}" "${expected_directory}" <<'PY'
import json
from pathlib import Path
import sys

index_path = Path(sys.argv[1])
expected_directory = Path(sys.argv[2])
try:
    document = json.loads(index_path.read_text(encoding="utf-8"))
except (OSError, json.JSONDecodeError) as error:
    print(f"Could not read PyPI Simple index JSON: {error}", file=sys.stderr)
    raise SystemExit(1) from error

files = document.get("files") if isinstance(document, dict) else None
if not isinstance(files, list):
    print("PyPI Simple index JSON is missing a files list.", file=sys.stderr)
    raise SystemExit(1)

expected = {
    path.name
    for path in expected_directory.iterdir()
    if path.is_file() and not path.name.endswith(".publish.attestation")
}
available = {
    item["filename"]
    for item in files
    if isinstance(item, dict) and isinstance(item.get("filename"), str)
}
print("\n".join(sorted(expected - available)))
PY
    )
    if [ -z "${missing}" ]; then
      exit 0
    fi
    if [ "${attempt}" -eq 10 ]; then
      printf 'PyPI Simple index did not list these published files after 10 checks:\n%s\n' \
        "${missing}" >&2
      exit 1
    fi
    sleep 3
    fetch_index
    attempt=$((attempt + 1))
  done
fi
