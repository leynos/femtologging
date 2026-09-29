"""Hold build-test's uv cache to a key that cannot cross runner or interpreter.

A restored uv `environments-v2` entry is bound to the interpreter that built
it. generate-coverage's own uv cache keys on the operating system and the
pyproject hash alone, so on a warm Ubicloud run it handed a lane an
environment `uv venv` could not use (leynos/shared-actions#547). The caller
therefore owns the cache, and these contracts hold it to that shape.
"""

from __future__ import annotations

from tests.test_runner_placement_contract import Document, load_documents


def _build_test_steps() -> list[Document]:
    """Return the `build-test` job's steps from the real `ci.yml`.

    Returns
    -------
    list[Document]
        The step mappings, in order.
    """
    job = load_documents()["ci.yml"]["jobs"]["build-test"]
    return [step for step in job["steps"] if isinstance(step, dict)]


def test_the_uv_cache_is_keyed_on_the_runner_and_the_python_version() -> None:
    """Name the runner environment and Python version in the key and restore-keys.

    The interpreter-bound `environments-v2` directory is not cached at all.
    """
    steps = _build_test_steps()
    caches = [step for step in steps if step.get("name") == "Cache uv"]
    assert len(caches) == 1, f"expected one 'Cache uv' step, found {len(caches)}"
    settings = caches[0].get("with", {})
    key = str(settings.get("key", ""))
    assert "runner.environment" in key, f"the uv cache key ignores the runner: {key}"
    assert "matrix.python-version" in key, f"the uv cache key ignores Python: {key}"
    restore = str(settings.get("restore-keys", ""))
    assert "runner.environment" in restore, f"restore-keys ignore the runner: {restore}"
    assert "matrix.python-version" in restore, f"restore-keys ignore Python: {restore}"
    paths = str(settings.get("path", "")).split()
    assert "!~/.cache/uv/environments-v2" in paths, (
        f"environments-v2 is cached: {paths}"
    )


def test_generate_coverage_leaves_the_uv_cache_to_the_caller() -> None:
    """Refuse the action's own uv cache, so the keyed one above is the only one."""
    coverage = [
        step
        for step in _build_test_steps()
        if "generate-coverage" in str(step.get("uses", ""))
    ]
    assert len(coverage) == 1, f"expected one generate-coverage step: {coverage}"
    provider = coverage[0].get("with", {}).get("cache-provider")
    assert provider == "external", f"the action still owns the uv cache: {provider}"
