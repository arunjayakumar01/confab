// Drives one relay session as a given twin user: keeps a typing indicator alive in a
// space for N seconds and optionally posts a message through the twin.
// node tests/drive.mjs users/1002 spaces/XYZ [seconds] ["message text"]
const [user, spaceArg, secsArg, text] = process.argv.slice(2);
const TWIN = process.env.PRISM_TWIN_BASE ?? "http://127.0.0.1:8081";
const RELAY = process.env.PRISM_RELAY_WS ?? "ws://127.0.0.1:8080/ws";
const secs = Number(secsArg ?? 20);
const tok = (await (await fetch(`${TWIN}/twin/token?user=${encodeURIComponent(user)}`, { method: "POST" })).json()).access_token;
const spaces = (await (await fetch(`${TWIN}/v1/spaces?pageSize=100`, { headers: { authorization: `Bearer ${tok}` } })).json()).spaces;
const space = spaceArg && spaceArg !== "-" ? spaceArg : spaces.find((s) => s.displayName === "announcements")?.name ?? spaces[0].name;
const ws = new WebSocket(RELAY);
await new Promise((r) => (ws.onopen = r));
ws.send(JSON.stringify({ type: "hello", token: tok, device_id: "drive", client_version: "drive" }));
await new Promise((r) => (ws.onmessage = (e) => JSON.parse(e.data).type === "welcome" && r()));
ws.send(JSON.stringify({ type: "subscribe", spaces: spaces.map((s) => s.name) }));
console.log(`${user} typing in ${space} for ${secs}s`);
const t0 = Date.now();
const tick = setInterval(() => ws.send(JSON.stringify({ type: "typing", space, active: true })), 2000);
ws.send(JSON.stringify({ type: "typing", space, active: true }));
if (text) {
  setTimeout(async () => {
    const r = await fetch(`${TWIN}/v1/${space}/messages?messageId=client-${crypto.randomUUID()}`, {
      method: "POST", headers: { authorization: `Bearer ${tok}`, "content-type": "application/json" }, body: JSON.stringify({ text }),
    });
    console.log(`posted: ${r.status}`);
  }, Math.max(1000, (secs * 1000) / 2));
}
setTimeout(() => { clearInterval(tick); ws.send(JSON.stringify({ type: "typing", space, active: false })); ws.close(); console.log(`done after ${(Date.now() - t0) / 1000}s`); process.exit(0); }, secs * 1000);
