"""Train and evaluate xDEVS Contract Risk (malicious-contract classifier on EVM bytecode).

Data: Forta "Malicious Smart Contract Classification Dataset" (MIT) and, if enabled, the FlipGuard
malicious-contracts dataset (MIT). Both are Ethereum mainnet contracts with creation bytecode and a label.

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
def iter_forta():
    from huggingface_hub import hf_hub_download

    pf = pq.ParquetFile(hf_hub_download(FORTA[0], FORTA[1], repo_type="dataset"))
    for batch in pf.iter_batches(batch_size=2000, columns=["creation_bytecode", "malicious"]):
        d = batch.to_pydict()
        yield from zip(d["creation_bytecode"], d["malicious"], ["forta"] * len(d["malicious"]))


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


def build_dataset(sources: list[str], max_benign: int | None):
    seen: dict[str, int] = {}
    chunks, chunk = [], []
    stats = {"rows": 0, "exact_duplicates": 0, "label_conflicts": 0, "benign_dropped_cap": 0, "per_source": {}}
    n_benign = 0
    its = [iter_forta()] + ([iter_flipguard()] if "flipguard" in sources else [])
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
    return X, y, groups, stats


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


def best_f1_threshold(y, p):
    pr, rc, th = precision_recall_curve(y, p)
    f1 = 2 * pr * rc / np.maximum(pr + rc, 1e-9)
    i = int(np.nanargmax(f1[:-1]))
    return float(th[i])


LGB_PARAMS = dict(objective="binary", learning_rate=0.05, num_leaves=63, min_data_in_leaf=10, feature_fraction=0.3,
                  bagging_fraction=0.8, bagging_freq=1, lambda_l2=1.0, max_bin=63, verbose=-1, seed=SEED,
                  num_threads=0)


def fit_lgb(Xtr, ytr, Xva=None, yva=None, rounds=2000):
    pos = max(1, int(ytr.sum()))
    params = dict(LGB_PARAMS, scale_pos_weight=float(np.sqrt((len(ytr) - pos) / pos)))
    dtr = lgb.Dataset(Xtr, ytr, free_raw_data=False)
    if Xva is None:
        return lgb.train(params, dtr, num_boost_round=rounds)
    dva = lgb.Dataset(Xva, yva, reference=dtr)
    return lgb.train(params, dtr, num_boost_round=rounds, valid_sets=[dva],
                     callbacks=[lgb.early_stopping(100, verbose=False)])


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
    ap.add_argument("--sources", default="forta,flipguard")
    ap.add_argument("--max-benign", type=int, default=None, help="cap benign contracts (speed); default keep all")
    ap.add_argument("--folds", type=int, default=5)
    a = ap.parse_args()
    out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    t0 = time.time()

    X, y, groups, stats = build_dataset(a.sources.split(","), a.max_benign)
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
    thr = best_f1_threshold(ytr, oof_lgb)
    cv_res = {"lightgbm": metrics_at(ytr, oof_lgb, thr),
              "logistic_regression": metrics_at(ytr, oof_lr, best_f1_threshold(ytr, oof_lr)),
              "lightgbm_iterations": iters}
    log("cv", json.dumps({k: {m: round(v, 4) for m, v in r.items() if isinstance(v, float)} for k, r in cv_res.items() if isinstance(r, dict)}))

    # ---- final model on all training groups, scored once on the test set ----
    rounds = int(np.median(iters) * 1.1)
    final = fit_lgb(Xtr, ytr, rounds=rounds)
    p_te = final.predict(Xte)
    test = metrics_at(yte, p_te, thr)
    lr_te = fit_lr(Xtr, ytr).predict_proba(_lr_x(Xte))[:, 1]
    test_lr = metrics_at(yte, lr_te, cv_res["logistic_regression"]["threshold"])
    log("test lightgbm", json.dumps({k: v for k, v in test.items() if k != "confusion"}), test["confusion"])
    log("test logistic", json.dumps({k: round(v, 4) for k, v in test_lr.items() if isinstance(v, float)}))

    gain = final.feature_importance("gain")
    nm = feature_names()
    importance = sorted(((nm(i), float(g)) for i, g in enumerate(gain) if g > 0), key=lambda t: -t[1])[:40]
    final.save_model(str(out / "model.txt"))
    medium = float(np.quantile(p_te[yte == 0], 0.99))  # top 1% of benign test scores
    cfg = {"feature_version": xr.FEATURE_VERSION, "threshold": thr, "medium_threshold": min(medium, thr),
           "rounds": rounds, "lgb_params": LGB_PARAMS}
    (out / "config.json").write_text(json.dumps(cfg, indent=2))
    report = {"data": stats, "split": {"train": int(len(ytr)), "train_malicious": int(ytr.sum()),
              "test": int(len(yte)), "test_malicious": int(yte.sum()), "grouping": "opcode skeleton (PUSH args removed)"},
              "cv_5fold": cv_res, "test": {"lightgbm": test, "logistic_regression_baseline": test_lr},
              "top_features": importance, "train_seconds": round(time.time() - t0)}
    (out / "metrics.json").write_text(json.dumps(report, indent=2))
    plots(out, yte, p_te, thr, importance)
    log("done in", round(time.time() - t0), "s")


if __name__ == "__main__":
    main()
