Feature: fileConfig

  Scenario: configure root logger via fileConfig
    Given the logging system is reset
    When I configure fileConfig from "tests/data/basic_file_config.ini"
    Then logging "file-config hello" at "INFO" from root matches snapshot

  Scenario: handler without class is rejected
    Given the logging system is reset
    When I attempt to configure fileConfig from "tests/data/fileconfig_invalid_handler.ini"
    Then fileConfig raises ValueError
