Feature: Module-level logging convenience functions
  As a Python developer
  I want to call femtologging.info(), debug(), warn(), error()
  So that I can log messages without manually retrieving a logger

  Background:
    Given the logging system is reset

  Scenario: info logs a message via the root logger
    When I call info with message "hello from info"
    Then the result is not None
    And the result contains "hello from info"

  Scenario: debug is suppressed when root level is INFO
    When I call debug with message "should be suppressed"
    Then the result is None

  Scenario: error logs a message at ERROR level
    When I call error with message "something went wrong"
    Then the result is not None
    And the result contains "something went wrong"

  Scenario: warn logs a message at WARN level
    When I call warn with message "disk space low"
    Then the result is not None
    And the result contains "disk space low"

  Scenario: named logger is used when name is provided
    Given a logger named "app.service" with level "WARN"
    When I call info with message "should be filtered" and name "app.service"
    Then the result is None
    When I call warn with message "visible warning" and name "app.service"
    Then the result is not None

  Scenario: info output matches snapshot
    When I call info with message "snapshot test"
    Then the info result matches snapshot

  Scenario: source location capture does not error
    When I call info with message "located call"
    Then the result is not None
    And the result format is "root [INFO] located call"
