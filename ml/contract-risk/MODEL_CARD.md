---
license: apache-2.0
library_name: lightgbm
tags:
- tabular-classification
- smart-contracts
- ethereum
- evm
- security
- blockchain
datasets:
- forta/malicious-smart-contract-dataset
metrics:
- roc_auc
- average_precision
- recall
- precision
pipeline_tag: tabular-classification
---

# xDEVS Contract Risk

A classifier that scores EVM contract **bytecode** for patterns seen in malicious contracts: wallet drainers, phishing
contracts and exploit contracts. It is built for the safety layer of [xDEVS Chain](https://xdevschain.tech), an
EVM-equivalent Layer 1 that warns people before they sign: the node already simulates transactions
(`xdevs_simulateTransaction`) and checks recipients (`xdevs_checkRecipient`); this model adds "the contract you are
about to call looks like known malicious code".

It needs only the contract's code (`eth_getCode`), so it works on any EVM chain and on brand-new contracts with no
history. Try it: [demo Space](https://huggingface.co/spaces/{space}).

## Results (held-out test set)

The test set is 20% of contract *groups* never seen in training (see "Evaluation" for why groups matter).

| Metric | Released model | LightGBM alone | Logistic regression alone |
|---|---|---|---|
| ROC-AUC | **{roc}** | {g_roc} | {b_roc} |
| Average precision (PR-AUC) | **{ap}** | {g_ap} | {b_ap} |
| Recall at 1% false-positive rate | **{r1}** | {g_r1} | {b_r1} |
| Recall at 0.1% false-positive rate | {r01} | | |

At the released threshold ({thr}): precision **{prec}**, recall **{rec}**, F1 {f1}, false-positive rate {fpr}.
Confusion matrix: {tp} malicious caught, {fn} missed, {fp} benign contracts flagged, {tn} benign passed.

5-fold grouped cross-validation on the training part (released model): ROC-AUC {cv_roc}, average precision {cv_ap}, recall at 1% FPR {cv_r1}.

![Evaluation](evaluation.png)

## Data

- [forta/malicious-smart-contract-dataset](https://huggingface.co/datasets/forta/malicious-smart-contract-dataset) (MIT):
  Ethereum mainnet contracts with creation bytecode and a malicious/benign label.
- [forta-network/labelled-datasets](https://github.com/forta-network/labelled-datasets) (MIT): malicious smart contracts
  (exploit and drainer contracts, with their creation transactions) and phishing/scam contracts on Ethereum. Their bytecode
  is fetched from mainnet: the creation input when available (so self-destructed contracts are included), otherwise the
  current runtime code.
- Rows read: {sources}.
- {rows} rows, {dups} exact duplicates removed, **{unique} unique contracts ({malicious} malicious)** in {groups} groups.
- Train: {train} contracts ({train_mal} malicious). Test: {test} contracts ({test_mal} malicious).

Malicious contracts are rare, as they are on a real chain, so the metrics that matter are precision/recall and recall at a
low false-positive rate, not accuracy (a model that flags nothing would be >99% "accurate").

## How it works

1. **Runtime code only.** The constructor and the trailing CBOR metadata are stripped, so training (creation code) matches
   what `eth_getCode` returns at inference time, and compiler hashes are not features.
2. **Features** (stateless, no fitted vocabulary): opcode unigram and bigram frequencies (hashed to 2^18), the set of 4-byte
   function selectors pushed by the code (hashed to 2^16), and ~120 scalar features: code size, counts and shares of
   fund-moving and control opcodes (`CALL`, `DELEGATECALL`, `SELFDESTRUCT`, `CREATE2`, `ORIGIN`, hard-coded addresses via
   `PUSH20`, ...) and well-known selectors (ERC-20 approve/transferFrom, flash loans, swaps, multicall, ownership).
3. **Model**: LightGBM gradient-boosted trees (class-weighted for the imbalance, rounds chosen by early stopping in CV)
   blended in logit space with an L2 logistic regression on log-compressed features. The blend weight ({w} on LightGBM)
   was chosen by out-of-fold average precision among 1.0 / 0.75 / 0.5 / 0.25 / 0.0.

## Evaluation

- **Grouped splits.** Scam contracts are often redeployed as near-identical copies. Contracts are grouped by their opcode
  skeleton (PUSH arguments removed) and every split keeps whole groups together, so the test set measures how well the
  model recognises *new* code, not copies it has memorised.
- **Threshold** chosen on out-of-fold predictions (best F1), then applied unchanged to the test set.
- **Baseline**: L2-regularised logistic regression on the same features.
- **Shortcut check.** Most malicious examples come from on-chain runtime code, most benign ones from creation code. To stop
  the model from learning *where the code came from*, constructor and metadata are stripped from both, and a random
  sample of benign contracts is scored from their on-chain runtime code. Test results per source:

{bysource}

## Use

```bash
pip install huggingface_hub lightgbm scikit-learn scipy numpy
python example.py 0xContractAddress https://rpc.xdevschain.tech
```

```python
from huggingface_hub import snapshot_download
import sys
d = snapshot_download("{repo}"); sys.path.insert(0, d)
from xdevs_contract_risk import ContractRiskModel
model = ContractRiskModel(d)
model.predict("0x6080...")   # {{'risk_score': ..., 'risk_level': 'low'|'medium'|'high', 'signals': {{...}}}}
```

## Limitations

- **Advice, not proof.** A high score means the code resembles malicious code in the training data. Proxies, obfuscated or
  brand-new attack patterns can be missed, and unusual but honest code can be flagged. Show it as a warning next to
  simulation results, never as a block on its own.
- Labels come from Ethereum mainnet (2022-2023 era) public reports; the mix of attacks changes over time. Retrain periodically.
- A proxy contract's risk lives in its implementation: score the implementation address too.
- Not trained on xDEVS Chain data (the Enclave testnet is new); xDEVS Chain is EVM-equivalent, so bytecode is the same.

## Licence and credits

Code and model: Apache-2.0, by THE xDEVS ([source](https://github.com/JACELL100/xdevs-chain/tree/main/ml/contract-risk)).
Training data by Forta Network (MIT).
