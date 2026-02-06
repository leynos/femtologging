Feature: Handler builders
  Scenario: build file handler builder
    Given a FileHandlerBuilder for path "test.log"
    When I set file capacity 10
    And I set flush after records 2
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

  Scenario: invalid file handler flush interval
    Given a FileHandlerBuilder for path "test.log"
    Then setting flush after records 0 fails

  Scenario: build rotating file handler builder
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set file capacity 10
    And I set flush after records 2
    And I set max bytes 1024
    And I set backup count 5
    And I set file formatter "default"
    Then the rotating file handler builder matches snapshot

  Scenario: dictConfig rotating file handler builder
    Given a dictConfig RotatingFileHandlerBuilder for path "test.log"
    When I set file capacity 10
    And I set flush after records 2
    And I set max bytes 1024
    And I set backup count 5
    And I set file formatter "default"
    Then the rotating file handler builder matches snapshot

  Scenario: dictConfig rotating builder zero thresholds
    Given a dictConfig RotatingFileHandlerBuilder for path "test.log"
    Then setting max bytes 0 fails with "max_bytes must be greater than zero"
    And setting backup count 0 fails with "backup_count must be greater than zero"

  Scenario: invalid rotating file handler capacity
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set file capacity 0
    Then building the rotating file handler fails with "capacity must be greater than zero"

  Scenario: invalid rotating file handler zero max bytes
    Given a RotatingFileHandlerBuilder for path "test.log"
    Then setting max bytes 0 fails with "max_bytes must be greater than zero"

  Scenario: invalid rotating file handler zero backup count
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set max bytes 1024
    Then setting backup count 0 fails with "backup_count must be greater than zero"

  Scenario: invalid rotating file handler zero thresholds
    Given a RotatingFileHandlerBuilder for path "test.log"
    Then setting max bytes 0 fails with "max_bytes must be greater than zero"
    And setting backup count 0 fails with "backup_count must be greater than zero"

  Scenario: missing rotating backup count
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set max bytes 1024
    Then building the rotating file handler fails with "backup_count must be provided when max_bytes is set"

  Scenario: missing rotating max bytes
    Given a RotatingFileHandlerBuilder for path "test.log"
    When I set backup count 2
    Then building the rotating file handler fails with "max_bytes must be provided when backup_count is set"

  Scenario: build stream handler builder
    Given a StreamHandlerBuilder targeting stdout
    When I set stream capacity 8
    And I set stream formatter "default"
    Then the stream handler builder matches snapshot

  Scenario: invalid stream handler capacity
    Given a StreamHandlerBuilder targeting stderr
    When I set stream capacity 0
    Then building the stream handler fails

  Scenario: invalid stream handler flush after ms
    Given a StreamHandlerBuilder targeting stdout
    Then setting stream flush after ms 0 fails

  Scenario: build stream handler builder with flush after ms
    Given a StreamHandlerBuilder targeting stdout
    When I set stream flush after ms 250
    And I set stream formatter "default"
    Then the stream handler builder matches snapshot

  Scenario: invalid stream handler negative flush after ms
    Given a StreamHandlerBuilder targeting stdout
    Then setting stream flush after ms -1 fails

  Scenario: build socket handler builder for tcp
    Given a SocketHandlerBuilder for host "127.0.0.1" port 9020
    When I set socket capacity 8
    And I set socket connect timeout 500
    And I set socket write timeout 250
    And I set socket max frame size 2048
    And I set socket tls domain "example.com"
    Then the socket handler builder matches snapshot

  Scenario: socket handler builder requires transport
    Given an empty SocketHandlerBuilder
    Then building the socket handler fails with "socket handler requires a transport"

  Scenario: build HTTP handler builder
    Given an HTTPHandlerBuilder for URL "http://localhost:8080/log"
    When I set HTTP method POST
    And I set HTTP connect timeout 1000
    And I set HTTP write timeout 5000
    Then the HTTP handler builder matches snapshot

  Scenario: HTTP handler builder with JSON format
    Given an HTTPHandlerBuilder for URL "http://localhost:8080/log"
    When I enable JSON format
    Then the JSON HTTP handler builder matches snapshot

  Scenario: HTTP handler builder with basic auth
    Given an HTTPHandlerBuilder for URL "http://localhost:8080/log"
    When I set basic auth user "admin" password "secret"
    Then the HTTP handler builder with auth matches snapshot

  Scenario: HTTP handler builder with bearer token
    Given an HTTPHandlerBuilder for URL "http://localhost:8080/log"
    When I set bearer token "my-api-token"
    Then the HTTP handler builder with bearer matches snapshot

  Scenario: HTTP handler builder with record fields
    Given an HTTPHandlerBuilder for URL "http://localhost:8080/log"
    When I set record fields to "name,msg,levelname"
    Then the HTTP handler builder with fields matches snapshot

  Scenario: HTTP handler requires URL
    Given an empty HTTPHandlerBuilder
    Then building the HTTP handler fails with "HTTP handler requires a URL"
