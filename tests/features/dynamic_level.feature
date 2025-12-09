Feature: Dynamic log level updates
  As a library user
  I want to change log levels at runtime
  So that I can adjust logging verbosity without restarting my application

  Background:
    Given a ConfigBuilder
    And I add stream handler "console" targeting "stderr"
    And I set root logger with level "TRACE"
    And the configuration is built and initialised

  Scenario: Level can be read after configuration
    Then logger "root" level is "TRACE"

  Scenario: Level can be changed at runtime
    When I set logger "root" level to "ERROR"
    Then logger "root" level is "ERROR"
    And logger "root" suppresses "INFO"
    And logger "root" emits "ERROR"

  Scenario: Level changes affect filtering immediately
    When I set logger "root" level to "WARN"
    Then logger "root" suppresses "INFO"
    And logger "root" emits "WARN"
    When I set logger "root" level to "DEBUG"
    Then logger "root" emits "INFO"
    And logger "root" emits "DEBUG"

  Scenario: Level state after set_level matches snapshot
    When I set logger "root" level to "ERROR"
    Then logger "root" level state matches snapshot
