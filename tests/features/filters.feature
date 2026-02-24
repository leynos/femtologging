Feature: Filters
  Scenario: level filter suppresses high severity
    Given a ConfigBuilder
    When I add stream handler "console" targeting "stderr"
    And I add level filter "only_info" with max level "INFO"
    And I add logger "core" with handler "console" and filter "only_info"
    And I set root logger with level "DEBUG"
    Then the configuration matches snapshot
    And the configuration is built and initialized
    And logger "core" emits "INFO"
    And logger "core" suppresses "ERROR"

  Scenario: name filter matches prefix
    Given a ConfigBuilder
    When I add stream handler "console" targeting "stderr"
    And I add name filter "core_only" with prefix "core"
    And I add logger "core.child" with handler "console" and filter "core_only"
    And I add logger "other" with handler "console" and filter "core_only"
    And I set root logger with level "DEBUG"
    Then the configuration matches snapshot
    And the configuration is built and initialized
    And logger "core.child" emits "INFO"
    And logger "other" suppresses "INFO"

  Scenario: unknown filter id
    Given a ConfigBuilder
    When I add logger "core" with filter "missing"
    And I set root logger with level "INFO"
    Then building the configuration fails with key error containing "missing"
