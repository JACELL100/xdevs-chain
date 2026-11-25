"""Print label balance and basic stats of the candidate training datasets (run in CI)."""
import pyarrow.parquet as pq
from huggingface_hub import hf_hub_download, list_repo_files

for repo in ["forta/malicious-smart-contract-dataset", "recare/malicious-smart-contract-dataset", "flipguard/malicious-contracts"]:
    try:
        files = [f for f in list_repo_files(repo, repo_type="dataset") if f.endswith((".parquet", ".csv"))]
        print(f"== {repo}: {files[:6]}")
        for f in files[:2]:
            p = hf_hub_download(repo, f, repo_type="dataset")
            if not f.endswith(".parquet"):
                print("  (csv)", f); continue
            pf = pq.ParquetFile(p)
            print("  rows", pf.metadata.num_rows, "cols", pf.schema_arrow.names)
            for col in ("malicious", "label", "is_malicious"):
                if col in pf.schema_arrow.names:
                    t = pf.read(columns=[col]).column(0).to_pandas()
                    print("  ", col, t.value_counts(dropna=False).to_dict())
    except Exception as e:  # keep probing the others
        print("  error", repr(e)[:200])
