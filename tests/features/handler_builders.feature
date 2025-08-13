Feature: Handler builders
  Scenario: build file handler builder
    Given a FileHandlerBuilder for path "test.log"
    When I set file capacity 10
    And I set flush record interval 2
    Then the file handler builder matches snapshot

  Scenario: file handler builder with timeout overflow policy
    Given a FileHandlerBuilder for path "test.log"
    When I set overflow policy to timeout with 500ms
    Then the file handler builder with timeout overflow matches snapshot

  Scenario: invalid file handler capacity
    Given a FileHandlerBuilder for path "test.log"
    When I set file capacity 0
    Then building the file handler fails

  Scenario: invalid file handler flush record interval
    Given a FileHandlerBuilder for path "test.log"
    When I set flush record interval 0
    Then building the file handler fails

  Scenario: build stream handler builder
    Given a StreamHandlerBuilder targeting stdout
    When I set stream capacity 8
    Then the stream handler builder matches snapshot

  Scenario: invalid stream handler capacity
    Given a StreamHandlerBuilder targeting stderr
    When I set stream capacity 0
    Then building the stream handler fails

  Scenario: invalid stream handler flush timeout
    Given a StreamHandlerBuilder targeting stdout
    When I set stream flush timeout 0
    Then building the stream handler fails

  Scenario: build stream handler builder with flush timeout
    Given a StreamHandlerBuilder targeting stdout
    When I set stream flush timeout 250
    Then the stream handler builder matches snapshot
