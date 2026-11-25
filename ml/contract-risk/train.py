"""Train and evaluate xDEVS Contract Risk (malicious-contract classifier on EVM bytecode).

Data (all MIT, Ethereum mainnet): the Forta "Malicious Smart Contract Classification Dataset" on Hugging Face
(83k unique contracts, 127 malicious) plus the malicious contracts listed in
forta-network/labelled-datasets (its phishing contracts are a subset of them), whose bytecode is fetched from an
Ethereum RPC. (flipguard/malicious-contracts
was checked and is an exact copy of the Forta parquet, so it adds nothing.)

Method
  1. Featurize every contract from its runtime code (constructor and CBOR metadata stripped):
     opcode 1-2-gram frequencies (hashed), 4-byte selectors present (hashed), and ~120 scalar features.
  2. De-duplicate exact copies and group near-copies by their opcode skeleton (PUSH arguments removed),
     so a clone of a test contract can never sit in training data.
  3. Hold out 20% of groups (stratified) as the test set. On the rest, 5-fold stratified group CV compares a
     logistic-regression baseline with LightGBM; the decision threshold is chosen on out-of-fold predictions.
  4. The released model is LightGBM trained on the training groups (early-stopping rounds from CV) and
     scored once on the untouched test set.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import os
import io
import json
import sys
import time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import lightgbm as lgb
import numpy as np
import pyarrow.parquet as pq
import scipy.sparse as sp
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import (average_precision_score, confusion_matrix, f1_score, precision_recall_curve,
                             precision_score, recall_score, roc_auc_score, roc_curve)
from sklearn.model_selection import StratifiedGroupKFold

import xdevs_contract_risk as xr

SEED = 17
FORTA = ("forta/malicious-smart-contract-dataset", "malicious_contract_training_dataset_final.parquet")
FLIPGUARD = "https://huggingface.co/datasets/flipguard/malicious-contracts/resolve/main/malicious-contracts.csv"


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


# ---------------- data ----------------
LABELS = "https://raw.githubusercontent.com/forta-network/labelled-datasets/main/labels/1"
ETH_RPC = os.environ.get("ETH_RPC", "https://ethereum-rpc.publicnode.com")


def _rpc_batch(calls, tries=4):
    import requests

    for k in range(tries):
        try:
            r = requests.post(ETH_RPC, json=[{"jsonrpc": "2.0", "id": i, "method": m, "params": p} for i, (m, p) in enumerate(calls)],
                              headers={"user-agent": "xdevs-contract-risk-training"}, timeout=60)
            r.raise_for_status()
            res = {x["id"]: x.get("result") for x in r.json()}
            return [res.get(i) for i in range(len(calls))]
        except Exception as e:  # rate limits, timeouts
            log("rpc retry", k, repr(e)[:120]); time.sleep(3 * (k + 1))
    return [None] * len(calls)


def iter_forta_labels():
    """Malicious contracts from forta-network/labelled-datasets (MIT): 'malicious_smart_contracts' (with creation tx)
    and the contracts in 'phishing_scams'. Code comes from Ethereum mainnet: the creation input when the creation tx
    deployed it directly (works even after SELFDESTRUCT), otherwise the current runtime code."""
    def rows(name):
        # vendored copies of forta-network/labelled-datasets labels/1 (MIT), see data/
        with open(Path(__file__).parent / "data" / f"forta_{name}", encoding="utf-8-sig", newline="") as fh:
            return list(csv.DictReader(fh))

    mal = rows("malicious_smart_contracts.csv")
    phish = [r for r in rows("phishing_scams.csv") if str(r.get("is_contract", "")).strip().lower() == "true"]
    items = [(r["contract_address"].lower(), r["contract_creation_tx"], "malicious_smart_contracts") for r in mal]
    have = {i[0] for i in items}
    items += [(r["address"].lower(), None, "phishing_scams") for r in phish if r["address"].lower() not in have]
    log("forta labels:", len(mal), "malicious contracts,", len(phish), "phishing contracts ->", len(items), "addresses")
    assert len(phish) > 500, "phishing contracts missing"  # all are also in the malicious list (checked)
    got = {"creation_input": 0, "runtime": 0, "empty": 0}
    for b in range(0, len(items), 40):
        chunk = items[b:b + 40]
        txs = _rpc_batch([("eth_getTransactionByHash", [t]) for _, t, _ in chunk if t])
        txmap = dict(zip([a for a, t, _ in chunk if t], txs))
        codes = _rpc_batch([("eth_getCode", [a, "latest"]) for a, _, _ in chunk])
        for (a, t, src), code in zip(chunk, codes):
            tx = txmap.get(a)
            if tx and tx.get("to") is None and len(tx.get("input") or "") > 10:
                got["creation_input"] += 1
                yield tx["input"], True, f"forta-labels/{src}"
            elif code and len(code) > 4:
                got["runtime"] += 1
                yield code, True, f"forta-labels/{src}"
            else:
                got["empty"] += 1
    log("forta labels fetched:", got)


def iter_forta(benign_runtime: int = 0):
    """Forta parquet. For a random sample of `benign_runtime` benign contracts, use their current on-chain runtime
    code instead of the creation code, so both classes contain both code sources (no source shortcut)."""
    from huggingface_hub import hf_hub_download

    pf = pq.ParquetFile(hf_hub_download(FORTA[0], FORTA[1], repo_type="dataset"))
    t = pf.read(columns=["contract_address", "creation_bytecode", "malicious"]).to_pydict()
    rng = np.random.default_rng(SEED)
    benign_idx = [i for i, m in enumerate(t["malicious"]) if not m]
    pick = set(rng.choice(benign_idx, size=min(benign_runtime, len(benign_idx)), replace=False).tolist()) if benign_runtime else set()
    runtime = {}
    picked = sorted(pick)
    for b in range(0, len(picked), 40):
        ch = picked[b:b + 40]
        for i, code in zip(ch, _rpc_batch([("eth_getCode", [t["contract_address"][i], "latest"]) for i in ch])):
            if code and len(code) > 4:
                runtime[i] = code
    log("benign runtime fetched:", len(runtime), "of", len(pick))
    for i, (code, mal) in enumerate(zip(t["creation_bytecode"], t["malicious"])):
        if i in runtime:
            yield runtime[i], bool(mal), "forta/benign-runtime"
        else:
            yield code, bool(mal), "forta"


def iter_flipguard():
    from huggingface_hub import hf_hub_download

    csv.field_size_limit(sys.maxsize)
    path = hf_hub_download("flipguard/malicious-contracts", "malicious-contracts.csv", repo_type="dataset")
    rd = csv.reader(open(path, encoding="utf-8-sig", newline=""))
    next(rd)
    for row in rd:
        yield row[0], row[-1].strip().lower() in ("true", "1"), "flipguard"


def _feat_chunk(chunk):
    codes = [c for c, _ in chunk]
    X = xr.featurize(codes)
    groups = []
    for c in codes:
        ops, _, _ = xr.disassemble(xr.strip_metadata(xr.runtime_part(c)))
        groups.append(hashlib.sha1(" ".join(o if not o.startswith("PUSH") else "PUSH" for o in ops).encode()).hexdigest()[:16])
    return X, groups, [m for _, m in chunk]


def build_dataset(sources: list[str], max_benign: int | None, benign_runtime: int = 0):
    seen: dict[str, int] = {}
    chunks, chunk, srcs = [], [], []
    stats = {"rows": 0, "exact_duplicates": 0, "label_conflicts": 0, "benign_dropped_cap": 0, "per_source": {}}
    n_benign = 0
    its = [iter_forta(benign_runtime)]
    if "forta-labels" in sources:
        its.append(iter_forta_labels())
    if "flipguard" in sources:  # an exact copy of the Forta parquet (checked); kept for completeness
        its.append(iter_flipguard())
    for it in its:
        for code, mal, src in it:
            stats["rows"] += 1
            ps = stats["per_source"].setdefault(src, {"rows": 0, "malicious": 0})
            ps["rows"] += 1
            ps["malicious"] += int(bool(mal))
            h = hashlib.sha1(xr.clean_hex(code).encode()).hexdigest()
            if h in seen:
                stats["exact_duplicates"] += 1
                if seen[h] != int(bool(mal)):
                    stats["label_conflicts"] += 1
                    seen[h] = 1  # keep the malicious label: a copy of a known-bad contract is bad
                continue
            if not mal and max_benign is not None and n_benign >= max_benign:
                stats["benign_dropped_cap"] += 1
                continue
            seen[h] = int(bool(mal))
            n_benign += int(not mal)
            chunk.append((code, bool(mal)))
            srcs.append(src)
            if len(chunk) == 500:
                chunks.append(chunk); chunk = []
    if chunk:
        chunks.append(chunk)
    log("featurizing", sum(len(c) for c in chunks), "unique contracts in", len(chunks), "chunks")
    Xs, G, Y = [], [], []
    with ProcessPoolExecutor() as ex:
        for X, g, y in ex.map(_feat_chunk, chunks, chunksize=2):
            Xs.append(X); G += g; Y += y
    X = sp.vstack(Xs, format="csr")
    y = np.asarray(Y, dtype=np.int8)
    groups = np.asarray(G)
    # a group containing any malicious member is labelled consistently for stratification purposes
    stats.update(unique=int(len(y)), malicious=int(y.sum()), groups=int(len(set(G))), nnz=int(X.nnz))
    return X, y, groups, np.asarray(srcs), stats


# ---------------- evaluation helpers ----------------
def recall_at_fpr(y, p, fpr_target):
    fpr, tpr, _ = roc_curve(y, p)
    return float(np.interp(fpr_target, fpr, tpr))


def metrics_at(y, p, thr):
    yhat = (p >= thr).astype(int)
    tn, fp, fn, tp = confusion_matrix(y, yhat, labels=[0, 1]).ravel()
    return {
        "roc_auc": float(roc_auc_score(y, p)), "pr_auc": float(average_precision_score(y, p)),
        "precision": float(precision_score(y, yhat, zero_division=0)), "recall": float(recall_score(y, yhat)),
        "f1": float(f1_score(y, yhat)), "fpr": float(fp / max(1, fp + tn)),
        "recall_at_fpr_0.1pct": recall_at_fpr(y, p, 0.001), "recall_at_fpr_1pct": recall_at_fpr(y, p, 0.01),
        "confusion": {"tn": int(tn), "fp": int(fp), "fn": int(fn), "tp": int(tp)}, "threshold": float(thr),
        "n": int(len(y)), "positives": int(y.sum()),
    }


def logit(p):
    p = np.clip(p, 1e-7, 1 - 1e-7)
    return np.log(p / (1 - p))


def blend(p_lgb, p_lr, w):
    return 1 / (1 + np.exp(-(w * logit(p_lgb) + (1 - w) * logit(p_lr))))


def by_source(y, p, thr, src):
    out = {}
    for s_ in sorted(set(src)):
        m = src == s_
        yy, pp = y[m], p[m]
        out[s_] = {"n": int(m.sum()), "malicious": int(yy.sum()),
                   "recall": float(((pp >= thr) & (yy == 1)).sum() / max(1, yy.sum())) if yy.sum() else None,
                   "false_positive_rate": float(((pp >= thr) & (yy == 0)).sum() / max(1, (yy == 0).sum())) if (yy == 0).any() else None}
    return out


def best_f1_threshold(y, p):
    pr, rc, th = precision_recall_curve(y, p)
    f1 = 2 * pr * rc / np.maximum(pr + rc, 1e-9)
    i = int(np.nanargmax(f1[:-1]))
    return float(th[i])


LGB_PARAMS = dict(objective="binary", learning_rate=0.03, num_leaves=31, min_data_in_leaf=20, feature_fraction=0.2,
                  bagging_fraction=0.8, bagging_freq=1, lambda_l2=5.0, max_bin=63, verbose=-1, seed=SEED,
                  num_threads=0)


def fit_lgb(Xtr, ytr, Xva=None, yva=None, rounds=4000):
    pos = max(1, int(ytr.sum()))
    params = dict(LGB_PARAMS, scale_pos_weight=float(np.sqrt((len(ytr) - pos) / pos)))
    dtr = lgb.Dataset(Xtr, ytr, free_raw_data=False)
    if Xva is None:
        return lgb.train(params, dtr, num_boost_round=rounds)
    dva = lgb.Dataset(Xva, yva, reference=dtr)
    return lgb.train(params, dtr, num_boost_round=rounds, valid_sets=[dva],
                     callbacks=[lgb.early_stopping(200, verbose=False)])


def fit_lr(Xtr, ytr):
    m = LogisticRegression(C=1.0, class_weight="balanced", max_iter=2000, solver="liblinear")
    # scalar features have very different scales; log-compress counts for the linear model
    return m.fit(_lr_x(Xtr), ytr)


def _lr_x(X):
    X = X.copy()
    X.data = np.log1p(np.abs(X.data))
    return X


def plots(out: Path, y, p, thr, importance):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    fig, ax = plt.subplots(1, 3, figsize=(15, 4.2))
    pr, rc, _ = precision_recall_curve(y, p)
    ax[0].plot(rc, pr); ax[0].set_xlabel("recall"); ax[0].set_ylabel("precision")
    ax[0].set_title(f"Precision-recall (test), AP={average_precision_score(y, p):.3f}")
    fpr, tpr, _ = roc_curve(y, p)
    ax[1].plot(fpr, tpr); ax[1].set_xscale("log"); ax[1].set_xlim(1e-4, 1); ax[1].set_xlabel("false-positive rate (log)")
    ax[1].set_ylabel("true-positive rate"); ax[1].set_title(f"ROC (test), AUC={roc_auc_score(y, p):.4f}")
    names, vals = zip(*importance[:20])
    ax[2].barh(range(len(vals))[::-1], vals); ax[2].set_yticks(range(len(vals))[::-1]); ax[2].set_yticklabels(names, fontsize=7)
    ax[2].set_title("Top features (gain)")
    fig.tight_layout(); fig.savefig(out / "evaluation.png", dpi=130)


def feature_names():
    n_sc = len(xr.SCALAR_NAMES)
    def name(i):
        if i < n_sc:
            return xr.SCALAR_NAMES[i]
        i -= n_sc
        if i < 2**18:
            return f"opcode_ngram_hash_{i}"
        return f"selector_hash_{i - 2**18}"
    return name


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="out")
    ap.add_argument("--sources", default="forta,forta-labels")
    ap.add_argument("--max-benign", type=int, default=None, help="cap benign contracts (speed); default keep all")
    ap.add_argument("--folds", type=int, default=5)
    ap.add_argument("--benign-runtime", type=int, default=8000, help="benign contracts scored from on-chain runtime code")
    a = ap.parse_args()
    out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    t0 = time.time()

    X, y, groups, src, stats = build_dataset(a.sources.split(","), a.max_benign, a.benign_runtime)
    log("data", json.dumps(stats))

    # ---- hold-out test set: 20% of groups, stratified ----
    outer = StratifiedGroupKFold(n_splits=5, shuffle=True, random_state=SEED)
    tr_idx, te_idx = next(outer.split(X, y, groups))
    Xtr, ytr, gtr = X[tr_idx], y[tr_idx], groups[tr_idx]
    Xte, yte = X[te_idx], y[te_idx]
    assert not set(gtr) & set(groups[te_idx]), "group leakage"
    log(f"train {len(ytr)} ({ytr.sum()} malicious), test {len(yte)} ({yte.sum()} malicious)")

    # ---- 5-fold grouped CV on the training part ----
    oof_lgb = np.zeros(len(ytr)); oof_lr = np.zeros(len(ytr)); iters = []
    cv = StratifiedGroupKFold(n_splits=a.folds, shuffle=True, random_state=SEED + 1)
    for k, (i, j) in enumerate(cv.split(Xtr, ytr, gtr)):
        m = fit_lgb(Xtr[i], ytr[i], Xtr[j], ytr[j])
        oof_lgb[j] = m.predict(Xtr[j], num_iteration=m.best_iteration)
        iters.append(m.best_iteration or 2000)
        oof_lr[j] = fit_lr(Xtr[i], ytr[i]).predict_proba(_lr_x(Xtr[j]))[:, 1]
        log(f"fold {k}: lgb AP={average_precision_score(ytr[j], oof_lgb[j]):.3f} (it {iters[-1]}), "
            f"lr AP={average_precision_score(ytr[j], oof_lr[j]):.3f}")
    # ---- choose the release model on OOF predictions: LightGBM, logistic regression, or a logit blend ----
    weights = [1.0, 0.75, 0.5, 0.25, 0.0]
    ap_w = {w: float(average_precision_score(ytr, blend(oof_lgb, oof_lr, w))) for w in weights}
    w_best = max(ap_w, key=ap_w.get)
    oof_best = blend(oof_lgb, oof_lr, w_best)
    thr = best_f1_threshold(ytr, oof_best)
    cv_res = {"lightgbm": metrics_at(ytr, oof_lgb, best_f1_threshold(ytr, oof_lgb)),
              "logistic_regression": metrics_at(ytr, oof_lr, best_f1_threshold(ytr, oof_lr)),
              "release": metrics_at(ytr, oof_best, thr), "blend_weight_lightgbm": w_best,
              "average_precision_by_weight": ap_w, "lightgbm_iterations": iters}
    log("cv AP by blend weight", ap_w, "-> w", w_best)
    log("cv", json.dumps({k: {m: round(v, 4) for m, v in r.items() if isinstance(v, float)} for k, r in cv_res.items() if isinstance(r, dict) and "roc_auc" in r}))

    # ---- final models on all training groups, scored once on the test set ----
    rounds = int(np.median(iters) * 1.1)
    final = fit_lgb(Xtr, ytr, rounds=rounds)
    lr_final = fit_lr(Xtr, ytr)
    p_lgb_te = final.predict(Xte)
    p_lr_te = lr_final.predict_proba(_lr_x(Xte))[:, 1]
    p_te = blend(p_lgb_te, p_lr_te, w_best)
    test = metrics_at(yte, p_te, thr)
    test_lgb = metrics_at(yte, p_lgb_te, cv_res["lightgbm"]["threshold"])
    test_lr = metrics_at(yte, p_lr_te, cv_res["logistic_regression"]["threshold"])
    diag = by_source(yte, p_te, thr, src[te_idx])
    log("test release", json.dumps({k: v for k, v in test.items() if k != "confusion"}), test["confusion"])
    log("test lightgbm-only", json.dumps({k: round(v, 4) for k, v in test_lgb.items() if isinstance(v, float)}))
    log("test logistic-only", json.dumps({k: round(v, 4) for k, v in test_lr.items() if isinstance(v, float)}))
    log("test by source", json.dumps(diag))

    gain = final.feature_importance("gain")
    nm = feature_names()
    importance = sorted(((nm(i), float(g)) for i, g in enumerate(gain) if g > 0), key=lambda t: -t[1])[:40]
    final.save_model(str(out / "model.txt"))
    np.savez_compressed(out / "linear.npz", coef=lr_final.coef_.ravel().astype(np.float32), intercept=np.float32(lr_final.intercept_[0]))
    medium = float(np.quantile(p_te[yte == 0], 0.99))  # top 1% of benign test scores
    cfg = {"feature_version": xr.FEATURE_VERSION, "threshold": thr, "medium_threshold": min(medium, thr),
           "blend_weight_lightgbm": w_best, "rounds": rounds, "lgb_params": LGB_PARAMS}
    (out / "config.json").write_text(json.dumps(cfg, indent=2))
    report = {"data": stats, "split": {"train": int(len(ytr)), "train_malicious": int(ytr.sum()),
              "test": int(len(yte)), "test_malicious": int(yte.sum()), "grouping": "opcode skeleton (PUSH args removed)"},
              "cv_5fold": cv_res, "test": {"release": test, "lightgbm": test_lgb, "logistic_regression_baseline": test_lr},
              "test_by_source": diag, "top_features": importance, "train_seconds": round(time.time() - t0)}
    (out / "metrics.json").write_text(json.dumps(report, indent=2))
    plots(out, yte, p_te, thr, importance)
    # self-check: the shipped inference code reproduces the test scores
    shipped = xr.ContractRiskModel(out)
    assert np.allclose(shipped.score_matrix(Xte[:200]), p_te[:200], atol=1e-3), "inference mismatch"
    log("done in", round(time.time() - t0), "s")


if __name__ == "__main__":
    main()
