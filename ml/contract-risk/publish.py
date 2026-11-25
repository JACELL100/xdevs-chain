"""Upload the trained model (model repo) and the demo (Gradio Space) to Hugging Face. Needs HF_TOKEN."""
from __future__ import annotations

import argparse
import json
import os
import shutil
import tempfile
from pathlib import Path

from huggingface_hub import HfApi

HERE = Path(__file__).parent


def fmt(x, pct=False):
    return f"{x * 100:.1f}%" if pct else f"{x:.4f}"


def card(m: dict, repo: str, space: str) -> str:
    t, b, cv, d, s = m["test"]["release"], m["test"]["logistic_regression_baseline"], m["cv_5fold"]["release"], m["data"], m["split"]
    c = t["confusion"]
    return (HERE / "MODEL_CARD.md").read_text(encoding="utf-8").format(
        repo=repo, space=space,
        rows=f'{d["rows"]:,}', unique=f'{d["unique"]:,}', malicious=f'{d["malicious"]:,}', groups=f'{d["groups"]:,}',
        dups=f'{d["exact_duplicates"]:,}',
        train=f'{s["train"]:,}', train_mal=s["train_malicious"], test=f'{s["test"]:,}', test_mal=s["test_malicious"],
        roc=fmt(t["roc_auc"]), ap=fmt(t["pr_auc"]), prec=fmt(t["precision"], True), rec=fmt(t["recall"], True),
        f1=fmt(t["f1"]), fpr=fmt(t["fpr"], True), r01=fmt(t["recall_at_fpr_0.1pct"], True), r1=fmt(t["recall_at_fpr_1pct"], True),
        tp=c["tp"], fp=c["fp"], fn=c["fn"], tn=f'{c["tn"]:,}', thr=fmt(t["threshold"]),
        b_roc=fmt(b["roc_auc"]), b_ap=fmt(b["pr_auc"]), b_r1=fmt(b["recall_at_fpr_1pct"], True),
        cv_roc=fmt(cv["roc_auc"]), cv_ap=fmt(cv["pr_auc"]), cv_r1=fmt(cv["recall_at_fpr_1pct"], True),
        w=m["cv_5fold"]["blend_weight_lightgbm"],
        g_roc=fmt(m["test"]["lightgbm"]["roc_auc"]), g_ap=fmt(m["test"]["lightgbm"]["pr_auc"]), g_r1=fmt(m["test"]["lightgbm"]["recall_at_fpr_1pct"], True),
        bysource="\n".join(["| Source | Contracts | Malicious | Recall | False-positive rate |", "|---|---|---|---|---|"] + [
            f'| {k} | {v["n"]:,} | {v["malicious"]:,} | {"-" if v["recall"] is None else fmt(v["recall"], True)} | '
            f'{"-" if v["false_positive_rate"] is None else fmt(v["false_positive_rate"], True)} |' for k, v in m["test_by_source"].items()]),
        sources=", ".join(f'{k}: {v["rows"]:,} rows ({v["malicious"]:,} malicious)' for k, v in d["per_source"].items()),
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-dir", default="out")
    ap.add_argument("--repo", default="xdevs-contract-risk")
    ap.add_argument("--space", default="xdevs-contract-risk-demo")
    ap.add_argument("--space-only", action="store_true", help="only (re)publish the demo Space")
    a = ap.parse_args()
    api = HfApi(token=os.environ["HF_TOKEN"])
    user = api.whoami()["name"]
    repo, space = f"{user}/{a.repo}", f"{user}/{a.space}"
    md = Path(a.model_dir)
    if not a.space_only:
        publish_model(api, md, repo, space)
    publish_space(api, repo, space)


def publish_model(api, md, repo, space):
    metrics = json.loads((md / "metrics.json").read_text())
    with tempfile.TemporaryDirectory() as tmp:
        t = Path(tmp)
        for f in ("model.txt", "linear.npz", "config.json", "metrics.json", "evaluation.png"):
            shutil.copy(md / f, t / f)
        for f in ("xdevs_contract_risk.py", "requirements-inference.txt", "example.py"):
            shutil.copy(HERE / f, t / f)
        (t / "README.md").write_text(card(metrics, repo, space), encoding="utf-8")
        api.create_repo(repo, repo_type="model", exist_ok=True)
        api.upload_folder(repo_id=repo, repo_type="model", folder_path=str(t), commit_message="Train and publish xDEVS Contract Risk")
    print("model:", f"https://huggingface.co/{repo}")


def publish_space(api, repo, space):
    with tempfile.TemporaryDirectory() as tmp:
        t = Path(tmp)
        shutil.copytree(HERE / "space", t, dirs_exist_ok=True)
        shutil.copy(HERE / "xdevs_contract_risk.py", t / "xdevs_contract_risk.py")
        for f in ("app.py", "README.md"):
            txt = (t / f).read_text(encoding="utf-8").replace("__MODEL_REPO__", repo).replace("__MODEL__", repo)
            (t / f).write_text(txt, encoding="utf-8")
        try:
            api.create_repo(space, repo_type="space", space_sdk="gradio", exist_ok=True)
        except Exception as e:  # e.g. 402 when the account may not create Spaces via the API: create it in the UI
            if not api.repo_exists(space, repo_type="space"):
                raise SystemExit(f"Could not create the Space {space} ({e}). Create it at https://huggingface.co/new-space "
                                 "(SDK: Gradio, hardware: CPU basic, public) and run again with --space-only.")
        api.upload_folder(repo_id=space, repo_type="space", folder_path=str(t), commit_message="Demo for xDEVS Contract Risk")
    print("space:", f"https://huggingface.co/spaces/{space}")


if __name__ == "__main__":
    main()
