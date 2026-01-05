Feature: Exception info logging
  Loggers should support exc_info parameter for exception logging.

  Scenario: Log with exc_info=True captures active exception
    Given a logger named "test"
    And an active ValueError exception with message "bad input"
    When I log at ERROR with message "failed" and exc_info=True
    Then the formatted output contains "ValueError"
    And the formatted output contains "bad input"
    And the formatted output matches snapshot

  Scenario: Log with exc_info=True and no active exception
    Given a logger named "test"
    When I log at INFO with message "no error" and exc_info=True
    Then the formatted output equals "test [INFO] no error"

  Scenario: Log with exception instance
    Given a logger named "test"
    And an exception instance KeyError with message "missing"
    When I log at ERROR with message "caught" and exc_info as the instance
    Then the formatted output contains "KeyError"
    And the formatted output matches snapshot

  Scenario: Log with chained exception cause
    Given a logger named "test"
    And an exception chain: RuntimeError from OSError
    When I log at ERROR with message "failed" and exc_info=True
    Then the formatted output contains "The above exception was the direct cause"
    And the formatted output contains "OSError"
    And the formatted output contains "RuntimeError"

  Scenario: Log with exception group
    Given a logger named "test"
    And an exception group with ValueError and TypeError
    When I log at ERROR with message "multiple" and exc_info=True
    Then the formatted output contains "ExceptionGroup"
    And the formatted output contains "ValueError"
    And the formatted output contains "TypeError"
