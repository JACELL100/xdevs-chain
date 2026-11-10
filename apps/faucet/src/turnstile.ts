// Server-side Cloudflare Turnstile verification.
// https://developers.cloudflare.com/turnstile/get-started/server-side-validation/

import { HttpError } from "./http.ts";

export const SITEVERIFY_URL = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
/** Must match data-action on the widget in page.ts. */
export const TURNSTILE_ACTION = "claim";

interface SiteverifyResult {
  success: boolean;
  "error-codes"?: string[];
  hostname?: string;
  action?: string;
}

export async function verifyTurnstile(opts: {
  token: unknown;
  secret: string;
  remoteIp: string;
  expectedHostname?: string;
}): Promise<void> {
  const { token, secret, remoteIp, expectedHostname } = opts;
  if (typeof token !== "string" || token.length === 0 || token.length > 2048) {
    throw new HttpError(403, "captcha_required", "Please complete the captcha.");
  }
  if (!secret) throw new HttpError(503, "misconfigured", "Faucet is temporarily unavailable.");

  const form = new FormData();
  form.append("secret", secret);
  form.append("response", token);
  form.append("remoteip", remoteIp);

  let result: SiteverifyResult;
  try {
    const res = await fetch(SITEVERIFY_URL, { method: "POST", body: form });
    if (!res.ok) throw new Error(`siteverify HTTP ${res.status}`);
    result = (await res.json()) as SiteverifyResult;
  } catch {
    throw new HttpError(503, "captcha_unavailable", "Captcha verification is unavailable. Please try again shortly.");
  }

  if (!result.success) {
    throw new HttpError(403, "captcha_failed", "Captcha verification failed. Please retry the captcha.");
  }
  // Test keys (and some widget configs) return an empty action; only a *different* action is a red flag
  // (a token minted for another form being replayed here).
  if (result.action && result.action !== TURNSTILE_ACTION) {
    throw new HttpError(403, "captcha_failed", "Captcha verification failed. Please retry the captcha.");
  }
  if (expectedHostname && result.hostname !== expectedHostname) {
    throw new HttpError(403, "captcha_failed", "Captcha verification failed. Please retry the captcha.");
  }
}
