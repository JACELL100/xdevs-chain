"""Score real mainnet contracts with the Python model and write parity.json for parity.mjs (browser port check)."""
import csv
import json
import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from xdevs_contract_risk import ContractRiskModel  # noqa: E402

RPC = "https://ethereum-rpc.publicnode.com"
KNOWN = [
    "0xdAC17F958D2ee523a2206206994597C13D831ec7", "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2", "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D",
    "0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984", "0x6B175474E89094C44Da98b954EedeAC495271d0F", "0x000000000022D473030F116dDEE9F6B43aC78BA3",
    "0x00000000000000ADc04C56Bf30aC9d3c0aAF14dC", "0xE592427A0AEce92De3Edee1F18E0157C05861564", "0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f",
    "0x514910771AF9Ca656af840dff83E8264EcF986CA",
]


def codes_for(addrs):
    r = requests.post(RPC, json=[{"jsonrpc": "2.0", "id": i, "method": "eth_getCode", "params": [a, "latest"]} for i, a in enumerate(addrs)],
                      headers={"user-agent": "xdevs-contract-risk-parity"}, timeout=60)
    r.raise_for_status()
    res = {x["id"]: x.get("result") for x in r.json()}
    return [res.get(i) or "0x" for i in range(len(addrs))]


def main(model_dir, out):
    here = Path(__file__).resolve().parent.parent
    mal = [r["contract_address"] for r in csv.DictReader(open(here / "data" / "forta_malicious_smart_contracts.csv", encoding="utf-8-sig"))]
    addrs = KNOWN + mal[::37][:20]
    codes = []
    for b in range(0, len(addrs), 10):
        for k in range(4):
            try:
                codes += codes_for(addrs[b:b + 10]); break
            except Exception as e:
                print("retry", e); time.sleep(5 * (k + 1))
        else:
            raise SystemExit("RPC unavailable")
    codes = [c for c in codes if len(c) > 4]
    codes.append("0x6080604052" + "60016000556000600155" * 40 + "f3fe" + "6080604052" + "3360005260206000f3" * 30)  # synthetic creation-style code
    m = ContractRiskModel(model_dir)
    scores = [float(s) for s in m.score(codes)]
    Path(out).write_text(json.dumps({"codes": codes, "scores": scores}))
    print("parity samples", len(codes), "flagged", sum(s >= m.threshold for s in scores))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
