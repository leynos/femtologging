Feature: ConfigBuilder
  Scenario: build simple configuration
    Given a ConfigBuilder
    When I add formatter "fmt" with format "{level} {message}"
    And I add logger "core" with level "INFO"
    And I set root logger with level "WARN"
    Then the configuration matches snapshot
    And the configuration is built and initialized

  Scenario: unsupported version
    Given a ConfigBuilder
    When I set version 2
    Then building the configuration fails

  Scenario: attach handler to multiple loggers
    Given a ConfigBuilder
    When I add stream handler "console" targeting "stderr"
    And I add logger "core" with handler "console"
    And I add logger "worker" with handler "console"
    And I set root logger with level "INFO"
    Then the configuration matches snapshot
    And the configuration is built and initialized
    And loggers "core" and "worker" share handler "console"

  Scenario: unknown handler id
    Given a ConfigBuilder
    When I add logger "core" with handler "missing"
    And I set root logger with level "INFO"
    Then building the configuration fails with key error containing "missing"
