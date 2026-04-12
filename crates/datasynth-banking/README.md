# datasynth-banking

KYC/AML banking transaction generator for compliance testing and fraud detection ML.

## Overview

`datasynth-banking` provides realistic banking data generation for:

- **KYC/AML Testing**: Generate transaction data for compliance system validation
- **Fraud Detection ML**: Labeled data for supervised learning models
- **Stress Testing**: High-volume transaction generation for system testing
- **Typology Simulation**: Realistic AML typologies (structuring, layering, mule networks)

## Key Components

### Models (`models/`)

| Model | Description |
|-------|-------------|
| `BankingCustomer` | Retail, Business, Trust customer personas |
| `BankAccount` | Account types with feature sets |
| `BankTransaction` | Transaction records with direction/channel |
| `KycProfile` | Expected activity envelope (turnover, frequency, sources) |
| `CounterpartyPool` | Transaction counterparty management |
| `CaseNarrative` | Investigation and compliance narratives |

### Generators (`generators/`)

| Generator | Description |
|-----------|-------------|
| `customer_generator` | Customer with KYC profile generation |
| `account_generator` | Account creation with proper features |
| `transaction_generator` | Persona-based transaction generation |
| `counterparty_generator` | Counterparty pool management |

### AML Typologies (`typologies/`) — 14 implemented

| Typology | Description |
|----------|-------------|
| `structuring` | Structuring below reporting thresholds |
| `funnel` | Funnel account patterns for layering |
| `layering` | Complex transaction layering schemes |
| `mule` | Money mule network patterns |
| `round_tripping` | Round-tripping via foreign accounts |
| `fraud` | ATO, BEC, fake vendor, APP, duplicate payment |
| `synthetic_identity` | Fabricated identity → credit seasoning → bust-out |
| `trade_based_ml` | Over/under-invoicing, phantom shipments (SWIFT) |
| `crypto_integration` | Fiat→exchange→off-chain gap→fiat peel chain |
| `sanctions_evasion` | Name variations, transshipment routing |
| `pouch_activity` | Multi-branch cash pouch deposits |
| `romance_scam` | Escalating outbound to foreign persona |
| `casino_integration` | Chip purchase → minimal play → winnings check |
| `real_estate_integration` | Earnest + closing via title companies |
| `spoofing` | Adversarial transaction camouflage |

### Multi-party networks (`typologies/network_*`)

| Component | Description |
|-----------|-------------|
| `network_generator` | Structuring rings, mule chains, shell pyramids |
| `network_topology` | Barabási-Albert preferential attachment (power-law) |

### Temporal realism (`generators/`)

| Component | Description |
|-----------|-------------|
| `lifecycle_engine` | Account phase assignment (New→RampUp→Steady→Decline→Dormant) |
| `lifecycle_stochastic` | Event-driven phase transitions (6 life events) |
| `velocity_computer` | Pre-computed rolling-window features per transaction |
| `device_realism` | Per-customer power-law device pool + trust evolution |
| `sanctions_variance` | Context-aware screening (risk × country × PEP × industry) |
| `payment_bridge` | Cross-layer bridge from document-flow Payments to BankTransactions |

### Quality injection

| Component | Description |
|-----------|-------------|
| `false_positive` | Tags legitimate transactions that look suspicious |
| `sophistication_sampler` | Context-correlated sophistication sampler |

### Customer Personas (`personas/`)

| Persona | Description |
|---------|-------------|
| `retail` | Individual customer behavioral patterns |
| `business` | Business account patterns |
| `trust` | Trust/corporate patterns |

### Labels (`labels/`)

| Label Type | Description |
|------------|-------------|
| `entity_labels` | Entity-level ML labels |
| `relationship_labels` | Relationship risk labels |
| `transaction_labels` | Transaction classification labels |
| `narrative_generator` | Investigation narrative generation |

## Usage

```rust
use datasynth_banking::{BankingOrchestrator, BankingConfig};

let config = BankingConfig::default();
let mut orchestrator = BankingOrchestrator::new(config, seed);

// Generate banking data
let result = orchestrator.generate()?;

// Access generated data
println!("Customers: {}", result.customers.len());
println!("Transactions: {}", result.transactions.len());
println!("Suspicious labels: {}", result.labels.suspicious_count());
```

## Output Files

| File | Description |
|------|-------------|
| `banking_customers.csv` | Customer profiles with KYC data |
| `bank_accounts.csv` | Account records with features |
| `bank_transactions.csv` | Transaction records |
| `kyc_profiles.csv` | Expected activity envelopes |
| `counterparties.csv` | Counterparty pool |
| `aml_typology_labels.csv` | AML typology labels |
| `entity_risk_labels.csv` | Entity-level risk classifications |
| `transaction_risk_labels.csv` | Transaction-level classifications |

## License

Apache-2.0 - See [LICENSE](../../LICENSE) for details.
