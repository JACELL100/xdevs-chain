"""Export a trained model directory for the browser: trees.json (compact LightGBM trees), linear.bin (float32
coefficients, little-endian) and config.json."""
import json
import shutil
import sys
from pathlib import Path

import lightgbm as lgb
import numpy as np


def compact(tree):
    t = {"f": [], "th": [], "dl": [], "mt": [], "l": [], "r": [], "v": []}

    def walk(node):
        if "leaf_value" in node:
            t["v"].append(node["leaf_value"])
            return ~(len(t["v"]) - 1)
        i = len(t["f"])
        for k in ("f", "th", "dl", "mt", "l", "r"):
            t[k].append(0)
        assert node["decision_type"] == "<=", node["decision_type"]
        t["f"][i] = node["split_feature"]
        t["th"][i] = node["threshold"]
        t["dl"][i] = 1 if node["default_left"] else 0
        t["mt"][i] = {"None": 0, "Zero": 1, "NaN": 2}[node["missing_type"]]
        t["l"][i] = walk(node["left_child"])
        t["r"][i] = walk(node["right_child"])
        return i

    root = walk(tree["tree_structure"])
    if root < 0:  # single-leaf tree: one dummy node that always goes left to the leaf
        t = {"f": [0], "th": [float("inf")], "dl": [1], "mt": [0], "l": [root], "r": [root], "v": t["v"]}
    return t


def export(model_dir, out_dir):
    md, out = Path(model_dir), Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    dump = lgb.Booster(model_file=str(md / "model.txt")).dump_model()
    assert dump["objective"].startswith("binary"), dump["objective"]
    trees = [compact(t) for t in dump["tree_info"]]
    (out / "trees.json").write_text(json.dumps(trees, separators=(",", ":")))
    lin = np.load(md / "linear.npz")
    (out / "linear.bin").write_bytes(lin["coef"].astype("<f4").tobytes())
    cfg = json.loads((md / "config.json").read_text())
    cfg["intercept"] = float(lin["intercept"])
    (out / "config.json").write_text(json.dumps(cfg))
    if (md / "metrics.json").exists():
        shutil.copy(md / "metrics.json", out / "metrics.json")
    print("exported", len(trees), "trees to", out)


if __name__ == "__main__":
    export(sys.argv[1], sys.argv[2])
