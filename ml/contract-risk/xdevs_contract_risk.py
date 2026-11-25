"""xDEVS Contract Risk: bytecode featurizer and inference.

Turns EVM bytecode (runtime code from eth_getCode, or creation code from a deployment) into
features, and scores it with the trained LightGBM model. Stateless: no fitted vocabulary, so the
same code runs in training, in the Hugging Face Space and in a node-side service.
"""
from __future__ import annotations

import json
import re
from collections import Counter
from pathlib import Path

import numpy as np
import scipy.sparse as sp
from sklearn.feature_extraction.text import HashingVectorizer

FEATURE_VERSION = 2

# ---- opcode table (Cancun/Osaka) ----
OPCODES: dict[int, str] = {
    0x00: "STOP", 0x01: "ADD", 0x02: "MUL", 0x03: "SUB", 0x04: "DIV", 0x05: "SDIV", 0x06: "MOD", 0x07: "SMOD",
    0x08: "ADDMOD", 0x09: "MULMOD", 0x0A: "EXP", 0x0B: "SIGNEXTEND",
    0x10: "LT", 0x11: "GT", 0x12: "SLT", 0x13: "SGT", 0x14: "EQ", 0x15: "ISZERO", 0x16: "AND", 0x17: "OR",
    0x18: "XOR", 0x19: "NOT", 0x1A: "BYTE", 0x1B: "SHL", 0x1C: "SHR", 0x1D: "SAR", 0x1E: "CLZ",
    0x20: "KECCAK256",
    0x30: "ADDRESS", 0x31: "BALANCE", 0x32: "ORIGIN", 0x33: "CALLER", 0x34: "CALLVALUE", 0x35: "CALLDATALOAD",
    0x36: "CALLDATASIZE", 0x37: "CALLDATACOPY", 0x38: "CODESIZE", 0x39: "CODECOPY", 0x3A: "GASPRICE",
    0x3B: "EXTCODESIZE", 0x3C: "EXTCODECOPY", 0x3D: "RETURNDATASIZE", 0x3E: "RETURNDATACOPY", 0x3F: "EXTCODEHASH",
    0x40: "BLOCKHASH", 0x41: "COINBASE", 0x42: "TIMESTAMP", 0x43: "NUMBER", 0x44: "PREVRANDAO", 0x45: "GASLIMIT",
    0x46: "CHAINID", 0x47: "SELFBALANCE", 0x48: "BASEFEE", 0x49: "BLOBHASH", 0x4A: "BLOBBASEFEE",
    0x50: "POP", 0x51: "MLOAD", 0x52: "MSTORE", 0x53: "MSTORE8", 0x54: "SLOAD", 0x55: "SSTORE", 0x56: "JUMP",
    0x57: "JUMPI", 0x58: "PC", 0x59: "MSIZE", 0x5A: "GAS", 0x5B: "JUMPDEST", 0x5C: "TLOAD", 0x5D: "TSTORE",
    0x5E: "MCOPY", 0x5F: "PUSH0",
    0xF0: "CREATE", 0xF1: "CALL", 0xF2: "CALLCODE", 0xF3: "RETURN", 0xF4: "DELEGATECALL", 0xF5: "CREATE2",
    0xFA: "STATICCALL", 0xFD: "REVERT", 0xFE: "INVALID", 0xFF: "SELFDESTRUCT",
}
for i in range(32):
    OPCODES[0x60 + i] = f"PUSH{i + 1}"
for i in range(16):
    OPCODES[0x80 + i] = f"DUP{i + 1}"
    OPCODES[0x90 + i] = f"SWAP{i + 1}"
for i in range(5):
    OPCODES[0xA0 + i] = f"LOG{i}"

