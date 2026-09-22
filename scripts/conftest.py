"""Collection policy for the helper scripts themselves.

``make test`` runs pytest with ``--doctest-modules``, so every module pytest
walks is imported, not just the ``test_*.py`` files. That is what makes the
docstring examples in these helpers load-bearing rather than decorative.

``lint_rust_lanes.py`` is a self-contained ``uv`` script: its dependencies are
declared in its own script block rather than in the project's dev group, per
the estate scripting standards, so the project virtual environment cannot
import it. ``make lint-lanes-test`` runs it, and its examples, in the script's
own environment. Collecting it here would fail on the import alone, so it is
excluded for the same reason `[tool.ty.src]` excludes it.

The remaining helpers import each other as top-level modules; the
``pythonpath`` setting in ``[tool.pytest.ini_options]`` is what lets them.
"""

from __future__ import annotations

collect_ignore: list[str] = ["lint_rust_lanes.py"]
