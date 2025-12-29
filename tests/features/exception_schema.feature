Feature: Exception schema serialization
  The exception schema types must serialize correctly for logging payloads.

  Scenario: Stack frame serializes to JSON
    Given a stack frame with filename "test.py" line 42 function "main"
    When I serialize the frame to JSON
    Then the JSON contains "filename" as "test.py"
    And the JSON contains "lineno" as 42
    And the JSON contains "function" as "main"
    And the JSON matches snapshot

  Scenario: Stack frame with optional fields
    Given a stack frame with all optional fields populated
    When I serialize the frame to JSON
    Then the JSON contains "end_lineno"
    And the JSON contains "colno"
    And the JSON contains "source_line"
    And the JSON contains "locals"
    And the JSON matches snapshot

  Scenario: Exception payload with cause chain
    Given an exception "RuntimeError" with message "operation failed"
    And the exception has cause "IOError" with message "read error"
    When I serialize the exception to JSON
    Then the JSON contains nested "cause" with "type_name" as "IOError"
    And the JSON matches snapshot

  Scenario: Exception group with nested exceptions
    Given an exception group "ExceptionGroup" with message "multiple errors"
    And the group contains exception "ValueError" with message "bad value"
    And the group contains exception "TypeError" with message "wrong type"
    When I serialize the exception to JSON
    Then the JSON contains "exceptions" array with 2 items
    And the JSON matches snapshot

  Scenario: Schema version is included
    Given an exception "Error" with message "test"
    When I serialize the exception to JSON
    Then the JSON contains "schema_version"
    And the schema version matches the Rust constant

  Scenario: Schema version constant is exported
    Then the EXCEPTION_SCHEMA_VERSION constant is accessible from Python
    And the constant value is a positive integer
