"""Test the release verification checks in scripts/release_wheel_probe.py.

The tests cover rejected invalid installations, accepted production wheels, the
HTTP delivery check, and the delivery of a probe record through the native HTTP
handler. The delivery check must reach a loopback endpoint, so it is exercised
against both malformed and absent deliveries as well as the real handler. The
sdist verification and rebuild steps in .github/workflows/build-wheels.yml are
covered here too.
"""

from __future__ import annotations

import importlib.machinery
import io
import queue
import sys
import tarfile
import typing as typ
from pathlib import Path
from types import SimpleNamespace

import pytest

import femtologging
from scripts import release_wheel_probe
from tests import conftest
from tests.make_contract_helpers import sole_workflow_step
from tests.release_workflow_helpers import run_bash

_REQUIRED = {
    "README.md",
    "pyproject.toml",
    "femtologging/__init__.py",
    "femtologging/_rust_compat.py",
    "rust_extension/Cargo.toml",
    "rust_extension/Cargo.lock",
    "rust_extension/src/lib.rs",
    "rust_extension/src/python_module.rs",
}


@pytest.mark.skipif(sys.platform == "win32", reason="The checked-in step uses Bash.")
@pytest.mark.parametrize(
    ("missing", "features", "expected"),
    [
        (None, '["python"]', None),
        ("rust_extension/src/lib.rs", '["python"]', "missing required members"),
        (None, '["python", "test-util"]', "only the python release feature"),
    ],
)
def test_checked_in_sdist_verification(
    tmp_path: Path, missing: str | None, features: str, expected: str | None
) -> None:
    """The actual workflow step accepts production archives and rejects defects."""
    archive = tmp_path / "source.tar.gz"
    with tarfile.open(archive, "w:gz") as source:
        for name in sorted(_REQUIRED - ({missing} if missing else set())):
            contents = (
                f"[tool.maturin]\nfeatures = {features}\n".encode()
                if name == "pyproject.toml"
                else b"fixture"
            )
            info = tarfile.TarInfo(f"package/{name}")
            info.size = len(contents)
            source.addfile(info, io.BytesIO(contents))
    step = sole_workflow_step(
        ".github/workflows/build-wheels.yml",
        "verify-wheel-install",
        "Check source distribution members and release features",
    )
    result = run_bash(str(step["run"]), tmp_path, {"SDIST_PATH": str(archive)})
    if expected is None:
        assert result.returncode == 0, result.stderr
    else:
        assert result.returncode != 0, "Invalid input must fail verification."
        assert expected in result.stderr, "Report the specific archive defect."


_UNEXPECTED_DELIVERY = "the delivery check ran before the identity checks passed"


def _unexpected_delivery_check() -> list[str]:
    """Fail a test whose setup must not reach the HTTP delivery check."""
    raise AssertionError(_UNEXPECTED_DELIVERY)


def _stub_production_extension(monkeypatch: pytest.MonkeyPatch) -> None:
    """Make the identity checks see a production extension inside ``sys.prefix``."""
    suffix = importlib.machinery.EXTENSION_SUFFIXES[0]
    origin = Path(sys.prefix) / "lib" / f"_femtologging_rs{suffix}"
    native = SimpleNamespace(__file__=str(origin))
    monkeypatch.setattr(
        release_wheel_probe.importlib, "import_module", lambda _: native
    )
    monkeypatch.setattr(femtologging, "_has_test_util", False)


@pytest.mark.parametrize(
    "case",
    [
        (None, False, False, "no filesystem origin"),
        ("/outside/native.so", False, False, "outside sys.prefix"),
        ("invalid.py", False, False, "extension suffix"),
        ("invalid.py", True, False, "exposes test hooks"),
        ("invalid.py", False, True, "_has_test_util must be False"),
    ],
)
def test_release_probe_rejects_invalid_installations(
    monkeypatch: pytest.MonkeyPatch,
    case: tuple[str | None, bool, bool, str],
) -> None:
    """Invalid module origins and test-enabled builds fail the probe."""
    origin, hooks, has_util, expected = case
    native = SimpleNamespace(__file__=origin)
    if hooks:
        native.set_timed_rotation_test_times_for_test = lambda: None
    monkeypatch.setattr(
        release_wheel_probe.importlib, "import_module", lambda _: native
    )
    monkeypatch.setattr(femtologging, "_has_test_util", has_util)
    # A defective installation must not reach the network at all.
    monkeypatch.setattr(
        release_wheel_probe, "http_behaviour_errors", _unexpected_delivery_check
    )
    assert any(
        expected in error for error in release_wheel_probe.release_wheel_errors()
    ), "Report the specific invalid installation."


