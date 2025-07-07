from femtologging import get_logger


def test_get_logger_singleton() -> None:
    a = get_logger("app.core")
    b = get_logger("app.core")
    assert a is b


def test_get_logger_parents() -> None:
    c = get_logger("a.b.c")
    b = get_logger("a.b")
    a = get_logger("a")
    root = get_logger("root")

    assert c.parent == "a.b"
    assert b.parent == "a"
    assert a.parent == "root"
    assert root.parent is None
