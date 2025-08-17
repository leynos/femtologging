@concurrency @send_sync
Feature: Send and Sync safety

  Background:
    Given a stream handler built for stderr

  Scenario: stream handler processes logs from multiple threads
    When I log messages from 3 threads
    Then the captured output matches snapshot

  Scenario: closed stream handler drops records
    Given the handler is closed
    When I log a message after closing
    Then the captured output matches snapshot
