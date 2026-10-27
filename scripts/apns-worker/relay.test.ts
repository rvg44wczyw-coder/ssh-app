import test from "node:test";
import assert from "node:assert/strict";
import { allowed, headersFor, parseRequest, payloadFor, rateSlot, safeEqual } from "./relay.ts";

const T = "ab".repeat(32);

test("valid request is parsed and the token lower-cased", () => {
  const r = parseRequest(JSON.stringify({ token: T.toUpperCase(), category: "approval", env: "sandbox" }));
  assert.deepEqual(r, { token: T, category: "approval", env: "sandbox" });
});

test("bad input is refused", () => {
  const bad = (o: unknown) => typeof parseRequest(typeof o === "string" ? o : JSON.stringify(o)) === "string";
  assert.ok(bad("not json"));
  assert.ok(bad("[]"));
  assert.ok(bad({ token: "short", category: "approval", env: "sandbox" }));
  assert.ok(bad({ token: T, category: "rm -rf", env: "sandbox" }));
  assert.deepEqual(parseRequest(JSON.stringify({ token: T, category: "blocked", env: "sandbox" })), { token: T, category: "blocked", env: "sandbox" });
  assert.equal((payloadFor("blocked") as any).aps.alert.body, "A sandbox blocked a host");
  assert.ok(bad({ token: T, category: "approval", env: "prod" }));
  assert.ok(bad({ token: T, category: "approval", env: "sandbox", pad: "x".repeat(600) }));
  assert.ok(bad({ token: T + "g", category: "finished", env: "production" }));
});

test("extra fields are never forwarded", () => {
  const r = parseRequest(JSON.stringify({ token: T, category: "finished", env: "production", command: "rm -rf /" }));
  assert.ok(typeof r === "object");
  assert.ok(!JSON.stringify(r).includes("rm -rf"));
  assert.ok(!JSON.stringify(payloadFor("finished")).includes("rm -rf"));
});

test("payload text is constant per category and time-sensitive only for approvals", () => {
  const a = payloadFor("approval") as any;
  const f = payloadFor("finished") as any;
  assert.equal(a.aps["interruption-level"], "time-sensitive");
  assert.equal(f.aps["interruption-level"], undefined);
  assert.notEqual(a.aps.alert.body, f.aps.alert.body);
  assert.equal(a.aps.alert.title, "SSH App");
});

test("headers: approvals expire after 120 s, others after an hour, collapse per category", () => {
  const h = headersFor("approval", "com.x", "jwt", 1000);
  assert.equal(h["apns-expiration"], "1120");
  assert.equal(h["apns-collapse-id"], "approval");
  assert.equal(headersFor("waiting", "com.x", "jwt", 1000)["apns-expiration"], "4600");
});

test("safeEqual", () => {
  assert.ok(safeEqual("Bearer a", "Bearer a"));
  assert.ok(!safeEqual("Bearer a", "Bearer b"));
  assert.ok(!safeEqual("a", "ab"));
  assert.ok(!safeEqual("", "x"));
});

test("rate limit is 20 per token per hour", () => {
  assert.ok(allowed(19));
  assert.ok(!allowed(20));
  assert.equal(rateSlot(T, 3600), rateSlot(T, 7199));
  assert.notEqual(rateSlot(T, 3599), rateSlot(T, 3600));
});

test("sid is optional, eight hex digits, and only that reaches the payload", () => {
  const ok = parseRequest(JSON.stringify({ token: T, category: "finished", env: "sandbox", sid: "0a1b2c3d" })) as any;
  assert.equal(ok.sid, "0a1b2c3d");
  assert.deepEqual((payloadFor("finished", ok.sid) as any).sid, "0a1b2c3d");
  assert.equal((payloadFor("finished") as any).sid, undefined);
  assert.equal((payloadFor("finished", ok.sid) as any).aps["mutable-content"], 1);
  assert.equal((payloadFor("finished") as any).aps["mutable-content"], undefined);
  for (const sid of ["0A1B2C3D", "short", "0a1b2c3d0", 5, "../../x"]) {
    assert.equal(typeof parseRequest(JSON.stringify({ token: T, category: "finished", env: "sandbox", sid })), "string");
  }
});
