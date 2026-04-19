# DataSynth Starter Template Pack (v3.2.0+)

This directory contains a **shape-only** starter template pack. Every
category appears as an empty array/map so you can see the schema and
drop in your own entries.

## How it works

1. Copy this directory to your own location:
   ```bash
   cp -r templates/packs/defaults ./my_templates
   ```

2. Edit the files you want to customise. For example,
   `bank_names.yaml`:
   ```yaml
   names:
     - "Deutsche Bank"
     - "Commerzbank"
     - "DZ Bank"
   ```

3. Point your config at the directory:
   ```yaml
   templates:
     path: ./my_templates
     merge_strategy: extend   # extend | replace | merge_prefer_file
   ```

4. Generate:
   ```bash
   datasynth-data generate --config my_config.yaml
   ```

## Merge strategies

- **`extend`** (default, safest) — append your entries to the embedded
  defaults. Empty categories in your file keep the embedded pool intact.
  Best for adding a few regional overrides without losing the baseline
  diversity.
- **`replace`** — discard embedded defaults entirely. Requires every
  category you care about to be fully populated in your file.
- **`merge_prefer_file`** — per-category replacement: if a category
  in your file is non-empty, it replaces the embedded pool; otherwise
  embedded stays. Good for "I have a complete vendor list but want
  the embedded material descriptions".

## Category reference

| File | Populates | Used by |
|---|---|---|
| `metadata.yaml` | pack name/version/region/sector | informational |
| `person_names.yaml` | first/last names per culture | all master-data name generators |
| `vendor_names.yaml` | vendor names per category | `VendorGenerator` |
| `customer_names.yaml` | customer names per industry | `CustomerGenerator` |
| `material_descriptions.yaml` | material descs per type | `MaterialGenerator` |
| `asset_descriptions.yaml` | asset descs per category | `AssetGenerator` |
| `line_item_descriptions.yaml` | JE line text per process+account | JE generator |
| `header_text_templates.yaml` | JE header text per process | JE generator |
| `bank_names.yaml` | bank name pool (flat) | `VendorGenerator::generate_bank_account` (**v3.2.0**) |
| `finding_titles.yaml` | audit finding titles per type | `FindingGenerator` (v3.2.1) |
| `finding_narratives.yaml` | audit narrative sections per type | `FindingGenerator` (v3.2.1) |
| `department_names.yaml` | display name per department code | `EmployeeGenerator` (v3.2.1) |

## Validation

Before using a pack, validate it:

```bash
datasynth-data templates validate --path ./my_templates
```

This catches YAML parse errors and structural issues (missing cultures,
empty required sections).

## v3.2.0 scope

v3.2.0 ships the full infrastructure (config field, trait, provider,
YAML schema, CLI export/validate) plus two rewired sites:

- **Vendor bank names** — `VendorGenerator` now routes through
  `TemplateProvider::get_bank_name`. Populate `bank_names.yaml` and
  every vendor's bank account gets a name from your pool.
- **Customer names per industry** — `CustomerGenerator` now routes
  through `TemplateProvider::get_customer_name`. Populate
  `customer_names.yaml` under the right industry key
  (e.g. `industries.retail`, `industries.automotive`).

The remaining sites (`material_descriptions`, `asset_descriptions`,
`finding_titles`, `finding_narratives`, `department_names`) are wired
through the trait but still read the embedded pools — v3.2.1 ships
the rest of the rewires.
