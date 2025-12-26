Feature: Exception schema serialisation
  The exception schema types must serialise correctly for logging payloads.

  Scenario: Stack frame serialises to JSON
    Given a stack frame with filename "test.py" line 42 function "main"
    When I serialise the frame to JSON
    Then the JSON contains "filename" as "test.py"
    And the JSON contains "lineno" as 42
    And the JSON contains "function" as "main"
    And the JSON matches snapshot

  Scenario: Stack frame with optional fields
    Given a stack frame with all optional fields populated
    When I serialise the frame to JSON
    Then the JSON contains "end_lineno"
    And the JSON contains "colno"
    And the JSON contains "source_line"
    And the JSON contains "locals"
    And the JSON matches snapshot

  Scenario: Exception payload with cause chain
    Given an exception "RuntimeError" with message "operation failed"
    And the exception has cause "IOError" with message "read error"
    When I serialise the exception to JSON
    Then the JSON contains nested "cause" with "type_name" as "IOError"
    And the JSON matches snapshot

  Scenario: Exception group with nested exceptions
    Given an exception group "ExceptionGroup" with message "multiple errors"
    And the group contains exception "ValueError" with message "bad value"
    And the group contains exception "TypeError" with message "wrong type"
    When I serialise the exception to JSON
    Then the JSON contains "exceptions" array with 2 items
    And the JSON matches snapshot

  Scenario: Schema version is included
    Given an exception "Error" with message "test"
    When I serialise the exception to JSON
    Then the JSON contains "schema_version" as 1
