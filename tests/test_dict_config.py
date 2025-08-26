from __future__ import annotations

from femtologging import dictConfig, get_logger, reset_manager
from pathlib import Path
from pytest_bdd import given, parsers, scenarios, then, when
import pytest

scenarios("features/dict_config.feature")


@given("the logging system is reset")
def reset_logging() -> None:
    reset_manager()


@when("I configure dictConfig with a stream handler")
def configure_dict_config() -> None:
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler"}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    dictConfig(cfg)


@then(parsers.parse('logging "{msg}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(msg: str, level: str, snapshot) -> None:
    logger = get_logger("root")
    assert logger.log(level, msg) == snapshot


@then("calling dictConfig with incremental true fails")
def dict_config_incremental_fails() -> None:
    with pytest.raises(ValueError, match="incremental configuration is not supported"):
        dictConfig({"version": 1, "incremental": True, "root": {}})


def test_dict_config_file_handler_args_kwargs(tmp_path: Path) -> None:
    """Verify args and kwargs are evaluated for handler construction."""
    reset_manager()
    path = tmp_path / "out.log"
    cfg = {
        "version": 1,
        "handlers": {
            "f": {
                "class": "femtologging.FileHandler",
                "args": f"('{path}',)",
                "kwargs": "{}",
            }
        },
        "root": {"level": "INFO", "handlers": ["f"]},
    }
    dictConfig(cfg)
    logger = get_logger("root")
    logger.log("INFO", "file")
    assert path.exists()


def test_dict_config_args_reject_bytes() -> None:
    reset_manager()
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler", "args": b"bytes"}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(ValueError, match="args must not be bytes or bytearray"):
        dictConfig(cfg)


@pytest.mark.parametrize(
    "config",
    [
        {"version": 2, "root": {}},
        {
            "version": 1,
            "handlers": {"h": {"class": "unknown"}},
            "root": {"handlers": ["h"]},
        },
        {"version": 1, "filters": {"f": {}}, "root": {}},
    ],
    ids=["version-unsupported", "handler-class-unknown", "filters-unsupported"],
)
def test_dict_config_invalid_configs(config: dict[str, object]) -> None:
    """Invalid configurations raise ``ValueError``."""
    reset_manager()
    with pytest.raises(ValueError):
        dictConfig(config)
