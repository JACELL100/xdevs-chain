"""Score a contract with xDEVS Contract Risk: python example.py <address> [rpc_url]"""
import json
import sys
import urllib.request

from huggingface_hub import snapshot_download

d = snapshot_download("JacellJamble/xdevs-contract-risk")
sys.path.insert(0, d)
from xdevs_contract_risk import ContractRiskModel  # noqa: E402

address = sys.argv[1]
rpc = sys.argv[2] if len(sys.argv) > 2 else "https://rpc.xdevschain.tech"
req = urllib.request.Request(rpc, json.dumps({"jsonrpc": "2.0", "id": 1, "method": "eth_getCode", "params": [address, "latest"]}).encode(),
                             {"content-type": "application/json", "user-agent": "xdevs-contract-risk-example"})
code = json.load(urllib.request.urlopen(req, timeout=15))["result"]
print(json.dumps(ContractRiskModel(d).predict(code), indent=2))
