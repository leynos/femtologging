from femtologging import FemtoLevel


def test_enum_variants_exposed() -> None:
    assert isinstance(FemtoLevel.Info, FemtoLevel)
    assert isinstance(FemtoLevel.Error, FemtoLevel)
