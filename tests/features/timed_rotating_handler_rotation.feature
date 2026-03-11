Feature: Timed rotating file handler rotation

  Scenario: rotates when the next write crosses a second boundary
    Given timed rotation test times:
      | timestamp           |
      | 2026-03-12T00:00:00 |
      | 2026-03-12T00:00:00 |
      | 2026-03-12T00:00:02 |
    And a timed rotating handler with when "S" interval 1 backup count 1 utc enabled
    When I log timed record "first message" at level "INFO" for logger "rotate"
    And I log timed record "second message" at level "INFO" for logger "rotate"
    And I close the timed rotating handler
    Then the timed rotating log files match snapshot

  Scenario: rotates at midnight with at_time
    Given timed rotation test times:
      | timestamp           |
      | 2026-03-11T23:59:59 |
      | 2026-03-11T23:59:59 |
      | 2026-03-12T00:00:01 |
    And a timed rotating handler with when "MIDNIGHT" interval 1 backup count 1 utc enabled at time "00:00:00"
    When I log timed record "first message" at level "INFO" for logger "rotate"
    And I log timed record "second message" at level "INFO" for logger "rotate"
    And I close the timed rotating handler
    Then the timed rotating log files match snapshot

  Scenario: backup count zero retains all timestamped files
    Given timed rotation test times:
      | timestamp           |
      | 2026-03-12T00:00:00 |
      | 2026-03-12T00:00:00 |
      | 2026-03-12T00:00:02 |
      | 2026-03-12T00:00:04 |
    And a timed rotating handler with when "S" interval 1 backup count 0 utc enabled
    When I log timed record "first message" at level "INFO" for logger "rotate"
    And I log timed record "second message" at level "INFO" for logger "rotate"
    And I log timed record "third message" at level "INFO" for logger "rotate"
    And I close the timed rotating handler
    Then the timed rotating log files match snapshot
