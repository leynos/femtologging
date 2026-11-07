Feature: Published first-run examples
  A newcomer copies the first example from the README or the users' guide
  and expects it to run and print what the document shows.

  Scenario Outline: A first-run example prints its documented output
    Given the published example "<example>"
    When I run it in a fresh interpreter
    Then it exits successfully
    And it prints the output published as "<example>-output"

    Examples:
      | example             |
      | readme-quick-start  |
      | guide-first-message |
