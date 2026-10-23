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

  Scenario: Rust log_enabled respects a WARN target threshold
    Given a stream handler attached to logger "rust.enabled.warn"
    When I set logger "rust.enabled.warn" level to "WARN"
    And I set logger "rust.enabled.trace" level to "TRACE"
    And I set up rust logging bridge
    Then the Rust log facade maximum is "TRACE"
    And the Rust log levels "TRACE,DEBUG,INFO" are disabled for target "rust.enabled.warn"
    And the Rust log levels "WARN,ERROR" are enabled for target "rust.enabled.warn"

  Scenario: Rust module targets are normalized for log_enabled
    Given a stream handler attached to logger "rust.module.path"
    When I set logger "rust.module.path" level to "WARN"
    And I set up rust logging bridge
    Then the Rust log levels "DEBUG" are disabled for target "rust::module::path"

  Scenario: Rust log_enabled follows dynamic level changes
    Given a stream handler attached to logger "rust.enabled.dynamic"
    When I set logger "rust.enabled.dynamic" level to "WARN"
    And I set up rust logging bridge
    Then the Rust log facade maximum is "INFO"
    And the Rust log levels "DEBUG" are disabled for target "rust.enabled.dynamic"
    When I set logger "rust.enabled.dynamic" level to "DEBUG"
    Then the Rust log facade maximum is "DEBUG"
    And the Rust log levels "DEBUG" are enabled for target "rust.enabled.dynamic"
    When I set logger "rust.enabled.dynamic" level to "ERROR"
    Then the Rust log facade maximum is "INFO"
    And the Rust log levels "DEBUG" are disabled for target "rust.enabled.dynamic"
    And the Rust log levels "ERROR" are enabled for target "rust.enabled.dynamic"

  Scenario: Rust log facade maximum follows config and runtime mutation
    Given a stream handler attached to logger "rust.enabled.configured"
    When I set logger "rust.enabled.configured" level to "WARN"
    And I set up rust logging bridge
    When I configure logger "rust.enabled.configured" to "DEBUG" via build_and_init
    Then the Rust log facade maximum is "DEBUG"
    And the Rust log levels "DEBUG" are enabled for target "rust.enabled.configured"
    When I set logger "rust.enabled.configured" level to "ERROR" via runtime mutation
    Then the Rust log facade maximum is "INFO"
    And the Rust log levels "DEBUG" are disabled for target "rust.enabled.configured"
    And the Rust log levels "ERROR" are enabled for target "rust.enabled.configured"

  Scenario: Resetting the manager restores the default facade maximum
    Given a stream handler attached to logger "rust.enabled.reset"
    When I set logger "rust.enabled.reset" level to "DEBUG"
    And I set up rust logging bridge
    Then the Rust log facade maximum is "DEBUG"
    When I reset the logger manager
    Then the Rust log facade maximum is "INFO"
    And the Rust log levels "DEBUG" are disabled for target "rust.enabled.reset"

  Scenario: Rust logging bridge is idempotent
    Given a stream handler attached to logger "rust.idempotent"
    When I set up rust logging bridge
    And I set up rust logging bridge
    And I emit a Rust log "second install" at "INFO" with target "rust.idempotent"
    Then the captured stderr output matches snapshot

  Scenario: Rust logging bridge rejects preinstalled global logger
    When I attempt to set up rust logging bridge in a fresh process and it fails
    Then the rust logging bridge error matches snapshot
