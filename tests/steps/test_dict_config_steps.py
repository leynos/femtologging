"""BDD steps for dictConfig feature scenarios."""

from __future__ import annotations

from pathlib import Path

import pytest
from pytest_bdd import parsers, scenarios, then, when

from femtologging import dictConfig

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "dict_config.feature"))


@when("I configure dictConfig with a stream handler")
def configure_dict_config() -> None:
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler"}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    dictConfig(cfg)
    return  # noqa: PLR1711


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
    with pytest.raises(
        ValueError,
        match=r"handler.*class|unsupported .*handler|failed to construct handler",
    ) as exc:
        dictConfig(cfg)
    return exc.value


@then("dictConfig raises ValueError")
def dict_config_raises_value_error(config_error: ValueError) -> None:
    assert config_error, (
        "config_error fixture did not capture ValueError from dictConfig"
    )
