Feature: Rust tracing compatibility
  As a library user
  I want Rust tracing events to flow through femtologging
  So that hybrid Python/Rust applications share handlers and structured data

  Background:
    Given the logging system is reset

  Scenario: Rust tracing events are routed to handlers
    Given a stream handler attached to logger "rust.tracing.basic"
    When I set up rust tracing bridge
    And I emit a Rust tracing event "hello from tracing" at "INFO"
    Then the captured tracing stderr output matches snapshot

  Scenario: Rust tracing events respect FemtoLogger levels
    Given a stream handler attached to logger "rust.tracing.basic"
    When I set logger "rust.tracing.basic" level to "WARN"
    And I set up rust tracing bridge
    And I emit a Rust tracing event "debug message" at "DEBUG"
    Then the captured tracing stderr output matches snapshot

  Scenario: Structured tracing fields arrive in Python handle_record payloads
    Given a record-collecting handler attached to logger "rust.tracing.structured"
    When I set up rust tracing bridge
    And I emit a structured Rust tracing event
    Then the captured tracing records match snapshot

  Scenario: Nested span context arrives in Python handle_record payloads
    Given a record-collecting handler attached to logger "rust.tracing.span"
    When I set up rust tracing bridge
    And I emit a nested Rust tracing span event
    Then the captured tracing records match snapshot

  Scenario: Rust tracing bridge rejects a preinstalled global subscriber
    When I attempt to set up rust tracing bridge in a fresh process and it fails
    Then the rust tracing bridge error matches snapshot
