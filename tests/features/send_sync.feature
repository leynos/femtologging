@concurrency @send_sync
Feature: Send and Sync safety

  Background:
    Given a stream handler built for stderr

  Scenario Outline: stream handler processes logs from N threads
    When I log messages from <count> threads
    Then the captured output matches snapshot

    Examples:
      | count |
      | 1     |
      | 10    |
      | 100   |

  Scenario: closed stream handler drops records
    Given the handler is closed
    When I log a message after closing
    Then the captured output matches snapshot
