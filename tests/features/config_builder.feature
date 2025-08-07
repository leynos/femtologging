Feature: ConfigBuilder
  Scenario: build simple configuration
    Given a ConfigBuilder
    When I add formatter "fmt" with format "{level} {message}"
    And I add logger "core" with level "INFO"
    And I set root logger with level "WARN"
    Then the configuration matches snapshot

  Scenario: unsupported version
    Given a ConfigBuilder
    When I set version 2
    Then building the configuration fails
