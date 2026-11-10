import assert from "node:assert/strict";
import { handle } from "./kofi-worker.js";
const env = { KOFI_TOKEN: "secret-123", GITHUB_TOKEN: "ghp_x" };
const tip = (o = {}) => ({ verification_token: "secret-123", message_id: "m1", type: "Donation", is_public: true,
  from_name: "Sara", message: "Love it!", amount: "6.00", email: "sara@mail.com", kofi_transaction_id: "abc", ...o });
const req = (data, method = "POST", type = "application/x-www-form-urlencoded") =>
  new Request("https://w.example/", { method, headers: { "content-type": type },
    body: method === "POST" ? new URLSearchParams({ data: typeof data === "string" ? data : JSON.stringify(data) }) : undefined });
function gh(status = 204) { const calls = []; const f = async (url, init) => { calls.push({ url, body: JSON.parse(init.body), auth: init.headers.Authorization }); return new Response(status === 204 ? null : "nope", { status }); }; f.calls = calls; return f; }
const quiet = console.log; let logs = []; console.log = (m) => logs.push(m);
let f, r;
const check = async (name, fn) => { logs = []; await fn(); quiet(`ok   ${name}  | log: ${logs.join(" / ") || "-"}`); };

await check("public tip goes to GitHub with name+message only", async () => {
  f = gh(); r = await handle(req(tip()), env, f);
  assert.equal(r.status, 200); assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].url, "https://api.github.com/repos/Mujtaba1i/Archtoys/dispatches");
  assert.equal(f.calls[0].auth, "Bearer ghp_x");
  assert.deepEqual(f.calls[0].body, { event_type: "kofi-supporter", client_payload: { name: "Sara", message: "Love it!", id: "m1", test: false } });
  assert.ok(!JSON.stringify(f.calls[0].body).includes("sara@mail.com") && !JSON.stringify(f.calls[0].body).includes("6.00"));
});
await check("wrong token is refused, GitHub not called", async () => {
  f = gh(); r = await handle(req(tip({ verification_token: "nope" })), env, f); assert.equal(r.status, 403); assert.equal(f.calls.length, 0); });
await check("private supporter: not sent to GitHub", async () => {
  f = gh(); r = await handle(req(tip({ is_public: false })), env, f); assert.equal(r.status, 200); assert.equal(f.calls.length, 0); });
await check("Ko-fi test payment runs the Action marked as test", async () => {
  f = gh(); r = await handle(req(tip({ kofi_transaction_id: "00000000-1111-2222-3333-444444444444", email: "jo.example@example.com", from_name: "Jo Example", is_public: true })), env, f);
  assert.equal(r.status, 200); assert.equal(f.calls[0].body.client_payload.test, true); });
await check("shop order / subscription ignored", async () => {
  f = gh(); r = await handle(req(tip({ type: "Shop Order" })), env, f); assert.equal(r.status, 200); assert.equal(f.calls.length, 0); });
await check("name: control chars and line breaks removed, cut to 60", async () => {
  f = gh(); await handle(req(tip({ from_name: "A\nB\u0000C " + "x".repeat(100), message: "hi\r\nthere" })), env, f);
  const p = f.calls[0].body.client_payload; assert.ok(!/[\n\r\u0000]/.test(p.name)); assert.equal([...p.name].length, 60); assert.equal(p.message, "hi there"); });
await check("empty name becomes 'Someone'", async () => {
  f = gh(); await handle(req(tip({ from_name: "   " })), env, f); assert.equal(f.calls[0].body.client_payload.name, "Someone"); });
await check("not a form / bad JSON -> 400", async () => {
  f = gh(); r = await handle(req("{not json"), env, f); assert.equal(r.status, 400);
  r = await handle(new Request("https://w.example/", { method: "POST", headers: { "content-type": "application/json" }, body: "{}" }), env, f); assert.equal(r.status, 400); assert.equal(f.calls.length, 0); });
await check("GET shows it is running", async () => { r = await handle(new Request("https://w.example/"), env, gh()); assert.equal(r.status, 200); });
await check("missing secrets -> 500 with a clear log", async () => { r = await handle(req(tip()), {}, gh()); assert.equal(r.status, 500); });
await check("expired GitHub token -> 502 and says how to fix", async () => {
  f = gh(401); r = await handle(req(tip()), env, f); assert.equal(r.status, 502); assert.ok(logs.join().includes("expired")); });
await check("custom repo via GITHUB_REPO", async () => {
  f = gh(); await handle(req(tip()), { ...env, GITHUB_REPO: "someone/fork" }, f); assert.ok(f.calls[0].url.includes("someone/fork")); });
console.log = quiet; console.log("ALL PASSED");
