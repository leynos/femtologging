Feature: Builder compatibility with legacy helpers
  Background:
    Given the logging system is reset

  Scenario: basicConfig matches ConfigBuilder output
    Given a canonical configuration example
    When I apply the builder configuration
    And I log "builder" at "INFO" capturing as "builder"
    And I reset the logging system
    And I call basicConfig with level "INFO" and stream stdout
    And I log "builder" at "INFO" capturing as "basic"
    Then the captured outputs match snapshot

  Scenario: dictConfig accepts ConfigBuilder schema
    Given a canonical configuration example
    When I apply the dictConfig schema
    Then logging "compat" at "INFO" from root matches snapshot

  Scenario: tampering with the schema removes required sections
    Given a canonical configuration example
    When I drop the root logger from the dictConfig schema
    Then applying the schema via dictConfig fails with "root"
