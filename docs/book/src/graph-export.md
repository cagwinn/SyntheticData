# Graph Export

DataSynth exports accounting network graphs in multiple formats for graph ML, knowledge graph, and visualization use cases.

## Enabling Graph Export

```bash
datasynth-data generate --config config.yaml --graph-export --output ./output
```

Or in config:

```yaml
graph_export:
  enabled: true
  formats: [pytorch_geometric, neo4j, dgl, hypergraph]
```

## Supported Formats

### PyTorch Geometric (.pt)

Binary `.pt` files loadable with `torch.load()`:

```python
import torch
data = torch.load("output/graphs/transaction_graph.pt")
# data.x         → Node features
# data.edge_index → Edge COO format
# data.edge_attr  → Edge features
# data.y         → Node labels
```

Graph types: `transaction_graph`, `approval_graph`, `entity_graph`.

### Neo4j (CSV + Cypher)

CSV files for nodes and edges, plus Cypher `CREATE` statements:

```
output/graphs/neo4j/
  nodes_*.csv
  edges_*.csv
  import.cypher
```

Load with:

```bash
cypher-shell -f output/graphs/neo4j/import.cypher
```

### DGL

DGL-compatible graph format for use with the Deep Graph Library.

### RustGraph JSON

JSONL format for streaming to RustGraph ingest endpoints:

```bash
datasynth-data generate --config config.yaml \
  --stream-target http://localhost:8080/ingest \
  --stream-api-key your-key \
  --stream-batch-size 1000
```

Streams hypergraph JSONL in batches during generation.

### Hypergraph

Unified hypergraph representation that captures n-ary relationships (e.g., a journal entry connecting multiple GL accounts, entities, and documents simultaneously).

## Graph Contents

The exported graphs include:

| Node Type | Source |
|-----------|--------|
| JournalEntry | Generated JEs |
| GLAccount | Chart of accounts |
| Vendor, Customer | Master data |
| Entity (Company) | Company config |
| Document | POs, invoices, payments |
| Employee | Master data |
| AuditEngagement | Audit FSM (if enabled) |

| Edge Type | Relationship |
|-----------|-------------|
| POSTED_TO | JE -> GL Account |
| APPROVED_BY | Document -> Employee |
| ISSUED_BY | Invoice -> Vendor/Customer |
| REFERENCES | Payment -> Invoice |
| BELONGS_TO | Account -> Entity |
| COMPONENT_OF | Audit component -> Group audit |

## Node Properties

All domain models implement the `ToNodeProperties` trait, which converts typed fields into `GraphPropertyValue` enums (String, Integer, Float, Boolean, DateTime) for format-agnostic export.

## Edge Constraints

Edge definitions include cardinality constraints (`OneToOne`, `OneToMany`, `ManyToMany`) enforced during graph construction.
