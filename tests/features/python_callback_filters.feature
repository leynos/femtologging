Feature: Python callback filters
  Scenario: callable filter enriches an emitted record
    Given a ConfigBuilder for python callback filters
    When I add python callback filter "context" using the enrich callback
    And I add logger "svc" with python filter "context"
    And I set the python callback root logger level to "DEBUG"
    Then the python callback filter configuration matches snapshot
    When the python callback filter configuration is built
    And I attach a record collector to logger "svc"
    And logger "svc" emits "INFO" with active request id "req-123"
    Then the collected record metadata contains "request_id" with value "req-123"

  Scenario: filter object suppresses a record
    Given a ConfigBuilder for python callback filters
    When I add python callback filter "reject" using the reject-all filter object
    And I add logger "svc" with python filter "reject"
    And I set the python callback root logger level to "DEBUG"
    When the python callback filter configuration is built
    And I attach a record collector to logger "svc"
    Then logger "svc" suppresses "INFO" through the python callback filter