# Opcodes whose counts are interesting on their own (fund movement, control of other contracts, env reads).
SCALAR_OPS = [
    "CALL", "DELEGATECALL", "STATICCALL", "CALLCODE", "SELFDESTRUCT", "CREATE", "CREATE2", "SSTORE", "SLOAD",
    "ORIGIN", "CALLER", "CALLVALUE", "BALANCE", "SELFBALANCE", "GAS", "TIMESTAMP", "NUMBER", "COINBASE",
    "BLOCKHASH", "EXTCODESIZE", "EXTCODEHASH", "EXTCODECOPY", "LOG0", "LOG1", "LOG2", "LOG3", "LOG4",
    "TSTORE", "TLOAD", "JUMPDEST", "REVERT", "PUSH20", "PUSH32", "CHAINID", "PREVRANDAO", "GASPRICE",
]
# Well-known 4-byte selectors (ERC-20/721, ownership, multicall/flash-loan style entry points).
KNOWN_SELECTORS = {
    "a9059cbb": "transfer", "23b872dd": "transferFrom", "095ea7b3": "approve", "70a08231": "balanceOf",
    "dd62ed3e": "allowance", "18160ddd": "totalSupply", "42842e0e": "safeTransferFrom", "a22cb465": "setApprovalForAll",
    "8da5cb5b": "owner", "715018a6": "renounceOwnership", "f2fde38b": "transferOwnership", "3ccfd60b": "withdraw",
    "2e1a7d4d": "withdraw(uint256)", "d0e30db0": "deposit", "ac9650d8": "multicall", "5cffe9de": "flashLoan",
    "ab9c4b5d": "flashLoan(aave)", "490e6cbc": "flash(uniswapV3)", "022c0d9f": "swap(uniswapV2)",
    "128acb08": "swap(uniswapV3)", "38ed1739": "swapExactTokensForTokens", "7ff36ab5": "swapExactETHForTokens",
    "1249c58b": "mint", "40c10f19": "mint(address,uint256)", "42966c68": "burn", "e8e33700": "addLiquidity",
    "f305d719": "addLiquidityETH", "c9567bf9": "openTrading", "8f70ccf7": "setTrading", "3b124fe7": "setFee?",
    "1cff79cd": "execute(address,bytes)", "b61d27f6": "execute(address,uint256,bytes)", "6a761202": "execTransaction",
    "e9e05c42": "unknown_claim", "4e71d92d": "claim", "379607f5": "claim(uint256)", "f4f3b200": "unknown_airdrop",
}

_RETURN_INVALID = re.compile(r"f3fe")


def clean_hex(code: str) -> str:
    code = (code or "").strip().lower()
    if code.startswith("0x"):
        code = code[2:]
    if len(code) % 2:
        code = code[:-1]
    return code


def runtime_part(code_hex: str) -> str:
    """Creation code = constructor + runtime (+ metadata, constructor args). solc ends the constructor with
    RETURN INVALID (f3fe); the runtime follows. Runtime code (eth_getCode) usually has no such marker in its
    first bytes, so it is returned unchanged. Keeps training (creation code) and inference (runtime) aligned."""
    code_hex = clean_hex(code_hex)
    m = _RETURN_INVALID.search(code_hex, 0, 4000)  # the constructor is short
    if m and m.start() % 2 == 0 and m.end() < len(code_hex) - 20:
        return code_hex[m.end():]
    return code_hex


_CBOR_MARKERS = ("a264697066735822", "a2646970667358", "a165627a7a72305820", "a265627a7a72315820", "a164736f6c6343")


def strip_metadata(code_hex: str) -> str:
    """Cut the solc CBOR metadata blob and everything after it (constructor arguments in creation code), so that
    creation-derived and on-chain runtime code look the same and compiler hashes are not features."""
    cut = -1
    for m in _CBOR_MARKERS:
        i = code_hex.rfind(m)
        if i > cut and i % 2 == 0:
            cut = i
    return code_hex[:cut] if cut > 0 else code_hex


def disassemble(code_hex: str) -> tuple[list[str], list[str], int]:
    """Returns (opcode names with PUSHn kept, PUSH4 selector candidates, n_bytes)."""
    b = bytes.fromhex(code_hex)
    ops: list[str] = []
    sels: list[str] = []
    i, n = 0, len(b)
    while i < n:
        op = b[i]
        name = OPCODES.get(op, "UNK")
        ops.append(name)
        if 0x60 <= op <= 0x7F:
            k = op - 0x5F
            if k == 4 and i + 5 <= n:
                sels.append(b[i + 1:i + 5].hex())
            i += 1 + k
        else:
            i += 1
    return ops, sels, n


_OPS_VEC = HashingVectorizer(n_features=2**18, ngram_range=(1, 2), token_pattern=r"\S+", lowercase=False,
                             alternate_sign=False, norm="l1")
_SEL_VEC = HashingVectorizer(n_features=2**16, token_pattern=r"\S+", lowercase=False, alternate_sign=False,
                             binary=True, norm=None)
