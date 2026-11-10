// Small HTTP helpers: JSON responses, typed errors, CORS.

export class HttpError extends Error {
  readonly status: number;
  readonly code: string;
  readonly headers: Record<string, string>;
  constructor(status: number, code: string, message: string, headers: Record<string, string> = {}) {
    super(message);
    this.status = status;
    this.code = code;
    this.headers = headers;
  }
}

const SECURITY_HEADERS: Record<string, string> = {
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "no-referrer",
};

export function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
      ...SECURITY_HEADERS,
      ...headers,
    },
  });
}

export function errorResponse(err: HttpError): Response {
  return json({ ok: false, error: err.code, message: err.message }, err.status, err.headers);
}

export function parseOrigins(list: string | undefined): string[] {
  if (!list) return [];
  return list
    .split(",")
    .map((s) => s.trim().replace(/\/+$/, ""))
    .filter((s) => s.length > 0);
}

/** CORS headers for an allowed cross-origin caller; empty object if the origin is not allowed. */
export function corsHeaders(request: Request, allowList: string | undefined): Record<string, string> {
  const origin = request.headers.get("Origin");
  if (!origin) return {};
  if (!parseOrigins(allowList).includes(origin)) return { Vary: "Origin" };
  return {
    "Access-Control-Allow-Origin": origin,
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type",
    "Access-Control-Max-Age": "86400",
    Vary: "Origin",
  };
}

export function withHeaders(res: Response, headers: Record<string, string>): Response {
  if (Object.keys(headers).length === 0) return res;
  const out = new Response(res.body, res);
  for (const [k, v] of Object.entries(headers)) out.headers.set(k, v);
  return out;
}
