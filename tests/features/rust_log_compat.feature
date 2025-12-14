Feature: Rust log crate compatibility
  As a library user
  I want Rust `log` crate records to flow through femtologging
  So that hybrid Python/Rust applications share handlers

  Background:
    Given the logging system is reset

  Scenario: Rust records are routed to handlers
    Given a stream handler attached to logger "rust.test"
    When I set up rust logging bridge
    And I emit a Rust log "hello from rust" at "INFO" with target "rust.test"
    Then the captured stderr output matches snapshot

  Scenario: Rust records respect FemtoLogger levels
    Given a stream handler attached to logger "rust.level"
    When I set logger "rust.level" level to "WARN"
    And I set up rust logging bridge
    And I emit a Rust log "debug message" at "DEBUG" with target "rust.level"
    Then the captured stderr output matches snapshot

  Scenario: Rust logging bridge is idempotent
    Given a stream handler attached to logger "rust.idempotent"
    When I set up rust logging bridge
    And I set up rust logging bridge
    And I emit a Rust log "second install" at "INFO" with target "rust.idempotent"
    Then the captured stderr output matches snapshot

  Scenario: Rust logging bridge rejects preinstalled global logger
    When I attempt to set up rust logging bridge in a fresh process and it fails
    Then the rust logging bridge error matches snapshot
