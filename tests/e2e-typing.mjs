// End-to-end proof: typing from user A reaches user B through the relay, and a
// message A posts to the twin reaches B as a Google-shaped event.
// Needs: google-twin on :8081 and relay on :8080 (see docs/ARCHITECTURE.md "Run locally").
// Run: node tests/e2e-typing.mjs
import assert from "node:assert/strict";

const TWIN = process.env.PRISM_TWIN_BASE ?? "http://127.0.0.1:8081";
const RELAY = process.env.PRISM_RELAY_WS ?? "ws://127.0.0.1:8080/ws";

async function token(user) {
  const r = await fetch(`${TWIN}/twin/token?user=${encodeURIComponent(user)}`, { method: "POST" });
  assert.equal(r.status, 200, `twin token for ${user}: ${r.status}`);
  const j = await r.json();
  return j.access_token;
}

async function api(tok, path, init = {}) {
  const r = await fetch(`${TWIN}${path}`, { ...init, headers: { authorization: `Bearer ${tok}`, "content-type": "application/json", ...(init.headers ?? {}) } });
  const body = await r.json().catch(() => ({}));
  assert.ok(r.ok, `${init.method ?? "GET"} ${path}: ${r.status} ${JSON.stringify(body)}`);
  return body;
}

function connect(tok, name) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(RELAY);
    const inbox = [];
    const waiters = [];
    ws.onmessage = (e) => {
      const m = JSON.parse(e.data);
      inbox.push(m);
      for (const w of waiters.splice(0)) w();
    };
    ws.onerror = (e) => reject(new Error(`${name}: socket error ${e.message ?? ""}`));
    ws.onopen = () => {
      ws.send(JSON.stringify({ type: "hello", token: tok, device_id: `${name}-dev`, client_version: "e2e" }));
    };
    const until = (pred, ms = 5000) =>
      new Promise((res, rej) => {
        const t = setTimeout(() => rej(new Error(`${name}: timed out waiting; inbox=${JSON.stringify(inbox.slice(-5))}`)), ms);
        const check = () => {
          const hit = inbox.find(pred);
          if (hit) { clearTimeout(t); res(hit); return true; }
          return false;
        };
        if (!check()) waiters.push(check);
      });
    until((m) => m.type === "welcome").then((w) => resolve({ ws, inbox, until, user: w.user.name, send: (m) => ws.send(JSON.stringify(m)) }), reject);
  });
}

const [ta, tb] = await Promise.all([token("users/1001"), token("users/1002")]);
const spaces = (await api(ta, "/v1/spaces?pageSize=100")).spaces;
const dm = spaces.find((s) => s.spaceType === "DIRECT_MESSAGE") ?? spaces[0];
console.log(`A and B share ${spaces.length} spaces; using ${dm.name} (${dm.spaceType})`);

const A = await connect(ta, "A");
const B = await connect(tb, "B");
console.log(`connected: A=${A.user} B=${B.user}`);
A.send({ type: "subscribe", spaces: spaces.map((s) => s.name) });
B.send({ type: "subscribe", spaces: spaces.map((s) => s.name) });
await new Promise((r) => setTimeout(r, 300));

// 1. typing A -> B, never back to A
A.send({ type: "typing", space: dm.name, active: true });
const typing = await B.until((m) => m.type === "typing" && m.space === dm.name);
assert.equal(typing.user, A.user);
assert.equal(typing.active, true);
assert.equal(typing.ttl_ms, 5000);
await new Promise((r) => setTimeout(r, 200));
assert.ok(!A.inbox.some((m) => m.type === "typing"), "A must not receive its own typing");
console.log("ok  typing A -> B via relay, not echoed to A");

// 2. message A -> twin -> push -> relay -> B as event
const clientId = `client-${crypto.randomUUID()}`;
const sent = await api(ta, `/v1/${dm.name}/messages?messageId=${clientId}&messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD`, {
  method: "POST", body: JSON.stringify({ text: "e2e: hello from A **bold**" }),
});
assert.equal(sent.clientAssignedMessageId, clientId);
const ev = await B.until((m) => m.type === "event" && m.event.event_type === "google.workspace.chat.message.v1.created" && m.event.message?.name === sent.name);
assert.equal(ev.event.space, dm.name);
console.log(`ok  message ${sent.name} reached B as a created event`);

// 3. read receipt B -> A
B.send({ type: "read", space: dm.name, message: sent.name });
const rc = await A.until((m) => m.type === "receipt" && m.message === sent.name);
assert.equal(rc.user, B.user);
console.log("ok  read receipt B -> A");

// 4. presence: B goes DND, A sees it
B.send({ type: "presence", state: "dnd", custom_status: { emoji: "🎧", text: "heads down" } });
const pr = await A.until((m) => m.type === "presence" && m.user === B.user && m.state === "dnd");
assert.equal(pr.custom_status.text, "heads down");
console.log("ok  presence B -> A");

A.ws.close(); B.ws.close();
console.log("ALL GOOD");
process.exit(0);
