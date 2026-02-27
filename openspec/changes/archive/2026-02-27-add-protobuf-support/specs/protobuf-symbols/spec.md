# Protobuf Symbols

## ADDED Requirements

### Requirement: Protobuf message extraction

#### Scenario: Top-level message

- **WHEN** a `.proto` file contains `message PaymentRequest { ... }`
- **THEN** a symbol SHALL be extracted with:
  - name: `PaymentRequest`
  - kind: `struct`
  - visibility: `public`
  - file_path: relative path to the `.proto` file
  - line: line number of the message declaration

#### Scenario: Nested message

- **WHEN** a `.proto` file contains a message defined inside another message
- **THEN** the nested message SHALL be extracted with `parent_symbol` set to the enclosing message name

### Requirement: Protobuf service extraction

#### Scenario: Service definition

- **WHEN** a `.proto` file contains `service PaymentAPI { ... }`
- **THEN** a symbol SHALL be extracted with:
  - name: `PaymentAPI`
  - kind: `interface`
  - visibility: `public`

### Requirement: Protobuf RPC extraction

#### Scenario: RPC method

- **WHEN** a `.proto` file contains `rpc ProcessPayment(PaymentRequest) returns (PaymentResponse) {}`
- **THEN** a symbol SHALL be extracted with:
  - name: `ProcessPayment`
  - kind: `method`
  - parent_symbol: the enclosing service name
  - parameters: `[{"name": "request", "type": "PaymentRequest"}]`
  - return_type: `PaymentResponse`

#### Scenario: Streaming RPC

- **WHEN** a `.proto` file contains `rpc StreamUpdates(stream UpdateRequest) returns (stream UpdateResponse) {}`
- **THEN** a symbol SHALL be extracted with:
  - name: `StreamUpdates`
  - kind: `method`
  - parent_symbol: the enclosing service name
  - parameters: `[{"name": "request", "type": "stream UpdateRequest"}]`
  - return_type: `stream UpdateResponse`

### Requirement: Protobuf enum extraction

#### Scenario: Top-level enum

- **WHEN** a `.proto` file contains `enum PaymentStatus { ... }`
- **THEN** a symbol SHALL be extracted with:
  - name: `PaymentStatus`
  - kind: `enum`
  - visibility: `public`

#### Scenario: Nested enum

- **WHEN** an enum is defined inside a message
- **THEN** the enum SHALL be extracted with `parent_symbol` set to the enclosing message name

### Requirement: Protobuf oneof extraction

#### Scenario: Oneof field group

- **WHEN** a `.proto` file contains `oneof payment_method { ... }` inside a message
- **THEN** a symbol SHALL be extracted with:
  - name: `payment_method`
  - kind: `type`
  - parent_symbol: the enclosing message name
