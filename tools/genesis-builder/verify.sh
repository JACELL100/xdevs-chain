#!/usr/bin/env bash
# Verify a genesis produced by the genesis builder against a live chain started from it.
#   bash tools/genesis-builder/verify.sh <genesis.json> [rpc-url|-] [totalSupplyTxds]
# Without rpc-url (or with "-") a fresh `anvil --init <genesis.json> --hardfork osaka` is started and stopped.
# With rpc-url (e.g. an `xdevs` node started from the same genesis) the checks run against that node.
set -euo pipefail

GENESIS=${1:?usage: verify.sh <genesis.json> [rpc-url|-] [totalSupplyTxds]}
RPC=${2:--}
SUPPLY=${3:-}
HERE=$(cd "$(dirname "$0")" && pwd)
export PATH="$HOME/.foundry/bin:$PATH"

ANVIL_PID=
cleanup() { if [ -n "$ANVIL_PID" ]; then kill "$ANVIL_PID" 2>/dev/null || true; fi; }
trap cleanup EXIT
if [ "$RPC" = "-" ]; then
  PORT=${VERIFY_PORT:-18645}
  anvil --init "$GENESIS" --hardfork osaka --port "$PORT" --silent &
  ANVIL_PID=$!
  RPC=http://127.0.0.1:$PORT
  for _ in $(seq 1 60); do cast chain-id --rpc-url "$RPC" >/dev/null 2>&1 && break; sleep 0.5; done
fi

FAIL=0
pass() { echo "ok   $*"; }
bad() { echo "FAIL $*"; FAIL=1; }
eq() { # eq <description> <got> <want>   (case-insensitive)
  if [ "$(echo "$2" | tr 'A-F' 'a-f')" = "$(echo "$3" | tr 'A-F' 'a-f')" ]; then pass "$1 = $3"; else bad "$1: got '$2', want '$3'"; fi
}
c() { cast call --block 0 --rpc-url "$RPC" "$@"; }
j() { node -e "const g=JSON.parse(require('fs').readFileSync(process.argv[1],'utf8')); $1" "$GENESIS"; }

echo "== $GENESIS @ $RPC"
# 1. generic: code / balance / nonce / every storage slot of every alloc account at block 0, chain id, total supply
node "$HERE/src/check-state.ts" "$GENESIS" "$RPC" $SUPPLY || FAIL=1

