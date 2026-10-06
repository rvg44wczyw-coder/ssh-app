/**
 * Cloudflare Worker: Zero-Knowledge APNs Wake-up Gateway for ssh-app
 *
 * PRIVACY GUARANTEE:
 * - This worker accepts ONLY an opaque device token (and optional notification type).
 * - NO command names, NO project paths, and NO agent details are ever sent to or processed by this worker.
 * - Cost: $0/month on Cloudflare Workers Free Tier (100,000 requests/day included).
 */

export interface Env {
  APPLE_KEY_ID: string;      // e.g. "ABC123XYZ"
  APPLE_TEAM_ID: string;     // e.g. "DEF456UVW"
  APPLE_BUNDLE_ID: string;   // e.g. "com.semenov.sshapp"
  APPLE_P8_PRIVATE_KEY: string; // OpenSSL PKCS#8 private key in PEM format
  AUTH_SECRET?: string;      // Optional shared secret between your Mac and Worker
}

interface WakeUpRequest {
  device_token: string;
  notification_type?: "alert" | "liveactivity";
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method !== "POST") {
      return new Response("Method Not Allowed", { status: 405 });
    }

    // Optional authentication check from your Mac
    if (env.AUTH_SECRET) {
      const authHeader = request.headers.get("Authorization");
      if (authHeader !== `Bearer ${env.AUTH_SECRET}`) {
        return new Response("Unauthorized", { status: 401 });
      }
    }

    try {
      const body = (await request.json()) as WakeUpRequest;
      const deviceToken = body.device_token?.trim();

      // Validate device token format (hex string 64 chars)
      if (!deviceToken || !/^[a-fA-F0-9]{64}$/.test(deviceToken)) {
        return new Response(
          JSON.stringify({ error: "Invalid device_token hex format" }),
          { status: 400, headers: { "Content-Type": "application/json" } }
        );
      }

      // Generate Apple APNs JWT (valid for 1 hour)
      const jwtToken = await generateApnsJwt(env);

      // Determine APNs endpoint and headers
      const isLiveActivity = body.notification_type === "liveactivity";
      const apnsTopic = isLiveActivity
        ? `${env.APPLE_BUNDLE_ID}.push-type.liveactivity`
        : env.APPLE_BUNDLE_ID;
      const apnsPushType = isLiveActivity ? "liveactivity" : "alert";

      // Dispatch to production APNs over HTTP/2
      const apnsUrl = `https://api.push.apple.com/3/device/${deviceToken}`;
      const apnsPayload = {
        aps: {
          "content-available": 1,
          alert: {
            title: "Agent Approval",
            body: "An agent requires human approval",
          },
          sound: "default",
          category: "AGENT_APPROVAL",
        },
      };

      const apnsResponse = await fetch(apnsUrl, {
        method: "POST",
        headers: {
          authorization: `bearer ${jwtToken}`,
          "apns-topic": apnsTopic,
          "apns-push-type": apnsPushType,
          "apns-priority": "10",
          "apns-expiration": "0",
          "content-type": "application/json",
        },
        body: JSON.stringify(apnsPayload),
      });

      if (!apnsResponse.ok) {
        const errorText = await apnsResponse.text();
        return new Response(
          JSON.stringify({ error: "APNs error", status: apnsResponse.status, detail: errorText }),
          { status: 502, headers: { "Content-Type": "application/json" } }
        );
      }

      return new Response(
        JSON.stringify({ success: true, message: "Wake-up ping dispatched" }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      );
    } catch (err: any) {
      return new Response(
        JSON.stringify({ error: "Internal Server Error", detail: err?.message || String(err) }),
        { status: 500, headers: { "Content-Type": "application/json" } }
      );
    }
  },
};

/**
 * Generates an ES256-signed JWT for Apple APNs.
 */
async function generateApnsJwt(env: Env): Promise<string> {
  const header = {
    alg: "ES256",
    kid: env.APPLE_KEY_ID,
  };

  const nowSec = Math.floor(Date.now() / 1000);
  const claims = {
    iss: env.APPLE_TEAM_ID,
    iat: nowSec,
  };

  const headerB64 = base64UrlEncode(JSON.stringify(header));
  const claimsB64 = base64UrlEncode(JSON.stringify(claims));
  const unsignedToken = `${headerB64}.${claimsB64}`;

  // Import PKCS#8 private key
  const pem = env.APPLE_P8_PRIVATE_KEY
    .replace(/-----BEGIN PRIVATE KEY-----/, "")
    .replace(/-----END PRIVATE KEY-----/, "")
    .replace(/\s+/g, "");
  const binaryDer = Uint8Array.from(atob(pem), (c) => c.charCodeAt(0));

  const cryptoKey = await crypto.subtle.importKey(
    "pkcs8",
    binaryDer.buffer,
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["sign"]
  );

  const signature = await crypto.subtle.sign(
    { name: "ECDSA", hash: { name: "SHA-256" } },
    cryptoKey,
    new TextEncoder().encode(unsignedToken)
  );

  const signatureB64 = base64UrlEncode(new Uint8Array(signature));
  return `${unsignedToken}.${signatureB64}`;
}

function base64UrlEncode(input: string | Uint8Array): string {
  let base64 = "";
  if (typeof input === "string") {
    base64 = btoa(input);
  } else {
    base64 = btoa(String.fromCharCode(...input));
  }
  return base64.replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_");
}
