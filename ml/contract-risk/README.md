# xDEVS Contract Risk

A classifier that scores EVM contract bytecode for patterns seen in malicious contracts (wallet drainers, exploit and
phishing contracts), built for the xDEVS Chain safety layer: the node and wallet can warn before you interact with a
contract that looks like known malicious code. It needs only `eth_getCode`, so it also works on brand-new contracts.

- Model, metrics and model card: https://huggingface.co/JacellJamble/xdevs-contract-risk
- Demo: https://huggingface.co/spaces/JacellJamble/xdevs-contract-risk-demo

Held-out test set (16,087 contracts never seen in training, 132 malicious): ROC-AUC 0.964, average precision 0.632,
precision 94% at a 0.025% false-positive rate, 65% recall at a 1% false-positive rate.

| File | What it does |
|---|---|
| `xdevs_contract_risk.py` | Disassembler, stateless featurizer and `ContractRiskModel` for inference |
| `train.py` | Data loading, de-duplication, clone grouping, grouped CV, model selection, test evaluation |
| `publish.py` | Uploads the model (with the generated model card) and the static demo to Hugging Face |
| `MODEL_CARD.md` | Model card template, filled with the run's metrics by `publish.py` |
| `static/` | Browser demo (static Space): JavaScript port of the featurizer and model, export and parity check |
| `data/` | Vendored labels from forta-network/labelled-datasets (MIT) |

Training runs on GitHub Actions: **Actions → ml-contract-risk → Run workflow** (`mode=train`, `publish=true` to upload;
needs the `HF_TOKEN` secret). `mode=space` re-publishes only the demo.
