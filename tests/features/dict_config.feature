Feature: dictConfig

  Scenario: configure root logger via dictConfig
    Given the logging system is reset
    When I configure dictConfig with a stream handler
    Then logging "hello" at "INFO" from root matches snapshot

  Scenario: incremental configuration is rejected
    Given the logging system is reset
    Then calling dictConfig with incremental true fails
