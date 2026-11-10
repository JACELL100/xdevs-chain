// GET / — single-page faucet form. Inline CSS/JS, no frameworks.

import { TURNSTILE_ACTION } from "./turnstile.ts";

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

function randomNonce(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes));
}

export function renderPage(opts: { siteKey: string; dripAmount: string }): Response {
  const nonce = randomNonce();
  const siteKey = esc(opts.siteKey);
  const drip = esc(opts.dripAmount);
  const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>tXDS Faucet · xDEVS Chain</title>
<meta name="description" content="Get free tXDS testnet tokens for xDEVS Chain. Testnet tokens have no monetary value.">
<style>
:root{--bg:#f6f7f9;--card:#fff;--fg:#14171f;--muted:#5b6474;--border:#d7dbe2;--accent:#3b4cca;--accent-fg:#fff;--ok:#0f7a3d;--ok-bg:#e7f6ee;--err:#a1251b;--err-bg:#fbeceb;--warn-bg:#fff6dc;--warn-border:#e8c766}
@media (prefers-color-scheme:dark){:root{--bg:#0f1115;--card:#181b22;--fg:#e9ecf2;--muted:#a3abba;--border:#2c313c;--accent:#7f8cff;--accent-fg:#0f1115;--ok:#6fdc9c;--ok-bg:#12291d;--err:#ff9a90;--err-bg:#2e1614;--warn-bg:#2a2412;--warn-border:#6d5a1e}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
main{max-width:34rem;margin:0 auto;padding:2rem 1rem 3rem}
h1{font-size:1.6rem;margin:0 0 .25rem}
.sub{color:var(--muted);margin:0 0 1.25rem}
.notice{background:var(--warn-bg);border:1px solid var(--warn-border);border-radius:.5rem;padding:.75rem 1rem;margin-bottom:1.25rem}
.card{background:var(--card);border:1px solid var(--border);border-radius:.75rem;padding:1.25rem}
label{display:block;font-weight:600;margin-bottom:.35rem}
.hint{color:var(--muted);font-size:.875rem;margin:.35rem 0 1rem}
input[type=text]{width:100%;padding:.7rem .8rem;font:inherit;font-family:ui-monospace,SFMono-Regular,Consolas,monospace;font-size:.95rem;color:var(--fg);background:var(--bg);border:1px solid var(--border);border-radius:.5rem}
input[type=text]:focus-visible,button:focus-visible,a:focus-visible{outline:3px solid var(--accent);outline-offset:2px}
.cf-turnstile{margin:0 0 1rem;min-height:65px}
button{width:100%;padding:.8rem 1rem;font:inherit;font-weight:600;color:var(--accent-fg);background:var(--accent);border:0;border-radius:.5rem;cursor:pointer}
button[disabled]{opacity:.6;cursor:progress}
#result{margin-top:1rem;padding:.75rem 1rem;border-radius:.5rem;word-break:break-all}
#result:empty{display:none}
#result.ok{background:var(--ok-bg);color:var(--ok)}
#result.err{background:var(--err-bg);color:var(--err)}
#result a{color:inherit}
.status{color:var(--muted);font-size:.875rem;margin-top:1.25rem}
footer{color:var(--muted);font-size:.8rem;margin-top:2rem}
code{font-family:ui-monospace,SFMono-Regular,Consolas,monospace;font-size:.9em}
</style>
<script nonce="${nonce}" src="https://challenges.cloudflare.com/turnstile/v0/api.js" async defer></script>
</head>
<body>
<main>
  <h1>tXDS Faucet</h1>
  <p class="sub">Free test coins for <strong>xDEVS Chain</strong> testnet, by THE xDEVS.</p>

  <div class="notice" role="note">
    <strong>Testnet tokens have no value.</strong> tXDS are free, worthless by design, must never be sold,
    and will <em>not</em> convert to mainnet XDS. Anyone offering to buy or sell tXDS is running a scam.
  </div>

  <form id="claim" class="card" novalidate>
    <label for="address">Wallet address or <code>.xdevs</code> name</label>
    <input id="address" name="address" type="text" inputmode="text" autocomplete="off" autocapitalize="off"
      spellcheck="false" required placeholder="0x… or alice.xdevs" aria-describedby="address-hint">
    <p id="address-hint" class="hint">You can claim <strong>${drip} tXDS</strong> once every 24 hours per address and per network.
      <code>.xdevs</code> names will work once the name service is live.</p>
    <div class="cf-turnstile" data-sitekey="${siteKey}" data-action="${TURNSTILE_ACTION}" data-theme="auto"></div>
    <button id="submit" type="submit">Send me ${drip} tXDS</button>
    <div id="result" role="status" aria-live="polite"></div>
  </form>

  <p class="status" id="faucet-status" aria-live="polite"></p>
  <p class="status">Need more for a validator or dApp? Ask the team on Discord.</p>
  <footer>THE xDEVS · xDEVS Chain testnet faucet. Never share your private key or seed phrase; this faucet will never ask for it.</footer>
</main>
<script nonce="${nonce}">
(function () {
  var form = document.getElementById("claim");
  var input = document.getElementById("address");
  var btn = document.getElementById("submit");
  var out = document.getElementById("result");
  var statusEl = document.getElementById("faucet-status");

  function show(kind, text, link) {
    out.className = kind;
    out.textContent = text;
    if (link && /^https:\\/\\//.test(link.href)) {
      out.appendChild(document.createTextNode(" "));
      var a = document.createElement("a");
      a.href = link.href; a.textContent = link.text; a.target = "_blank"; a.rel = "noopener noreferrer";
      out.appendChild(a);
    }
  }

  function tokenValue() {
    var el = form.querySelector('[name="cf-turnstile-response"]');
    return el ? el.value : "";
  }

  form.addEventListener("submit", function (ev) {
    ev.preventDefault();
    var address = input.value.trim();
    if (!address) { show("err", "Please enter your wallet address."); input.focus(); return; }
    var token = tokenValue();
    if (!token) { show("err", "Please complete the captcha first."); return; }
    btn.disabled = true; show("", "Sending…");
    fetch("/api/claim", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ address: address, turnstileToken: token })
    }).then(function (r) { return r.json().catch(function () { return { ok: false, message: "Unexpected response (" + r.status + ")." }; }); })
      .then(function (data) {
        if (data && data.ok) {
          show("ok", "Sent " + data.amount + " " + data.symbol + ". Transaction: " + data.txHash,
            data.explorerUrl ? { href: data.explorerUrl, text: "View in explorer" } : null);
        } else {
          show("err", (data && data.message) || "Something went wrong. Please try again.");
        }
      })
      .catch(function () { show("err", "Network error. Please try again."); })
      .finally(function () {
        btn.disabled = false;
        if (window.turnstile) { try { window.turnstile.reset(); } catch (e) {} }
      });
  });

  fetch("/api/status").then(function (r) { return r.json(); }).then(function (s) {
    if (s && s.balance) statusEl.textContent = "Faucet balance: " + Number(s.balance).toLocaleString() + " " + s.symbol + " · chain id " + s.chainId;
  }).catch(function () {});
})();
</script>
</body>
</html>`;
  const csp = [
    "default-src 'none'",
    `script-src 'nonce-${nonce}' https://challenges.cloudflare.com`,
    "style-src 'unsafe-inline'",
    "frame-src https://challenges.cloudflare.com",
    "connect-src 'self'",
    "img-src 'self' data:",
    "base-uri 'none'",
    "form-action 'self'",
    "frame-ancestors 'none'",
  ].join("; ");
  return new Response(html, {
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
      "Content-Security-Policy": csp,
      "X-Content-Type-Options": "nosniff",
      "Referrer-Policy": "no-referrer",
    },
  });
}
