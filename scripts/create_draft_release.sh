#!/usr/bin/env bash
# Create a draft release only when the tag has no existing release.
set -euo pipefail
release_list_path="${RELEASE_TEMP_PATH}/releases.json"
gh api --paginate --slurp \
  "repos/${RELEASE_REPOSITORY}/releases" > "${release_list_path}"
release_state="$(python3 - "${release_list_path}" "${TAG_NAME}" <<'PY'
import json
from pathlib import Path
import sys

release_list = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
if not isinstance(release_list, list) or not all(
    isinstance(page, list) for page in release_list
):
    raise SystemExit("GitHub release list response is malformed.")
releases = [release for page in release_list for release in page]
matches = [
    release
    for release in releases
    if isinstance(release, dict)
    and release.get("tag_name") == sys.argv[2]
]
if len(matches) > 1:
    raise SystemExit(
        f"Multiple GitHub Releases use tag {sys.argv[2]}; "
        "remove duplicates before retrying."
    )
print("existing" if matches else "missing")
PY
)"
if [ "${release_state}" = existing ]; then
  echo "Reusing the existing release for ${TAG_NAME}."
else
  flags=(--draft --verify-tag --generate-notes --title "${TAG_NAME}")
  if [ "${IS_PRERELEASE}" = true ]; then
    flags+=(--prerelease)
  fi
  gh release create "${TAG_NAME}" "${flags[@]}"
fi
