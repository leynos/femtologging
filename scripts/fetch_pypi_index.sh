#!/usr/bin/env bash
set -euo pipefail

: "${PYPI_SIMPLE_URL:?PYPI_SIMPLE_URL must be set}"
: "${PYPI_INDEX_PATH:?PYPI_INDEX_PATH must be set}"
allow_missing="${PYPI_ALLOW_MISSING:-false}"

attempt=0
status=000
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
