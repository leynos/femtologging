"""BDD steps for dictConfig feature scenarios."""

from __future__ import annotations

from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import dictConfig, get_logger, reset_manager

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "dict_config.feature"))


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


@then("calling dictConfig with incremental true raises ValueError")
def dict_config_incremental_fails() -> None:
    with pytest.raises(ValueError, match="incremental configuration is not supported"):
        dictConfig({"version": 1, "incremental": True, "root": {}})


@when(
    parsers.parse('I configure dictConfig with handler class "{cls}"'),
    target_fixture="config_error",
)
def configure_with_handler_class(cls: str) -> ValueError:
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": cls}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(ValueError) as exc:
        dictConfig(cfg)
    return exc.value


@then("dictConfig raises ValueError")
def dict_config_raises_value_error(config_error: ValueError) -> None:
    assert isinstance(config_error, ValueError)
