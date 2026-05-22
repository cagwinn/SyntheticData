#!/usr/bin/env python3
"""In-distribution light-B: replicate the campaign's run_one config build exactly
(deepcopy inverse_base.yaml, apply the 3 PARAMS, generate, apply posterior), fixing
amount_mu/sigma at prior centres and sweeping ONLY fraud.fraud_rate. This puts the
probe x on the training manifold, so it is a FAIR test of fraud_rate recovery
(vs my first probe which used the inverse_audit healthcare base → OOD collapse)."""
import sys, copy, json, subprocess
from pathlib import Path
import numpy as np, yaml
sys.path.insert(0, ".")
from inverse import params as P
from inverse.simulate import _set_dotted

BASE = yaml.safe_load(Path("inverse/inverse_base.yaml").read_text())
MU_C, SIG_C = 6.5, 1.55          # prior centres: mu∈[3,10], sigma∈[0.5,2.6]
ROOT = Path("/tmp/iaf3"); ROOT.mkdir(parents=True, exist_ok=True)
rows = []
for rate in [0.0, 0.02, 0.05, 0.10]:
    theta = np.array([rate, MU_C, SIG_C], dtype=float)
    cfg = copy.deepcopy(BASE)
    for k, v in P.to_config_overrides(theta).items():
        _set_dotted(cfg, k, v)
    _set_dotted(cfg, "global.seed", 12345)
    d = ROOT / f"sweep_{int(round(rate*100)):02d}"; d.mkdir(parents=True, exist_ok=True)
    cfgp = d / "cfg.yaml"; cfgp.write_text(yaml.safe_dump(cfg))
    subprocess.run(["datasynth-data", "generate", "--config", str(cfgp),
                    "--output", str(d / "gl")], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run([sys.executable, "-m", "inverse.apply", "--weights", "inverse/weights",
                    "--gl-canonical", str(d / "gl" / "journal_entries.csv"),
                    "--out", str(d / "posterior.json")], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    post = json.loads((d / "posterior.json").read_text())
    fr = post.get("fraud.fraud_rate", {})
    rows.append({"injected": rate, "recovered_median": fr.get("median"), "ci90": fr.get("ci90")})
print("INDIST_LIGHTB_RESULT")
print(json.dumps(rows, indent=2))
# correlation of recovered vs injected
inj = np.array([r["injected"] for r in rows]); rec = np.array([r["recovered_median"] or 0.0 for r in rows])
if rec.std() > 1e-9:
    print(f"monotone? recovered={list(np.round(rec,4))}  pearson_r={np.corrcoef(inj,rec)[0,1]:.3f}")
else:
    print(f"recovered is flat ({list(np.round(rec,4))}) → still degenerate")
