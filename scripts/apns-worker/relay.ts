// Pure parts of the relay (no Cloudflare APIs), so they can be tested with `node --test`.

export type Category = "approval" | "finished" | "waiting" | "alert";
export type ApnsEnv = "production" | "sandbox";

export interface PushRequest {
  token: string;
  category: Category;
  env: ApnsEnv;
}

export const MAX_BODY_BYTES = 512;
export const HOURLY_LIMIT = 20;

const CATEGORIES: readonly Category[] = ["approval", "finished", "waiting", "alert"];

/** Constant strings only: nothing about the work on the Mac may reach Apple. */
const TEXT: Record<Category, string> = {
  approval: "An agent is waiting for your approval",
  finished: "An agent has finished",
  waiting: "An agent is waiting for you",
  alert: "Test notification",
};

export function parseRequest(raw: string): PushRequest | string {
  if (new TextEncoder().encode(raw).length > MAX_BODY_BYTES) return "body too large";
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return "bad json";
  }
  if (typeof v !== "object" || v === null || Array.isArray(v)) return "bad body";
  const o = v as Record<string, unknown>;
  const token = o.token;
  if (typeof token !== "string" || !/^[0-9a-fA-F]{64}$/.test(token)) return "bad token";
  const category = o.category;
  if (typeof category !== "string" || !CATEGORIES.includes(category as Category)) return "bad category";
  const env = o.env;
  if (env !== "production" && env !== "sandbox") return "bad env";
  return { token: token.toLowerCase(), category: category as Category, env };
}

export function apnsHost(env: ApnsEnv): string {
  return env === "sandbox" ? "api.sandbox.push.apple.com" : "api.push.apple.com";
}

export function payloadFor(category: Category): object {
  const aps: Record<string, unknown> = {
    alert: { title: "SSH App", body: TEXT[category] },
    sound: "default",
    "thread-id": category,
  };
  if (category === "approval") aps["interruption-level"] = "time-sensitive";
  return { aps, category };
}

export function headersFor(category: Category, bundleId: string, jwt: string, nowSec: number): Record<string, string> {
  const ttl = category === "approval" ? 120 : 3600;
  return {
    authorization: `bearer ${jwt}`,
    "apns-topic": bundleId,
    "apns-push-type": "alert",
    "apns-priority": "10",
    "apns-expiration": String(nowSec + ttl),
    "apns-collapse-id": category,
    "content-type": "application/json",
  };
}

/** Constant-time comparison of two strings of any length. */
export function safeEqual(a: string, b: string): boolean {
  const x = new TextEncoder().encode(a);
  const y = new TextEncoder().encode(b);
  let diff = x.length ^ y.length;
  const n = Math.max(x.length, y.length);
  for (let i = 0; i < n; i++) diff |= (x[i] ?? 0) ^ (y[i] ?? 0);
  return diff === 0;
}

/** Key and next count for the hourly counter of one token. */
export function rateSlot(token: string, nowSec: number): string {
  return `r:${token}:${Math.floor(nowSec / 3600)}`;
}

export function allowed(count: number): boolean {
  return count < HOURLY_LIMIT;
}

export function base64Url(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_");
}
