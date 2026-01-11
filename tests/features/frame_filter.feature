Feature: Frame filtering for stack traces
  Users should be able to filter frames from exc_info and stack_info payloads.

  Scenario: Filter out logging infrastructure frames from stack payload
    Given a stack_info payload with frames from "app.py", "femtologging/__init__.py", "logging/__init__.py"
    When I filter with exclude_logging=True
    Then the filtered payload has 1 frame
    And the filtered frame filename is "app.py"

  Scenario: Filter frames by filename pattern
    Given a stack_info payload with frames from "app.py", ".venv/lib/requests.py", "utils.py"
    When I filter with exclude_filenames=[".venv/"]
    Then the filtered payload has 2 frames
    And the filtered frames do not contain ".venv/"

  Scenario: Limit stack depth
    Given a stack_info payload with frames from "a.py", "b.py", "c.py", "d.py", "e.py"
    When I filter with max_depth=2
    Then the filtered payload has 2 frames
    And the filtered frames are "d.py", "e.py"

  Scenario: Filter exception payload with cause chain
    Given an exception payload with frames from "main.py", "femtologging/__init__.py"
    And the exception has a cause with frames from "cause.py", "logging/__init__.py"
    When I filter with exclude_logging=True
    Then the filtered payload has 1 frame
    And the filtered cause has 1 frame
    And the filtered cause frame filename is "cause.py"

  Scenario: Combined filters
    Given a stack_info payload with frames from "outer.py", ".venv/lib/foo.py", "inner.py", "femtologging/__init__.py"
    When I filter with exclude_logging=True, exclude_filenames=[".venv/"], max_depth=1
    Then the filtered payload has 1 frame
    And the filtered frame filename is "inner.py"

  Scenario: Get logging infrastructure patterns
    When I get the logging infrastructure patterns
    Then the patterns contain "femtologging"
    And the patterns contain "logging/__init__"
