Feature: Runtime handler and filter reconfiguration
  As a library user
  I want to mutate handlers and filters at runtime
  So that logging behaviour changes immediately without a rebuild

  Background:
    Given a runtime-configured logger named "core"

  Scenario: Appending a handler at runtime
    When I append runtime handler "stdout" targeting "stdout" to logger "core"
    Then logger "core" has 2 handlers
    And logger "core" runtime state matches snapshot

  Scenario: Replacing filters at runtime
    When I replace logger "core" filters with name filter "name" using prefix "core"
    Then logger "core" emits "ERROR"
    And logger "core" runtime state matches snapshot

  Scenario: Failed mutation preserves the prior state
    When I try to replace logger "core" filters with missing id "missing"
    Then the runtime mutation fails with key error containing "missing"
    And logger "core" suppresses "ERROR"
    And logger "core" runtime state matches snapshot

  Scenario: Root logger can be mutated at runtime
    When I set root logger level to "ERROR" via runtime mutation
    Then logger "root" suppresses "INFO"
    And logger "root" emits "ERROR"

  Scenario: Clearing handlers and filters at runtime
    When logger "core" runtime handlers and filters are cleared
    Then logger "core" has no runtime handlers
    And logger "core" runtime state matches snapshot
