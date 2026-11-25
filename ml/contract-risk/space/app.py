"""Gradio demo for xDEVS Contract Risk."""
import gradio as gr
import requests
from huggingface_hub import snapshot_download

from xdevs_contract_risk import ContractRiskModel, clean_hex

MODEL = ContractRiskModel(snapshot_download("__MODEL_REPO__", allow_patterns=["model.txt", "config.json"]))
RPCS = {
    "xDEVS Chain Enclave (testnet)": "https://rpc.xdevschain.tech",
    "Ethereum mainnet": "https://ethereum-rpc.publicnode.com",
}


def get_code(address: str, network: str) -> str:
    r = requests.post(RPCS[network], json={"jsonrpc": "2.0", "id": 1, "method": "eth_getCode", "params": [address.strip(), "latest"]},
                      headers={"user-agent": "xdevs-contract-risk-demo"}, timeout=15)
    r.raise_for_status()
    return r.json()["result"]


def score(address: str, network: str, bytecode: str):
    code = bytecode.strip() if bytecode.strip() else get_code(address, network)
    if len(clean_hex(code)) < 4:
        return "No contract code at this address (an ordinary account or an empty address).", {}
    res = MODEL.predict(code)
    level = {"high": "High risk", "medium": "Unusual: review carefully", "low": "No known malicious pattern"}[res["risk_level"]]
    head = f"### {level}\nRisk score **{res['risk_score']:.3f}** (flagged at ≥ {res['threshold']:.3f}). A score is advice, not proof."
    return head, res["signals"]


with gr.Blocks(title="xDEVS Contract Risk") as demo:
    gr.Markdown("# xDEVS Contract Risk\nScores a contract's bytecode for patterns seen in malicious contracts (drainers, phishing and exploit contracts). Built for the safety checks of [xDEVS Chain](https://xdevschain.tech).")
    with gr.Row():
        addr = gr.Textbox(label="Contract address", placeholder="0x…")
        net = gr.Dropdown(list(RPCS), value="Ethereum mainnet", label="Network")
    code = gr.Textbox(label="…or paste bytecode instead (runtime or creation code)", lines=3)
    btn = gr.Button("Score contract", variant="primary")
    out = gr.Markdown()
    sig = gr.JSON(label="Signals")
    btn.click(score, [addr, net, code], [out, sig])

demo.launch()
