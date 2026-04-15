# Banking & AML

The `datasynth-banking` crate generates KYC/AML banking data with realistic money laundering typologies, criminal network structures, and velocity features.

## Enabling Banking

```bash
datasynth-data generate --config config.yaml --banking --output ./output
```

Or in config:

```yaml
banking:
  enabled: true
  customer_count: 1000
  account_count: 2000
  transaction_count: 50000
  typologies:
    suspicious_rate: 0.05
    structuring_rate: 0.01
    funnel_rate: 0.01
    layering_rate: 0.01
    mule_rate: 0.005
    fraud_rate: 0.005
    false_positive_rate: 0.02
    network_typology_rate: 0.05
```

## AML Typologies (20 Types)

### Structuring
- **Structuring** -- Deposits just below reporting threshold
- **Smurfing** -- Multiple people making small deposits
- **Cuckoo Smurfing** -- Using legitimate account holders

### Funnel Patterns
- **Funnel Account** -- Many inflows, few outflows
- **Concentration Account** -- Abuse of pooling accounts
- **Pouch Activity** -- Bulk cash collection and deposit

### Layering
- **Layering** -- Multiple rapid transfers to obscure origin
- **Rapid Movement** -- Funds moved through accounts within hours
- **Shell Company** -- Transactions through shell entities

### Round-Tripping
- **Round Tripping** -- Funds cycled through foreign accounts
- **Trade-Based ML** -- Over/under-invoicing trade transactions
- **Invoice Manipulation** -- Fictitious invoices for fund movement

### Mule Networks
- **Money Mule** -- Recruited individuals moving funds
- **Romance Scam** -- Social engineering for fund transfers
- **Advance Fee Fraud** -- Upfront payments for fictitious services

### Spoofing & Fraud
- Additional typologies for identity spoofing, account takeover, and transaction fraud

## Output Files

Generated in the `banking/` directory:

| File | Description |
|------|-------------|
| `banking_customers.json` | KYC profiles with risk scores |
| `banking_accounts.json` | Account metadata and types |
| `banking_transactions.json` | Full transaction history with velocity features |
| `aml_transaction_labels.json` | Per-transaction AML labels |
| `aml_customer_labels.json` | Per-customer risk labels |
| `aml_account_labels.json` | Per-account labels |
| `aml_relationship_labels.json` | Network relationship labels |
| `aml_narratives.json` | Human-readable scenario narratives |

## Criminal Network Generation

The banking orchestrator builds multi-layer criminal networks:
- Controller nodes directing mule chains
- Layering intermediaries with shell company fronts
- Cross-typology co-occurrence (configurable via `network_typology_rate`)
- False positive injection for realistic model evaluation

## Velocity Features

Transaction records include pre-computed velocity features for ML:
- Rolling transaction counts and amounts (1h, 24h, 7d, 30d windows)
- Cross-border transaction ratios
- Counterparty concentration scores
- Time-of-day and day-of-week patterns
