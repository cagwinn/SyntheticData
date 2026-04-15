# Python SDK

The Python SDK (`datasynth-py`) provides a high-level interface to DataSynth for use in data science workflows.

## Installation

```bash
cd python && pip install -e ".[all]"
```

## Basic Usage

```python
from datasynth_py import DataSynth, Config, GlobalSettings, CompanyConfig, ChartOfAccountsSettings

config = Config(
    global_settings=GlobalSettings(
        industry="retail",
        start_date="2024-01-01",
        period_months=12,
    ),
    companies=[
        CompanyConfig(code="C001", name="Retail Corp", currency="USD", country="US"),
    ],
    chart_of_accounts=ChartOfAccountsSettings(complexity="small"),
)

result = DataSynth().generate(
    config=config,
    output={"format": "csv", "sink": "temp_dir"},
)
```

## Blueprints

Pre-built configurations for common use cases:

```python
from datasynth_py import blueprints

config = blueprints.retail_small()
config = blueprints.manufacturing_large()
config = blueprints.banking_medium()
config = blueprints.ml_training()
config = blueprints.statistical_validation()
config = blueprints.with_distributions()
```

Each blueprint returns a `Config` object that can be modified before generation.

## Spark Integration

Load generated data directly into Spark DataFrames:

```python
from pyspark.sql import SparkSession

spark = SparkSession.builder.getOrCreate()

# Generate to a temp directory, then load
result = DataSynth().generate(config=config, output={"format": "csv", "sink": "temp_dir"})
journal_entries = spark.read.csv(f"{result.output_dir}/journal_entries.csv", header=True)
```

## dbt Integration

Generate seed data for dbt projects:

```python
result = DataSynth().generate(
    config=config,
    output={"format": "csv", "sink": "./dbt_project/seeds/"},
)
# CSV files land directly in the dbt seeds directory
```

## Airflow Integration

Use DataSynth in an Airflow DAG:

```python
from airflow.decorators import task

@task
def generate_synthetic_data():
    from datasynth_py import DataSynth, blueprints
    config = blueprints.manufacturing_large()
    result = DataSynth().generate(
        config=config,
        output={"format": "csv", "sink": "/data/synthetic/"},
    )
    return result.output_dir
```

## MLflow Integration

Log generated datasets as MLflow artifacts:

```python
import mlflow
from datasynth_py import DataSynth, blueprints

with mlflow.start_run():
    config = blueprints.ml_training()
    result = DataSynth().generate(
        config=config,
        output={"format": "csv", "sink": "temp_dir"},
    )
    mlflow.log_artifacts(result.output_dir, artifact_path="synthetic_data")
    mlflow.log_param("seed", config.global_settings.seed)
    mlflow.log_param("period_months", config.global_settings.period_months)
```
