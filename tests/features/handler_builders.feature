Feature: Handler builders
  Scenario: build file handler builder
    Given a FileHandlerBuilder for path "test.log"
    When I set file capacity 10
    And I set flush record interval 2
    And I set file formatter "default"
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

  Scenario: build rotating file handler builder
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set file capacity 10
    And I set flush record interval 2
    And I set max bytes 1024
    And I set backup count 5
    And I set file formatter "default"
    Then the rotating file handler builder matches snapshot

  Scenario: invalid rotating file handler capacity
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set file capacity 0
    Then building the rotating file handler fails

  Scenario: invalid rotating file handler zero max bytes
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set max bytes 0
    And I set backup count 1
    Then building the rotating file handler fails

  Scenario: invalid rotating file handler zero backup count
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set max bytes 1024
    And I set backup count 0
    Then building the rotating file handler fails

  Scenario: missing rotating backup count
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set max bytes 1024
    Then building the rotating file handler fails

  Scenario: missing rotating max bytes
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set backup count 2
    Then building the rotating file handler fails

  Scenario: build stream handler builder
    Given a StreamHandlerBuilder targeting stdout
    When I set stream capacity 8
    And I set stream formatter "default"
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
    And I set stream formatter "default"
    Then the stream handler builder matches snapshot

  Scenario: invalid stream handler negative flush timeout
    Given a StreamHandlerBuilder targeting stdout
    Then setting stream flush timeout -1 fails
