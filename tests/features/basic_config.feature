Feature: basicConfig convenience configuration
  Scenario: configure root logger with stream handler
    Given the logging system is reset
    When I call basicConfig with level "INFO"
    Then logging "hello" at "INFO" from root matches snapshot
    And root logger has 1 handler

  Scenario Outline: force removes existing handlers
    Given the logging system is reset
    And root logger has a handler
    When I call basicConfig with level "<level>" and force true
    Then root logger has 1 handler
    And logging "post-force" at "<level>" from root matches snapshot

    Examples:
      | level   |
      | INFO    |
      | WARNING |

  Scenario: filename and stream together is invalid
    Given the logging system is reset
    Then calling basicConfig with filename "log.txt" and stream stdout fails

  Scenario: handlers with stream or filename is invalid
    Given the logging system is reset
    Then calling basicConfig with handler "stream_handler" and stream stdout fails
    And calling basicConfig with handler "file_handler" and filename "log.txt" fails
