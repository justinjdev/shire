# Java/Kotlin Symbols

## ADDED Requirements

### Requirement: Java class extraction

#### Scenario: Public class

- **WHEN** a `.java` file contains `public class PaymentService { ... }`
- **THEN** a symbol SHALL be extracted with:
  - name: `PaymentService`
  - kind: `class`
  - visibility: `public`

#### Scenario: Public interface

- **WHEN** a `.java` file contains `public interface PaymentHandler { ... }`
- **THEN** a symbol SHALL be extracted with kind `interface`

#### Scenario: Public enum

- **WHEN** a `.java` file contains `public enum PaymentStatus { ... }`
- **THEN** a symbol SHALL be extracted with kind `enum`

#### Scenario: Package-private or private class

- **WHEN** a `.java` class has no access modifier or is `private`
- **THEN** it SHALL NOT be extracted

### Requirement: Java method extraction

#### Scenario: Public method

- **WHEN** a `.java` file contains `public Receipt processPayment(double amount, String currency)` inside class `PaymentService`
- **THEN** a symbol SHALL be extracted with:
  - name: `processPayment`
  - kind: `method`
  - parent_symbol: `PaymentService`
  - parameters: `[{"name": "amount", "type": "double"}, {"name": "currency", "type": "String"}]`
  - return_type: `Receipt`

#### Scenario: Public static method

- **WHEN** a `.java` file contains `public static PaymentService create()`
- **THEN** a symbol SHALL be extracted with kind `function`

#### Scenario: Private or package-private method

- **WHEN** a method has no access modifier or is `private`
- **THEN** it SHALL NOT be extracted

### Requirement: Java constant extraction

#### Scenario: Public static final field

- **WHEN** a `.java` file contains `public static final int MAX_RETRIES = 3`
- **THEN** a symbol SHALL be extracted with kind `constant`

### Requirement: Kotlin class extraction

#### Scenario: Class

- **WHEN** a `.kt` file contains `class PaymentService { ... }`
- **THEN** a symbol SHALL be extracted with:
  - name: `PaymentService`
  - kind: `class`
  - visibility: `public`

#### Scenario: Object declaration

- **WHEN** a `.kt` file contains `object PaymentFactory { ... }`
- **THEN** a symbol SHALL be extracted with kind `class`

#### Scenario: Interface

- **WHEN** a `.kt` file contains `interface PaymentHandler { ... }`
- **THEN** a symbol SHALL be extracted with kind `interface`

#### Scenario: Enum class

- **WHEN** a `.kt` file contains `enum class PaymentStatus { ... }`
- **THEN** a symbol SHALL be extracted with kind `enum`

#### Scenario: Private or internal class

- **WHEN** a `.kt` class is declared with `private` or `internal` modifier
- **THEN** it SHALL NOT be extracted

### Requirement: Kotlin function extraction

#### Scenario: Top-level function

- **WHEN** a `.kt` file contains `fun processPayment(amount: Double, currency: String): Receipt`
- **THEN** a symbol SHALL be extracted with:
  - name: `processPayment`
  - kind: `function`
  - parameters: `[{"name": "amount", "type": "Double"}, {"name": "currency", "type": "String"}]`
  - return_type: `Receipt`

#### Scenario: Class method

- **WHEN** a `.kt` file contains `fun validate(token: String): Boolean` inside class `AuthService`
- **THEN** a symbol SHALL be extracted with kind `method` and parent_symbol `AuthService`

#### Scenario: Private or internal function

- **WHEN** a function is declared with `private` or `internal` modifier
- **THEN** it SHALL NOT be extracted

### Requirement: Skip Gradle Kotlin DSL files

#### Scenario: build.gradle.kts

- **WHEN** a file is named `build.gradle.kts` or ends with `.gradle.kts`
- **THEN** it SHALL be skipped during Kotlin symbol extraction
