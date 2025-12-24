Feature: Logger propagation behaviour
  Log records should propagate from child loggers to their ancestors by default.
  When propagation is disabled, records stay with the logger that emitted them.

  Background:
    Given a clean manager state

  Scenario: default propagation sends child logs to root handler
    Given a file handler "root_handler" writing to a temporary file
    And a ConfigBuilder with root logger using handler "root_handler" at level "INFO"
    And a child logger "child" at level "INFO" without handlers
    When I build and initialise the configuration
    And I log "hello from child" at level "INFO" from logger "child"
    And I flush all loggers
    Then the root handler file contains "hello from child"

  Scenario: propagation disabled prevents records reaching root
    Given a file handler "root_handler" writing to a temporary file
    And a ConfigBuilder with root logger using handler "root_handler" at level "INFO"
    And a child logger "child" at level "INFO" without propagation
    When I build and initialise the configuration
    And I log "hidden message" at level "INFO" from logger "child"
    And I flush all loggers
    Then the root handler file does not contain "hidden message"

  Scenario: runtime toggle of propagation
    Given a file handler "root_handler" writing to a temporary file
    And a ConfigBuilder with root logger using handler "root_handler" at level "INFO"
    And a child logger "child" at level "INFO" without handlers
    When I build and initialise the configuration
    And I disable propagation on logger "child"
    And I log "before toggle" at level "INFO" from logger "child"
    And I flush all loggers
    Then the root handler file does not contain "before toggle"
    When I enable propagation on logger "child"
    And I log "after toggle" at level "INFO" from logger "child"
    And I flush all loggers
    Then the root handler file contains "after toggle"

  Scenario: multi-level hierarchy propagation
    Given a file handler "root_handler" writing to a temporary file
    And a ConfigBuilder with root logger using handler "root_handler" at level "INFO"
    And a logger "parent" at level "INFO" without handlers
    And a logger "parent.child" at level "INFO" without handlers
    When I build and initialise the configuration
    And I log "deep message" at level "INFO" from logger "parent.child"
    And I flush all loggers
    Then the root handler file contains "deep message"

  Scenario: handler on both child and root receives record once each
    Given a file handler "root_handler" writing to a temporary file
    And a file handler "child_handler" writing to a temporary file
    And a ConfigBuilder with root logger using handler "root_handler" at level "INFO"
    And a child logger "child" at level "INFO" using handler "child_handler"
    When I build and initialise the configuration
    And I log "dual handler message" at level "INFO" from logger "child"
    And I flush all loggers
    Then the root handler file contains "dual handler message"
    And the child handler file contains "dual handler message"

  Scenario: configuration matches snapshot with propagate enabled
    Given a ConfigBuilder for snapshot test
    And a stream handler "console" targeting "stderr"
    And a logger "worker" at level "DEBUG" with propagate true
    And a root logger at level "INFO"
    Then the configuration matches propagate enabled snapshot

  Scenario: configuration matches snapshot with propagate disabled
    Given a ConfigBuilder for snapshot test
    And a stream handler "console" targeting "stderr"
    And a logger "worker" at level "DEBUG" with propagate false
    And a root logger at level "INFO"
    Then the configuration matches propagate disabled snapshot
