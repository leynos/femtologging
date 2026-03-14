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

  Scenario: log_context adds structured metadata key-values
    Given a record-collecting logger named "ctx.logger" with level "INFO"
    When I call info with message "ctx log" and name "ctx.logger" inside context "request_id"="42" and "user"="alice"
    Then the latest record metadata key_values contain "request_id"="42" and "user"="alice"

  Scenario: inline fields override outer context values
    Given a record-collecting logger named "override.logger" with level "INFO"
    When I call info with message "inner" and name "override.logger" inside nested context "request_id"="outer" then "request_id"="inner" and "phase"="test"
    Then the latest record metadata key_values contain "request_id"="inner" and "phase"="test"

  Scenario: invalid context value type is rejected
    When I push log context with an invalid nested value
    Then a context error is raised containing "context values must be str, int, float, bool, or None"

  Scenario: context metadata output matches snapshot
    Given a record-collecting logger named "snap.logger" with level "INFO"
    When I call info with message "snap" and name "snap.logger" inside context "request_id"="123" and "user"="bob"
    Then the latest record metadata key_values match snapshot
