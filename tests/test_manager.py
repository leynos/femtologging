from femtologging import get_logger, reset_manager


def test_get_logger_singleton() -> None:
    reset_manager()
    a = get_logger("app.core")
    b = get_logger("app.core")
    assert a is b


def test_get_logger_different_names() -> None:
    reset_manager()
    first = get_logger("first")
    second = get_logger("second")
    assert first is not second


def test_get_logger_parents() -> None:
    reset_manager()
    c = get_logger("a.b.c")
    b = get_logger("a.b")
    a = get_logger("a")
    root = get_logger("root")

    assert c.parent == "a.b"
    assert b.parent == "a"
    assert a.parent == "root"
    assert root.parent is None


def test_get_logger_auto_creates_root() -> None:
    reset_manager()
    child = get_logger("child")
    root = get_logger("root")
    assert child.parent == "root"
    assert root.parent is None


def test_get_logger_invalid_names() -> None:
    reset_manager()
    for name in ["", ".bad", "bad.", "a..b"]:
        try:
            get_logger(name)
            msg = "expected ValueError"
            raise AssertionError(msg)
        except ValueError:
            pass
