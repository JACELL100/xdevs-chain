import type { Snippet } from '@/components/CodeTabs'

/** Code samples, taken from the docs (guide/quickstart, developers/deploy-contracts, developers/sdk, developers/json-rpc). */
export const SNIPPETS: Snippet[] = [
  {
    id: 'foundry',
    label: 'Foundry',
    note: 'Unchanged Foundry. Get tXDS from the faucet first.',
    code: `# Enclave looks like any EVM chain to your tools
export RPC=https://rpc.xdevschain.tech
cast chain-id --rpc-url $RPC        # 717171
cast block-number --rpc-url $RPC    # grows every 2 s

# Deploy a contract
forge init counter && cd counter
forge create src/Counter.sol:Counter --rpc-url $RPC --chain 717171 --private-key $PK --broadcast
cast send $COUNTER "increment()" --rpc-url $RPC --chain 717171 --private-key $PK`,
  },
  {
    id: 'viem',
    label: 'viem + SDK',
    note: '@xdevs/sdk is built from the repo; it is not on npm yet.',
    code: `import { createPublicClient, http } from 'viem'
import { xdevsActions, xdevsEnclave, formatXds } from '@xdevs/sdk'

// Enclave, the public testnet: chain id 717171
const client = createPublicClient({ chain: xdevsEnclave, transport: http() }).extend(xdevsActions)

const balance = await client.getBalance({ address: '0x...' })
console.log(\`\${formatXds(balance)} tXDS\`)`,
  },
  {
    id: 'simulate',
    label: 'Safety RPC',
    note: 'Simulation commits nothing. Warnings are advice, not fact.',
    code: `# What would this transaction do? Ask the node before signing.
curl -s https://rpc.xdevschain.tech -H 'content-type: application/json' -d '{
  "jsonrpc": "2.0", "id": 1, "method": "xdevs_simulateTransaction",
  "params": [{ "from": "0xf39F...2266", "to": "0x...1009", "data": "0x095ea7b3..." }, "latest"]
}'
# => asset changes, approvals it sets, Guardian Vault outcome, warnings

# Is this recipient new to me, or a look-alike of someone I paid?
#   xdevs_checkRecipient(from, to)`,
  },
  {
    id: 'metamask',
    label: 'MetaMask',
    note: 'Or press “Add Enclave to MetaMask” on the Enclave page.',
    code: `Network name     xDEVS Chain Enclave
RPC URL          https://rpc.xdevschain.tech
Chain ID         717171
Currency symbol  tXDS
Block explorer   https://explorer.xdevschain.tech`,
  },
]