def test_release_probe_accepts_a_production_wheel(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    """A native extension inside the environment with no hooks passes the probe."""
    _stub_production_extension(monkeypatch)
    deliveries: list[bool] = []

    def record_delivery_check() -> list[str]:
        """Record that the probe checked delivery, without using the network."""
        deliveries.append(True)
        return []

    monkeypatch.setattr(
        release_wheel_probe, "http_behaviour_errors", record_delivery_check
    )

    assert not release_wheel_probe.release_wheel_errors(), (
        "A production native extension reports no violations."
    )
    assert deliveries == [True], "The probe must verify delivery as well as location."
    assert release_wheel_probe.main() == 0, "The probe exits successfully."
    assert capsys.readouterr().out == "release wheel probe passed\n", (
        "The probe reports its success on standard output."
    )


def test_release_probe_reports_a_delivery_failure(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    """A wheel whose native worker cannot deliver a record fails the probe."""
    _stub_production_extension(monkeypatch)
    monkeypatch.setattr(
        release_wheel_probe,
        "http_behaviour_errors",
        lambda: ["no HTTP delivery reached http://127.0.0.1:1/probe"],
    )

    assert release_wheel_probe.main() == 1, "A delivery failure must exit non-zero."
    assert "no HTTP delivery reached" in capsys.readouterr().err, (
        "Report the specific delivery failure."
    )


def test_release_probe_delivers_a_record_through_the_native_handler() -> None:
    """The installed extension's native HTTP worker delivers the probe record."""
    assert not release_wheel_probe.http_behaviour_errors(), (
        "The handler built from this installation must deliver its record."
    )


def _collector_receiving(pending: tuple[str, bytes] | None) -> SimpleNamespace:
    """Build a collector stand-in that holds at most one delivered request."""
    requests: queue.Queue[tuple[str, bytes]] = queue.Queue()
    if pending is not None:
        requests.put(pending)
    return SimpleNamespace(requests=requests)


@pytest.mark.parametrize(
    ("delivered", "expected"),
    [
        (("/other", b"release-wheel-probe"), "wrong request path"),
        (("/probe", b"unrelated"), "did not contain the record message"),
    ],
)
def test_collect_delivery_reports_a_malformed_delivery(
    delivered: tuple[str, bytes], expected: str
) -> None:
    """A request on the wrong path or without the record message is a failure."""
    errors = release_wheel_probe._collect_delivery(
        typ.cast(
            "release_wheel_probe._RecordCollector", _collector_receiving(delivered)
        ),
        "http://127.0.0.1:1/probe",
    )

    assert any(expected in error for error in errors), (
        "Report the specific defect in the delivered request."
    )


def test_collect_delivery_reports_an_absent_delivery(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A collector that receives nothing names the endpoint it waited on."""
    monkeypatch.setattr(release_wheel_probe, "_DELIVERY_TIMEOUT_SECONDS", 0.01)

    errors = release_wheel_probe._collect_delivery(
        typ.cast("release_wheel_probe._RecordCollector", _collector_receiving(None)),
        "http://127.0.0.1:1/probe",
    )

    assert errors == ["no HTTP delivery reached http://127.0.0.1:1/probe"], (
        "A missing delivery must name the endpoint the handler was given."
    )


def test_promised_test_hooks_missing_exits_with_usage_error(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A test run promising unavailable hooks fails before collection."""
    monkeypatch.setenv("FEMTOLOGGING_EXPECT_TEST_UTIL", "1")
    monkeypatch.setattr(femtologging, "_has_test_util", False)
    with pytest.raises(pytest.exit.Exception, match="expected test-util hooks") as exc:
        conftest.pytest_configure(typ.cast("pytest.Config", None))
    assert exc.value.returncode == pytest.ExitCode.USAGE_ERROR, (
        "Missing promised hooks must produce a usage error."
    )


@pytest.mark.skipif(sys.platform == "win32", reason="The checked-in step uses Bash.")
@pytest.mark.parametrize("wheel_count", [0, 1, 2])
def test_checked_in_sdist_rebuild_requires_one_wheel(
    tmp_path: Path, wheel_count: int
) -> None:
    """The real rebuild step rejects missing or duplicate build outputs."""
    import os

    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    python = bin_dir / "python3.12"
    python.write_text(
        '#!/bin/bash\nmkdir -p "$3/bin"\n'
        'printf "#!/bin/bash\\nexit 0\\n" > "$3/bin/python"\n'
        'chmod +x "$3/bin/python"\n',
        encoding="utf-8",
    )
    uv = bin_dir / "uv"
    uv.write_text(
        '#!/bin/bash\nif [ "$1" = build ]; then\n'
        '  for ((i=0; i<WHEEL_COUNT; i++)); do touch "$5/$i.whl"; done\n'
        "fi\n",
        encoding="utf-8",
    )
    python.chmod(0o755)
    uv.chmod(0o755)
    step = sole_workflow_step(
        ".github/workflows/build-wheels.yml",
        "verify-wheel-install",
        "Rebuild the sdist and install it in a fresh environment",
    )
    result = run_bash(
        str(step["run"]),
        tmp_path,
        {
            "PATH": f"{bin_dir}{os.pathsep}{os.defpath}",
            "WHEEL_RUNNER_TEMP": str(tmp_path),
            "SDIST_PATH": "source.tar.gz",
            "PROBE_PATH": "probe.py",
            "WHEEL_COUNT": str(wheel_count),
        },
    )
    if wheel_count == 1:
        assert result.returncode == 0, result.stderr
    else:
        assert result.returncode != 0, "Invalid input must fail verification."
        assert "Expected exactly one wheel rebuilt from the sdist" in result.stderr, (
            "Report the invalid rebuilt wheel count."
        )
