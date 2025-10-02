Feature: Rotating file handler size-based rotation

  Scenario: rotates when the next record exceeds the byte budget
    Given a rotating handler with max bytes 30 and backup count 1
    When I log record "first message" at level "INFO" for logger "rotate"
    And I log record "second message" at level "INFO" for logger "rotate"
    And I close the rotating handler
    Then the rotating log files match snapshot

  Scenario: rotates when a single record exceeds the byte budget
    Given a rotating handler with max bytes 10 and backup count 1
    When I log record "overlong message" at level "INFO" for logger "rotate"
    And I close the rotating handler
    Then the rotating log files match snapshot

  Scenario: respects multibyte characters when evaluating record sizes
    Given a rotating handler with max bytes 34 and backup count 1
    When I log record "emoji 😀" at level "INFO" for logger "rotate"
    And I log record "emoji 😁" at level "INFO" for logger "rotate"
    And I close the rotating handler
    Then the rotating log files match snapshot

  Scenario: leaves logs intact when rotation is disabled
    Given a rotating handler with max bytes 0 and backup count 0
    When I log record "first message" at level "INFO" for logger "rotate"
    And I log record "second message" at level "INFO" for logger "rotate"
    And I close the rotating handler
    Then the rotating log files match snapshot
