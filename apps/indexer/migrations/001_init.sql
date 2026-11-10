-- xDEVS explorer indexer schema v1.
-- Conventions: addresses and hashes are lowercase 0x hex text; big integers are NUMERIC(78,0); block numbers and
-- Unix timestamps are BIGINT. One statement per ';' line end (db.ts splits on it).

CREATE TABLE indexer_cursor (
  id text PRIMARY KEY,
  block_number bigint NOT NULL,
  block_hash text NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE chain_info (
  id int PRIMARY KEY DEFAULT 1,
  chain_id bigint NOT NULL,
  genesis_hash text,
  burn_bps int NOT NULL DEFAULT 10000
);

CREATE TABLE blocks (
  number bigint PRIMARY KEY,
  hash text NOT NULL UNIQUE,
  parent_hash text NOT NULL,
  timestamp bigint NOT NULL,
  miner text NOT NULL,
  gas_used numeric(78,0) NOT NULL,
  gas_limit numeric(78,0) NOT NULL,
  base_fee numeric(78,0),
  tx_count int NOT NULL,
  size int,
  burned_fees numeric(78,0) NOT NULL DEFAULT 0,
  treasury_fees numeric(78,0) NOT NULL DEFAULT 0,
  priority_fees numeric(78,0) NOT NULL DEFAULT 0,
  burn_bps int NOT NULL DEFAULT 10000
);

CREATE TABLE transactions (
  hash text PRIMARY KEY,
  block_number bigint NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
  tx_index int NOT NULL,
  timestamp bigint NOT NULL,
  from_address text NOT NULL,
  to_address text,
  contract_address text,
  value numeric(78,0) NOT NULL,
  nonce bigint NOT NULL,
  type smallint NOT NULL,
  gas numeric(78,0) NOT NULL,
  gas_price numeric(78,0),
  max_fee_per_gas numeric(78,0),
  max_priority_fee_per_gas numeric(78,0),
  effective_gas_price numeric(78,0) NOT NULL,
  gas_used numeric(78,0) NOT NULL,
  status smallint NOT NULL,
  input text NOT NULL,
  method_id text,
  fee numeric(78,0) NOT NULL,
  base_fee_part numeric(78,0) NOT NULL,
  burned numeric(78,0) NOT NULL,
  treasury numeric(78,0) NOT NULL,
  priority_fee numeric(78,0) NOT NULL,
  authorization_list jsonb,
  spam boolean NOT NULL DEFAULT false,
  spam_reasons jsonb NOT NULL DEFAULT '[]'
);
CREATE INDEX transactions_block_idx ON transactions (block_number DESC, tx_index DESC);
CREATE INDEX transactions_from_idx ON transactions (from_address, block_number DESC, tx_index DESC);
CREATE INDEX transactions_to_idx ON transactions (to_address, block_number DESC, tx_index DESC);
CREATE INDEX transactions_created_idx ON transactions (contract_address) WHERE contract_address IS NOT NULL;

CREATE TABLE logs (
  block_number bigint NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
  log_index int NOT NULL,
  tx_hash text NOT NULL,
  tx_index int NOT NULL,
  address text NOT NULL,
  topic0 text,
  topic1 text,
  topic2 text,
  topic3 text,
  data text NOT NULL,
  PRIMARY KEY (block_number, log_index)
);
CREATE INDEX logs_tx_idx ON logs (tx_hash);
CREATE INDEX logs_address_idx ON logs (address, block_number DESC);

CREATE TABLE tokens (
  address text PRIMARY KEY,
  name text,
  symbol text,
  decimals int,
  is_xdrc20 boolean NOT NULL DEFAULT false,
  is_canonical boolean NOT NULL DEFAULT false,
  first_seen_block bigint NOT NULL
);

-- seq = log_index for token rows, -(tx_index + 1) for the native value of a transaction.
CREATE TABLE token_transfers (
  block_number bigint NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
  seq int NOT NULL,
  tx_hash text NOT NULL,
  log_index int,
  timestamp bigint NOT NULL,
  tx_from text NOT NULL,
  kind text NOT NULL,
  token text,
  from_address text NOT NULL,
  to_address text NOT NULL,
  value numeric(78,0) NOT NULL,
  spam boolean NOT NULL DEFAULT false,
  spam_reasons jsonb NOT NULL DEFAULT '[]',
  look_alike_of text,
  PRIMARY KEY (block_number, seq)
);
CREATE INDEX token_transfers_from_idx ON token_transfers (from_address, block_number DESC, seq DESC);
CREATE INDEX token_transfers_to_idx ON token_transfers (to_address, block_number DESC, seq DESC);
CREATE INDEX token_transfers_tx_idx ON token_transfers (tx_hash);

CREATE TABLE addresses (
  address text PRIMARY KEY,
  first_seen_block bigint NOT NULL,
  last_seen_block bigint NOT NULL,
  is_contract boolean NOT NULL DEFAULT false,
  tx_count int NOT NULL DEFAULT 0
);

-- Non-spam interaction history used by the look-alike check (both directions).
CREATE TABLE counterparties (
  owner text NOT NULL,
  counterparty text NOT NULL,
  first_block bigint NOT NULL,
  PRIMARY KEY (owner, counterparty)
);

-- scope: token (ERC-20 / XDRC-20 allowance on the token) or permit2 (Permit2Expiring allowance).
CREATE TABLE approvals (
  scope text NOT NULL,
  kind text NOT NULL,
  token text NOT NULL,
  owner text NOT NULL,
  spender text NOT NULL,
  amount numeric(78,0) NOT NULL,
  expiry bigint,
  revoked boolean NOT NULL DEFAULT false,
  block_number bigint NOT NULL,
  log_index int NOT NULL,
  tx_hash text NOT NULL,
  PRIMARY KEY (scope, token, owner, spender)
);
CREATE INDEX approvals_owner_idx ON approvals (owner, block_number DESC);

CREATE TABLE xns_names (
  labelhash text PRIMARY KEY,
  label text NOT NULL,
  name text NOT NULL,
  node text NOT NULL UNIQUE,
  owner text NOT NULL,
  addr text,
  expires bigint NOT NULL,
  registered_block bigint NOT NULL,
  registered_tx text NOT NULL,
  updated_block bigint NOT NULL
);
CREATE INDEX xns_names_owner_idx ON xns_names (owner);
CREATE INDEX xns_names_addr_idx ON xns_names (addr);
CREATE INDEX xns_names_label_idx ON xns_names (label text_pattern_ops);

CREATE TABLE xns_reverse (
  address text PRIMARY KEY,
  node text NOT NULL UNIQUE,
  name text,
  updated_block bigint NOT NULL
);

CREATE TABLE vault_actions (
  account text NOT NULL,
  action_id numeric(78,0) NOT NULL,
  target text NOT NULL,
  value numeric(78,0) NOT NULL,
  data text NOT NULL,
  execute_after bigint NOT NULL,
  status text NOT NULL,
  queued_block bigint NOT NULL,
  queued_tx text NOT NULL,
  cancelled_by text,
  final_block bigint,
  final_tx text,
  PRIMARY KEY (account, action_id)
);
CREATE INDEX vault_actions_account_idx ON vault_actions (account, queued_block DESC);

CREATE TABLE account_security (
  account text PRIMARY KEY,
  account_kind text NOT NULL DEFAULT 'unknown',
  vault_enabled boolean,
  frozen_until bigint,
  eoa_lock_state smallint,
  eoa_key_revoked boolean NOT NULL DEFAULT false,
  recovery_id text,
  recovery_approvals int,
  recovery_ready_at bigint,
  guardian_count int,
  guardian_threshold int,
  updated_block bigint NOT NULL
);

CREATE TABLE security_events (
  block_number bigint NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
  log_index int NOT NULL,
  tx_hash text NOT NULL,
  account text NOT NULL,
  event text NOT NULL,
  args jsonb NOT NULL,
  PRIMARY KEY (block_number, log_index)
);
CREATE INDEX security_events_account_idx ON security_events (account, block_number DESC);

CREATE TABLE labels (
  address text PRIMARY KEY,
  label text NOT NULL,
  source text NOT NULL DEFAULT 'manual'
);

CREATE TABLE alert_subscriptions (
  account text NOT NULL,
  channel text NOT NULL,
  target text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (account, channel, target)
);

CREATE TABLE alert_outbox (
  id bigserial PRIMARY KEY,
  dedupe_key text NOT NULL UNIQUE,
  event text NOT NULL,
  account text NOT NULL,
  destination text NOT NULL,
  payload jsonb NOT NULL,
  attempts int NOT NULL DEFAULT 0,
  next_attempt_at timestamptz NOT NULL DEFAULT now(),
  delivered_at timestamptz,
  failed boolean NOT NULL DEFAULT false,
  last_error text,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX alert_outbox_pending_idx ON alert_outbox (next_attempt_at) WHERE delivered_at IS NULL AND failed = false;

CREATE TABLE chain_stats (
  id int PRIMARY KEY DEFAULT 1,
  tx_count bigint NOT NULL DEFAULT 0,
  address_count bigint NOT NULL DEFAULT 0,
  transfer_count bigint NOT NULL DEFAULT 0,
  burned numeric(78,0) NOT NULL DEFAULT 0,
  treasury numeric(78,0) NOT NULL DEFAULT 0
);
INSERT INTO chain_stats (id) VALUES (1);