SCALAR_NAMES = (["log_bytes", "log_ops", "n_unique_selectors", "n_known_selectors", "erc20_like", "frac_unknown"]
                + [f"cnt_{o}" for o in SCALAR_OPS] + [f"frac_{o}" for o in SCALAR_OPS]
                + [f"sel_{v}" for v in KNOWN_SELECTORS.values()])


def featurize_one(code: str) -> tuple[str, str, np.ndarray]:
    h = strip_metadata(runtime_part(code))
    ops, sels, nbytes = disassemble(h)
    c = Counter(ops)
    nops = max(1, len(ops))
    sel_set = set(sels)
    known = [v for k, v in KNOWN_SELECTORS.items() if k in sel_set]
    erc20 = int(all(s in sel_set for s in ("a9059cbb", "095ea7b3", "70a08231")))
    scal = [np.log1p(nbytes), np.log1p(len(ops)), len(sel_set), len(known), erc20, c.get("UNK", 0) / nops]
    scal += [c.get(o, 0) for o in SCALAR_OPS]
    scal += [c.get(o, 0) / nops for o in SCALAR_OPS]
    scal += [int(v in known) for v in KNOWN_SELECTORS.values()]
    op_text = " ".join("PUSH" if o.startswith("PUSH") and o != "PUSH0" else o for o in ops)
    return op_text, " ".join(f"s{s}" for s in sel_set), np.asarray(scal, dtype=np.float32)


def featurize(codes: list[str]) -> sp.csr_matrix:
    parts = [featurize_one(c) for c in codes]
    X_ops = _OPS_VEC.transform([p[0] for p in parts])
    X_sel = _SEL_VEC.transform([p[1] for p in parts])
    X_sc = sp.csr_matrix(np.vstack([p[2] for p in parts]))
    return sp.hstack([X_sc, X_ops, X_sel], format="csr", dtype=np.float32)


def explain_one(code: str) -> dict:
    """Human-readable signals for one contract (used by the demo; not by the model)."""
    h = strip_metadata(runtime_part(code))
    ops, sels, nbytes = disassemble(h)
    c = Counter(ops)
    sel_set = set(sels)
    return {
        "bytes": nbytes,
        "known_functions": [v for k, v in KNOWN_SELECTORS.items() if k in sel_set],
        "notable_opcodes": {o: c[o] for o in ("DELEGATECALL", "SELFDESTRUCT", "CALL", "CREATE2", "ORIGIN", "PUSH20") if c.get(o)},
        "erc20_like": all(s in sel_set for s in ("a9059cbb", "095ea7b3", "70a08231")),
    }


class ContractRiskModel:
    def __init__(self, model_dir: str | Path):
        import lightgbm as lgb

        d = Path(model_dir)
        self.booster = lgb.Booster(model_file=str(d / "model.txt"))
        self.config = json.loads((d / "config.json").read_text())
        assert self.config["feature_version"] == FEATURE_VERSION, "feature version mismatch"
        self.threshold = float(self.config["threshold"])
        self.w = float(self.config.get("blend_weight_lightgbm", 1.0))
        lin = np.load(d / "linear.npz") if (d / "linear.npz").exists() else None
        self.coef = None if lin is None else lin["coef"]
        self.intercept = None if lin is None else float(lin["intercept"])

    def score_matrix(self, X: sp.csr_matrix) -> np.ndarray:
        """LightGBM and a logistic model on log-compressed features, blended in logit space."""
        p_lgb = np.clip(self.booster.predict(X), 1e-7, 1 - 1e-7)
        if self.coef is None or self.w >= 1.0:
            return p_lgb
        Xl = X.copy()
        Xl.data = np.log1p(np.abs(Xl.data))
        z = self.w * np.log(p_lgb / (1 - p_lgb)) + (1 - self.w) * (Xl @ self.coef + self.intercept)
        return 1 / (1 + np.exp(-z))

    def score(self, codes: list[str]) -> np.ndarray:
        empty = np.array([len(clean_hex(c)) < 4 for c in codes])
        p = self.score_matrix(featurize(codes))
        p[empty] = 0.0  # no code: an EOA or empty account, nothing to score
        return p

    def predict(self, code: str) -> dict:
        p = float(self.score([code])[0])
        level = "high" if p >= self.threshold else ("medium" if p >= self.config.get("medium_threshold", self.threshold / 3) else "low")
        return {"risk_score": round(p, 4), "risk_level": level, "threshold": self.threshold, "signals": explain_one(code)}