# 2. every predeploy and canonical helper has code
for a in 1000 1001 1002 1003 1004 1005 1006 1007 1008 1009; do
  code=$(cast code --block 0 --rpc-url "$RPC" 0x000000000000000000000000000000000000$a)
  if [ ${#code} -gt 2 ]; then pass "0x...$a has code ($(( (${#code} - 2) / 2 )) bytes)"; else bad "0x...$a has no code"; fi
done
for a in 0x4e59b44847b379578588920cA78FbF26c0B4956C 0x433709009B8330FDa32311DF1C2AFA402eD8D009 \
         0x0A630a99Df908A81115A3022927Be82f9299987e 0xcA11bde05977b3631167028862bE2a173976CA11; do
  code=$(cast code --block 0 --rpc-url "$RPC" $a)
  if [ ${#code} -gt 2 ]; then pass "$a has code ($(( (${#code} - 2) / 2 )) bytes)"; else bad "$a has no code"; fi
done

CHAIN=$(cast chain-id --rpc-url "$RPC")
STK=0x0000000000000000000000000000000000001000
GOV=0x0000000000000000000000000000000000001001
TRE=0x0000000000000000000000000000000000001002
FEE=0x0000000000000000000000000000000000001003
XNS=0x0000000000000000000000000000000000001004
EP=0x433709009B8330FDa32311DF1C2AFA402eD8D009
MC3=0xcA11bde05977b3631167028862bE2a173976CA11

# 3. Staking: active set == validators recorded in the genesis
WANT_OPS=$(j "const v=g.validators||[]; if(v.length&&v[0].operator) console.log('['+v.map(x=>x.operator).join(', ')+']')")
OPS=$(c $STK "activeValidators()(address[],uint256[])" | head -1)
if [ -n "$WANT_OPS" ]; then eq "Staking.activeValidators() operators" "$OPS" "$WANT_OPS"; else pass "Staking.activeValidators() operators: $OPS (genesis has no operator list)"; fi
WANT_KEYS=$(j "const v=g.validators||[]; if(v.length) console.log(v.map(x=>x.consensusKey.replace('ed25519:','').toLowerCase()).sort().join(','))")
KEYS=$(c $STK "consensusValidatorSet()(address[],bytes32[],uint256[])" | sed -n 2p | tr -d '[] ' | tr ',' '\n' | tr 'A-F' 'a-f' | sort | paste -sd, -)
if [ -n "$WANT_KEYS" ]; then eq "Staking consensus keys (sorted) == genesis validators[] keys" "$KEYS" "$WANT_KEYS"; else pass "Staking consensus keys: $KEYS (validators[] not emitted: dev chain)"; fi
WANT_POW=$(j "const v=g.validators||[]; if(v.length&&v[0].stake) console.log(v.every(x=>BigInt(x.stake)/10n**18n===BigInt(x.power)))")
if [ -n "$WANT_POW" ]; then eq "genesis power == Staking stake / 1e18 for every validator" "$WANT_POW" "true"; fi
STAKE_SUM=$(c $STK "activeValidators()(address[],uint256[])" --json | node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>console.log(JSON.parse(s)[1].reduce((a,b)=>a+BigInt(b),0n).toString()))")
eq "sum of Staking powers == Staking balance" "$STAKE_SUM" "$(cast balance --block 0 --rpc-url "$RPC" $STK)"
eq "Staking.currentEpoch()" "$(c $STK 'currentEpoch()(uint64)')" "0"
eq "Staking.governance()" "$(c $STK 'governance()(address)')" "$GOV"
# slot 3 = currentEpoch | setVersion | lastSetChangeBlock | lastEpochBlock (u64 each, low to high):
# the two block numbers (high 16 bytes) must be 0, i.e. initialize ran at block 0
eq "Staking slot 3 lastSetChangeBlock/lastEpochBlock" "$(cast storage --block 0 --rpc-url "$RPC" $STK 3 | cut -c3-34)" "00000000000000000000000000000000"

# 4. Governance / Treasury
eq "Governance.staking()" "$(c $GOV 'staking()(address)')" "$STK"
eq "Treasury.governance()" "$(c $TRE 'governance()(address)')" "$GOV"

# 5. FeeConfig raw slots (docs/specs/predeploys-system.md section 4)
WANT_GAS=$(j "console.log(BigInt(g.gasLimit).toString())")
WANT_MIN=$(j "console.log(BigInt(g.config.baseFee.minimum).toString())")
names=(governance blockGasLimit minBaseFee burnBps baseFeeMaxChangeDenominator elasticityMultiplier maxTxSizeBytes localFeeEnabled localFeeThresholdBps localFeeMaxChangeDenominator version)
want=($((16#1001)) "$WANT_GAS" "$WANT_MIN" 8000 8 2 131072 0 2500 8 1)
for i in $(seq 0 10); do
  eq "FeeConfig slot $i (${names[$i]})" "$(cast to-dec "$(cast storage --block 0 --rpc-url "$RPC" $FEE $i)")" "${want[$i]}"
done

# 6. XNSRegistry initialized via SYSTEM_ADDRESS
eq "XNSRegistry.governance()" "$(c $XNS 'governance()(address)')" "$GOV"
eq "XNSRegistry.treasury()" "$(c $XNS 'treasury()(address)')" "$TRE"
eq "XNSRegistry.available(\"xdevs\") (reserved)" "$(c $XNS 'available(string)(bool)' xdevs)" "false"
eq "XNSRegistry.reserved(keccak(\"xdevs\"))" "$(c $XNS 'reserved(bytes32)(bool)' "$(cast keccak xdevs)")" "true"
eq "XNSRegistry.reserved(keccak(\"faucet\"))" "$(c $XNS 'reserved(bytes32)(bool)' "$(cast keccak faucet)")" "true"
eq "XNSRegistry.reserved(keccak(\"alice\"))" "$(c $XNS 'reserved(bytes32)(bool)' "$(cast keccak alice)")" "false"

# 7. EntryPoint v0.9 at its canonical address, immutables for THIS chain id
eq "EntryPoint.supportsInterface(IERC165)" "$(c $EP 'supportsInterface(bytes4)(bool)' 0x01ffc9a7)" "true"
eq "EntryPoint.getNonce(0x...a11ce0, 0)" "$(c $EP 'getNonce(address,uint192)(uint256)' 0x0000000000000000000000000000000000a11ce0 0)" "0"
eq "EntryPoint.senderCreator()" "$(c $EP 'senderCreator()(address)')" "0x0A630a99Df908A81115A3022927Be82f9299987e"
DOMAIN=$(cast keccak "$(cast abi-encode 'f(bytes32,bytes32,bytes32,uint256,address)' \
  "$(cast keccak 'EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)')" \
  "$(cast keccak ERC4337)" "$(cast keccak 1)" "$CHAIN" $EP)")
eq "EntryPoint.getDomainSeparatorV4() (chain $CHAIN)" "$(c $EP 'getDomainSeparatorV4()(bytes32)')" "$DOMAIN"
eq "EntryPoint.balanceOf(Paymaster 0x...1007)" "$(c $EP 'balanceOf(address)(uint256)' 0x0000000000000000000000000000000000001007)" "0"

# 8. Multicall3 / CREATE2 deployer / WXDS
eq "Multicall3.getChainId()" "$(c $MC3 'getChainId()(uint256)' | awk '{print $1}')" "$CHAIN"
AGG=$(c $MC3 'aggregate3((address,bool,bytes)[])((bool,bytes)[])' \
  "[($TRE,false,$(cast calldata 'governance()')),($FEE,false,$(cast calldata 'version()'))]")
eq "Multicall3.aggregate3([Treasury.governance(), FeeConfig.version()])" "$AGG" \
  "[(true, 0x0000000000000000000000000000000000000000000000000000000000001001), (true, 0x0000000000000000000000000000000000000000000000000000000000000001)]"
INIT=0x600a600c600039600a6000f3602a60005260206000f3
SALT=0x0000000000000000000000000000000000000000000000000000000000000001
GOT=$(cast call --block 0 --rpc-url "$RPC" 0x4e59b44847b379578588920cA78FbF26c0B4956C "${SALT}${INIT#0x}")
WANT=$(cast create2 --deployer 0x4e59b44847b379578588920cA78FbF26c0B4956C --salt $SALT --init-code $INIT | cut -f1)
eq "CREATE2 deployer (eth_call) returns the CREATE2 address" "0x${GOT: -40}" "$WANT"
eq "WXDS.symbol()" "$(c 0x0000000000000000000000000000000000001009 'symbol()(string)')" '"WXDS"'
eq "WXDS.totalSupply()" "$(c 0x0000000000000000000000000000000000001009 'totalSupply()(uint256)')" "0"

if [ "$FAIL" = 0 ]; then echo "ALL CHECKS PASSED"; else echo "SOME CHECKS FAILED"; exit 1; fi
