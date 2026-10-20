/**
 * Cloudflare Worker: zero-knowledge APNs relay for ssh-app (feature 35).
 *
 * The request carries a device token, one of four fixed categories and the APNs environment.
 * Nothing about the work on the Mac (command, path, project, agent, message) can be sent: there is
 * no field for it, and the notification text is a constant per category.
 */

import {
  allowed,
  apnsHost,
  base64Url,
  headersFor,
  parseRequest,
  payloadFor,
  rateSlot,
  safeEqual,
} from "./relay.ts";

export interface Env {
  APPLE_KEY_ID: string;
  APPLE_TEAM_ID: string;
  APPLE_BUNDLE_ID: string;
  APPLE_P8_PRIVATE_KEY: string; // secret: PKCS#8 PEM
  RELAY_SECRET: string; // secret: bearer token shared with the phone and its hosts
  RATE: KVNamespace;
}

let cachedJwt: { token: string; at: number } | undefined;

const json = (status: number, body: object) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname !== "/push") return json(404, { error: "not found" });
    if (request.method !== "POST") return json(405, { error: "method" });
    if (!env.RELAY_SECRET || env.RELAY_SECRET.length < 32) return json(503, { error: "not configured" });

    const auth = request.headers.get("authorization") ?? "";
    if (!safeEqual(auth, `Bearer ${env.RELAY_SECRET}`)) return json(401, { error: "unauthorized" });

    const parsed = parseRequest(await request.text());
    if (typeof parsed === "string") return json(400, { error: parsed });

    const nowSec = Math.floor(Date.now() / 1000);
    const slot = rateSlot(parsed.token, nowSec);
    const count = Number((await env.RATE.get(slot)) ?? "0");
    if (!allowed(count)) return json(429, { error: "rate limited" });
    await env.RATE.put(slot, String(count + 1), { expirationTtl: 7200 });

    let jwt: string;
    try {
      jwt = await apnsJwt(env, nowSec);
    } catch {
      return json(503, { error: "not configured" });
    }
    const res = await fetch(`https://${apnsHost(parsed.env)}/3/device/${parsed.token}`, {
      method: "POST",
      headers: headersFor(parsed.category, env.APPLE_BUNDLE_ID, jwt, nowSec),
      body: JSON.stringify(payloadFor(parsed.category, parsed.sid)),
    });
    if (res.ok) return json(200, { ok: true });
    // Apple's reason is a short code ("BadDeviceToken"); no raw text is passed on.
    if (res.status === 410) return json(410, { error: "unregistered" });
    const reason = ((await res.json().catch(() => ({}))) as { reason?: string }).reason ?? "";
    if (res.status === 400 && reason === "BadDeviceToken") return json(410, { error: "unregistered" });
    // Only a plain code like "InvalidProviderToken" is passed on.
    return json(502, { error: "apns", status: res.status, reason: /^[A-Za-z]{1,40}$/.test(reason) ? reason : "" });
  },
};

async function apnsJwt(env: Env, nowSec: number): Promise<string> {
  if (cachedJwt && nowSec - cachedJwt.at < 50 * 60) return cachedJwt.token;
  const enc = new TextEncoder();
  const head = base64Url(enc.encode(JSON.stringify({ alg: "ES256", kid: env.APPLE_KEY_ID })));
  const claims = base64Url(enc.encode(JSON.stringify({ iss: env.APPLE_TEAM_ID, iat: nowSec })));
  const pem = env.APPLE_P8_PRIVATE_KEY.replace(/-----(BEGIN|END) PRIVATE KEY-----/g, "").replace(/\s+/g, "");
  const der = Uint8Array.from(atob(pem), (c) => c.charCodeAt(0));
  const key = await crypto.subtle.importKey("pkcs8", der, { name: "ECDSA", namedCurve: "P-256" }, false, ["sign"]);
  const sig = await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, key, enc.encode(`${head}.${claims}`));
  const token = `${head}.${claims}.${base64Url(new Uint8Array(sig))}`;
  cachedJwt = { token, at: nowSec };
  return token;
}
