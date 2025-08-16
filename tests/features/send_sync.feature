Feature: Send and Sync safety

  Scenario: stream handler processes logs from multiple threads
    Given a stream handler built for stderr
    When I log messages from 3 threads
    Then the captured output matches snapshot

  Scenario: closed stream handler drops records
    Given a stream handler built for stderr
    And the handler is closed
    When I log a message
    Then the captured output matches snapshot
