"""Retry creating Oracle Cloud Always-Free Ampere A1 VMs until capacity is available.

Oracle's free A1 shape is often "out of host capacity". This script asks for the missing
instances (one request per interval to avoid rate limiting) and stops when they all exist.
It never creates duplicates: an existing, non-terminated instance with the same display
name counts as done. Used by .github/workflows/oci-capacity.yml; also runnable locally.

Configuration (environment variables):
  OCI_CONFIG_FILE   OCI config file (default ~/.oci/config)
  OCI_NAMES         comma-separated instance names (default "xdevs-val-3,xdevs-val-4")
  OCI_OCPUS         OCPUs per instance (default 1; Always Free A1 is 2 OCPU / 12 GB in total)
  OCI_MEMORY_GB     memory per instance in GB (default 6)
  OCI_VCN_NAME      VCN whose public subnet is used (default "xdevs-vcn")
  OCI_SSH_PUBKEY    SSH public key (default: contents of ~/.ssh/xdevs_oracle.pub)
  OCI_MAX_MINUTES   stop after this many minutes (default 0 = no limit)
  OCI_INTERVAL_SECS seconds between launch requests (default 60)

Exit codes: 0 = all instances exist, 3 = still no capacity when time ran out,
2 = limit/quota or configuration error (needs a human).
"""

import os
import random
import sys
import time
from pathlib import Path

import oci

SHAPE = "VM.Standard.A1.Flex"
CAPACITY_HINTS = ("out of host capacity", "out of capacity")


def env(name: str, default: str) -> str:
    return os.environ.get(name) or default


def log(msg: str) -> None:
    print(time.strftime("%Y-%m-%d %H:%M:%S"), msg, flush=True)


def main() -> int:
    names = [n.strip() for n in env("OCI_NAMES", "xdevs-val-3,xdevs-val-4").split(",") if n.strip()]
    ocpus = float(env("OCI_OCPUS", "1"))
    memory = float(env("OCI_MEMORY_GB", "6"))
    interval = int(env("OCI_INTERVAL_SECS", "60"))
    max_minutes = float(env("OCI_MAX_MINUTES", "0"))
    ssh_key = os.environ.get("OCI_SSH_PUBKEY") or (Path.home() / ".ssh" / "xdevs_oracle.pub").read_text()
    ssh_key = ssh_key.strip()

    cfg = oci.config.from_file(env("OCI_CONFIG_FILE", oci.config.DEFAULT_LOCATION))
    tenancy = cfg["tenancy"]
    identity = oci.identity.IdentityClient(cfg)
    compute = oci.core.ComputeClient(cfg)
    network = oci.core.VirtualNetworkClient(cfg)

    ad = identity.list_availability_domains(tenancy).data[0].name
    image = compute.list_images(
        tenancy, operating_system="Canonical Ubuntu", operating_system_version="24.04",
        shape=SHAPE, sort_by="TIMECREATED", sort_order="DESC").data[0]
    vcn_name = env("OCI_VCN_NAME", "xdevs-vcn")
    vcns = [v for v in network.list_vcns(tenancy).data if v.display_name == vcn_name]
    if not vcns:
        log(f"STOP: VCN {vcn_name!r} not found")
        return 2
    subnet = next(s for s in network.list_subnets(tenancy, vcn_id=vcns[0].id).data
                  if not s.prohibit_public_ip_on_vnic)
    log(f"config: image={image.display_name} subnet={subnet.display_name} "
        f"shape={SHAPE} {ocpus:g} OCPU / {memory:g} GB, names={names}")

    def existing() -> dict:
        return {i.display_name: i for i in compute.list_instances(tenancy).data
                if i.lifecycle_state not in ("TERMINATED", "TERMINATING")}

    deadline = time.time() + max_minutes * 60 if max_minutes > 0 else float("inf")
    attempt = 0
    while True:
        missing = [n for n in names if n not in existing()]
        if not missing:
            log(f"DONE all instances exist: {names}")
            return 0
        if time.time() >= deadline:
            log(f"NO_CAPACITY yet after {attempt} attempts; missing {missing}")
            return 3
        attempt += 1
        name = missing[0]
        details = oci.core.models.LaunchInstanceDetails(
            availability_domain=ad,
            compartment_id=tenancy,
            display_name=name,
            shape=SHAPE,
            shape_config=oci.core.models.LaunchInstanceShapeConfigDetails(
                ocpus=ocpus, memory_in_gbs=memory),
            source_details=oci.core.models.InstanceSourceViaImageDetails(image_id=image.id),
            create_vnic_details=oci.core.models.CreateVnicDetails(
                subnet_id=subnet.id, assign_public_ip=True),
            metadata={"ssh_authorized_keys": ssh_key},
        )
        try:
            inst = compute.launch_instance(details).data
            log(f"CREATED {name} ({inst.id[-12:]}) on attempt {attempt}")
        except oci.exceptions.ServiceError as e:
            msg = (e.message or "").lower()
            if any(h in msg for h in CAPACITY_HINTS):
                log(f"ATTEMPT {attempt}: {name}: out of capacity")
            elif e.status == 429:
                log(f"ATTEMPT {attempt}: {name}: rate limited, backing off")
                time.sleep(120)
            elif "limit" in msg or e.code in ("LimitExceeded", "QuotaExceeded"):
                log(f"STOP {name}: limit/quota error: {e.code}: {e.message}")
                return 2
            else:
                log(f"ATTEMPT {attempt}: {name}: {e.status} {e.code}: {e.message}")
        time.sleep(interval + random.randint(0, 20))


if __name__ == "__main__":
    sys.exit(main())
