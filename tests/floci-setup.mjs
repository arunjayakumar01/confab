// Creates the Pub/Sub topic and push subscription on the Floci GCP emulator so
// events flow twin -> Pub/Sub -> relay exactly as they will on real GCP.
// Run: node tests/floci-setup.mjs   (env: PUBSUB_EMULATOR_HOST, PRISM_PUSH_ENDPOINT)
const host = process.env.PUBSUB_EMULATOR_HOST ?? "localhost:4588";
const project = process.env.GOOGLE_CLOUD_PROJECT ?? "floci-local";
const topic = `projects/${project}/topics/${process.env.PRISM_TOPIC ?? "prism-events"}`;
const sub = `projects/${project}/subscriptions/${process.env.PRISM_SUBSCRIPTION ?? "prism-relay"}`;
const endpoint = process.env.PRISM_PUSH_ENDPOINT ?? "http://host.docker.internal:8080/events/push?secret=dev";
const base = `http://${host}/v1`;

async function put(path, body) {
  let r;
  for (let i = 0; ; i++) {
    try { r = await fetch(`${base}/${path}`, { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }); break; }
    catch (e) { if (i >= 60) throw e; await new Promise((res) => setTimeout(res, 1000)); }
  }
  const text = await r.text();
  if (r.ok) return JSON.parse(text || "{}");
  if (r.status === 409) return { exists: true };
  throw new Error(`PUT ${path} -> ${r.status} ${text}`);
}

const t = await put(topic, {});
console.log(`topic ${topic}: ${t.exists ? "exists" : "created"}`);
const s = await put(sub, { topic, pushConfig: { pushEndpoint: endpoint }, ackDeadlineSeconds: 10 });
console.log(`subscription ${sub}: ${s.exists ? "exists" : "created"} -> ${endpoint}`);
