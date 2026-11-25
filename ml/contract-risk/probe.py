"""Stream the FlipGuard CSV (10 GB) and count labels, unique bytecodes, and overlap with Forta."""
import csv, hashlib, sys, io
import requests
import pyarrow.parquet as pq
from huggingface_hub import hf_hub_download

csv.field_size_limit(sys.maxsize)
forta = pq.read_table(hf_hub_download("forta/malicious-smart-contract-dataset", "malicious_contract_training_dataset_final.parquet", repo_type="dataset"), columns=["creation_bytecode", "malicious"]).to_pandas()
fh = {hashlib.sha1(b.encode()).hexdigest(): m for b, m in zip(forta.creation_bytecode, forta.malicious)}
print("forta unique bytecodes", len(fh), "malicious unique", sum(fh.values()))
url = "https://huggingface.co/datasets/flipguard/malicious-contracts/resolve/main/malicious-contracts.csv"
r = requests.get(url, stream=True, timeout=60)
lines = io.TextIOWrapper(r.raw, encoding="utf-8-sig", newline="")
rd = csv.reader(lines)
hdr = next(rd); print("header", hdr)
n = mal = 0; uniq = {}; inforta = 0
for row in rd:
    n += 1
    b, m = row[0], row[-1].strip().lower() in ("true", "1")
    mal += m
    h = hashlib.sha1(b.encode()).hexdigest()
    uniq[h] = uniq.get(h, False) or m
    inforta += h in fh
    if n % 20000 == 0: print("rows", n, "malicious", mal, flush=True)
print("TOTAL rows", n, "malicious", mal, "unique bytecodes", len(uniq), "unique malicious", sum(uniq.values()), "overlap with forta", inforta)
