Feature: Stack info logging
  Loggers should support stack_info parameter for stack trace logging.

  Scenario: Log with stack_info=True includes call stack
    Given a logger named "test"
    When I log at INFO with message "debug" and stack_info=True
    Then the formatted output contains "Stack (most recent call last)"
    And the formatted output matches snapshot

  Scenario: Log without stack_info excludes stack
    Given a logger named "test"
    When I log at INFO with message "normal"
    Then the formatted output equals "test [INFO] normal"

  Scenario: Log with both exc_info and stack_info
    Given a logger named "test"
    And an active ValueError exception with message "oops"
    When I log at ERROR with message "debug" and exc_info=True and stack_info=True
    Then the formatted output contains "Stack (most recent call last)"
    And the formatted output contains "ValueError"
