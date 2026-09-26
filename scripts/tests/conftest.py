"""Collection policy for the script test suites.

``lint_rust_lanes.py`` is a self-contained ``uv`` script: its dependencies are
declared in its own script block rather than in the project's dev group, per
the estate scripting standards. The project virtual environment therefore
cannot import it, and collecting its tests there fails outright.

``make lint-lanes-test`` runs those tests in the script's own environment and
sets ``LINT_LANES_TEST``. Every other runner skips them, so the project suite
neither fails on the import nor pretends to have covered the driver.
``test_run_loom_models.py`` follows the same rule under ``LOOM_RUNNER_TEST``,
which ``make loom-runner-test`` sets.
"""

from __future__ import annotations

import os

collect_ignore: list[str] = []

if not os.environ.get("LINT_LANES_TEST"):
    collect_ignore.append("test_lint_rust_lanes.py")
if not os.environ.get("LOOM_RUNNER_TEST"):
    collect_ignore.append("test_run_loom_models.py")
