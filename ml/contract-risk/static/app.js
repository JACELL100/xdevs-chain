import { predict, cleanHex } from './core.js';

const $ = (id) => document.getElementById(id);
let model;

async function load() {
  const [trees, config, lin, metrics] = await Promise.all([
    fetch('trees.json').then((r) => r.json()),
    fetch('config.json').then((r) => r.json()),
    fetch('linear.bin').then((r) => r.arrayBuffer()),
    fetch('metrics.json').then((r) => (r.ok ? r.json() : null)).catch(() => null),
  ]);
  model = { trees, coef: new Float32Array(lin), intercept: config.intercept, config };
  if (metrics) {
    const t = metrics.test.release;
    const pct = (x) => `${(x * 100).toFixed(x < 0.01 ? 3 : 0)}%`;
    $('stats').innerHTML = [
      [t.roc_auc.toFixed(3), 'ROC-AUC (held-out test)'],
      [pct(t.precision), 'precision when flagged'],
      [pct(t.fpr), 'false-positive rate'],
      [pct(t['recall_at_fpr_1pct']), 'recall at 1% FPR'],
    ].map(([b, s]) => `<div class="stat"><b>${b}</b><span>${s}</span></div>`).join('');
  }
}

async function getCode(addr, rpc) {
  const r = await fetch(rpc, { method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_getCode', params: [addr, 'latest'] }) });
  const j = await r.json();
  if (j.error) throw new Error(j.error.message || 'RPC error');
  return j.result;
}

const LEVELS = {
  high: ['High risk', 'var(--red)'],
  medium: ['Unusual: review carefully', 'var(--warn)'],
  low: ['No known malicious pattern', 'var(--teal)'],
};

async function run() {
  $('err').textContent = '';
  $('go').disabled = true;
  try {
    if (!model) await load();
    let code = $('code').value.trim();
    if (!code) {
      const a = $('addr').value.trim();
      if (!/^0x[0-9a-fA-F]{40}$/.test(a)) throw new Error('Enter a 0x… address (40 hex characters) or paste bytecode.');
      code = await getCode(a, $('net').value);
    }
    if (cleanHex(code).length < 4) {
      $('result').style.display = 'none';
      throw new Error('No contract code at this address: it is an ordinary account or empty.');
    }
    const res = predict(model, code);
    const [label, col] = LEVELS[res.risk_level];
    $('level').textContent = label;
    $('level').style.color = col;
    $('score').textContent = `score ${res.risk_score.toFixed(3)} · flagged at ≥ ${res.threshold.toFixed(3)}`;
    $('fill').style.width = `${Math.max(2, res.risk_score * 100)}%`;
    $('fill').style.background = col;
    const s = res.signals;
    const rows = [
      ['Code size', `${s.bytes.toLocaleString()} bytes`],
      ['Known functions', s.known_functions.join(', ') || 'none recognised'],
      ['Notable opcodes', Object.entries(s.notable_opcodes).map(([k, v]) => `${k}×${v}`).join(', ') || 'none'],
      ['ERC-20 interface', s.erc20_like ? 'yes' : 'no'],
    ];
    $('signals').innerHTML = rows.map(([k, v]) => `<dt>${k}</dt><dd></dd>`).join('');
    [...$('signals').querySelectorAll('dd')].forEach((dd, i) => { dd.textContent = rows[i][1]; });
    $('result').style.display = 'block';
  } catch (e) {
    $('err').textContent = e.message || String(e);
  } finally {
    $('go').disabled = false;
  }
}

$('go').addEventListener('click', run);
document.querySelectorAll('.examples button').forEach((b) => b.addEventListener('click', () => {
  $('addr').value = b.dataset.a; $('code').value = ''; $('net').value = 'https://ethereum-rpc.publicnode.com'; run();
}));
load().catch((e) => { $('err').textContent = `Could not load the model: ${e.message}`; });
