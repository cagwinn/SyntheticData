# DataSynth v3.0.0

Synthetic enterprise data generation for ML training, audit analytics, and system testing.

DataSynth generates statistically realistic, fully interconnected enterprise financial data across 20+ process families. Generated data respects accounting identities, follows empirical distributions, and maintains referential integrity across 100+ output tables.

## Key Features

- **306 domain models** covering GL, AR/AP, banking, manufacturing, HR, audit, compliance
- **50+ generators** with 20 AML typologies, multi-stage fraud schemes, process simulation
- **AI capabilities** — neural diffusion, LLM config generation, adversarial testing, auto-tuning
- **Scenario engine** — counterfactual paired generation with causal DAG propagation
- **Generation-time assertions** — every non-anomaly JE balances, IC eliminations net to zero
- **XXL performance** — 200K+ JEs in 20.6s (CSV-only), 4x speedup with format-aware output
- **10 audit methodology blueprints** — ISA, PCAOB, Big 4 approaches, SOC 2
- **Python SDK** with Spark, dbt, Airflow, MLflow integrations

## Example Datasets

Pre-generated datasets at [huggingface.co/VynFi](https://huggingface.co/VynFi):

| Dataset | Records | Domain |
|---------|---------|--------|
| [vynfi-aml-100k](https://huggingface.co/datasets/VynFi/vynfi-aml-100k) | 749K | Banking/AML with velocity features |
| [vynfi-audit-p2p](https://huggingface.co/datasets/VynFi/vynfi-audit-p2p) | 234 | P2P document chain with fraud labels |
| [vynfi-ocel-manufacturing](https://huggingface.co/datasets/VynFi/vynfi-ocel-manufacturing) | 344 | OCEL event log for process mining |

## Commercial Offering

SDKs, hosted generation, and enterprise support: [vynfi.com](https://vynfi.com)
