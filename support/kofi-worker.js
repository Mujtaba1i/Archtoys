// Archtoys: Ko-fi -> GitHub, so supporters are thanked in SUPPORTERS.md by themselves.
//
// Ko-fi sends every tip here (its "webhook"). This Worker:
//   1. checks the message really comes from Ko-fi (the verification token),
//   2. keeps only one-time tips from supporters who chose to be public,
//   3. asks GitHub to run the "Add a supporter" Action with the name and
//      message only (never the email, the amount or anything else).
// A test from Ko-fi's "Send a test" button runs the Action too, marked as a
// test, so you can see the whole chain work; the Action adds nobody for it.
//
// Runs on Cloudflare Workers (free plan: 100,000 calls a day). Two secrets, set
// in the Worker's Settings > Variables and Secrets:
//   KOFI_TOKEN    Ko-fi > Settings > More > API > Verification Token
//   GITHUB_TOKEN  a fine-grained GitHub token, Archtoys only, Contents: read and write
// Optional plain variable: GITHUB_REPO (default Mujtaba1i/Archtoys).
//
// Everything it does is written to the Worker's "Logs" tab in plain words.

const DEFAULT_REPO = "Mujtaba1i/Archtoys";
const EVENT = "kofi-supporter";
const MAX_NAME = 60;
const MAX_MESSAGE = 200;

// Ko-fi's own test payments use this transaction id and example.com addresses.
const TEST_TRANSACTION = "00000000-1111-2222-3333-444444444444";

export default {
  async fetch(request, env) {
    return handle(request, env, fetch);
  },
};

// The whole job, with `fetchImpl` passed in so tests can stand in for GitHub.
export async function handle(request, env, fetchImpl) {
  if (request.method === "GET") {
    return text(200, "Archtoys Ko-fi webhook: running. Ko-fi sends tips here; nothing to see in a browser.");
  }
  if (request.method !== "POST") {
    return text(405, "Only Ko-fi's POST requests are accepted here.");
  }
  if (!env.KOFI_TOKEN || !env.GITHUB_TOKEN) {
    console.log("Not set up: add the KOFI_TOKEN and GITHUB_TOKEN secrets in Settings > Variables and Secrets.");
    return text(500, "This webhook isn't set up yet.");
  }

  let tip;
  try {
    tip = await readKofi(request);
  } catch (err) {
    console.log(`Not a Ko-fi message: ${err.message}`);
    return text(400, "That doesn't look like a message from Ko-fi.");
  }

  if (!sameSecret(String(tip.verification_token ?? ""), env.KOFI_TOKEN)) {
    console.log("Wrong verification token: this didn't come from your Ko-fi page (or KOFI_TOKEN is out of date). Ignored.");
    return text(403, "Wrong verification token.");
  }

  const test = isTest(tip);
  if (tip.type !== "Donation" && !test) {
    console.log(`A "${tip.type}" from Ko-fi, not a one-time tip: not added.`);
    return text(200, "Thanks, Ko-fi. Not a one-time tip, nothing to do.");
  }
  if (tip.is_public !== true && !test) {
    // A private supporter: their name isn't even sent to GitHub.
    console.log("A tip from a supporter who chose to stay private: thank you, and not listed.");
    return text(200, "Thanks, Ko-fi. Private supporter, not listed.");
  }

  const supporter = {
    name: clean(tip.from_name, MAX_NAME) || "Someone",
    message: clean(tip.message, MAX_MESSAGE),
    id: clean(tip.message_id || tip.kofi_transaction_id, 80),
    test,
  };

  const repo = env.GITHUB_REPO || DEFAULT_REPO;
  const res = await fetchImpl(`https://api.github.com/repos/${repo}/dispatches`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${env.GITHUB_TOKEN}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "archtoys-kofi-worker",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ event_type: EVENT, client_payload: supporter }),
  });

  if (res.status === 204) {
    console.log(test
      ? `Ko-fi test received: GitHub is running "Add a supporter" as a test (nobody is added).`
      : `New public supporter "${supporter.name}": GitHub is adding them to SUPPORTERS.md.`);
    return text(200, "Thanks, Ko-fi. Passed to GitHub.");
  }

  const why = {
    401: "GitHub refused the token: it has expired or was deleted. Make a new fine-grained token (Archtoys only, Contents: read and write) and replace the GITHUB_TOKEN secret.",
    403: "GitHub refused: the token can't start Actions on this repo. It needs Contents: read and write, for Archtoys.",
    404: `GitHub can't find ${repo} with this token: check the repo name (GITHUB_REPO) and that the token includes it.`,
  }[res.status] ?? `GitHub answered ${res.status}: ${(await res.text()).slice(0, 200)}`;
  console.log(`Couldn't pass "${supporter.name}" to GitHub. ${why}`);
  // Not 200, so Ko-fi knows it didn't go through.
  return text(502, "Couldn't pass this on to GitHub. See the Worker's logs.");
}

// Ko-fi posts a form with one field, `data`, holding the tip as JSON.
async function readKofi(request) {
  const type = request.headers.get("content-type") || "";
  if (!type.includes("application/x-www-form-urlencoded")) {
    throw new Error(`expected a form, got "${type || "nothing"}"`);
  }
  const form = await request.formData();
  const data = form.get("data");
  if (typeof data !== "string" || data.length > 20000) {
    throw new Error("no `data` field");
  }
  const tip = JSON.parse(data);
  if (!tip || typeof tip !== "object") {
    throw new Error("`data` isn't an object");
  }
  return tip;
}

function isTest(tip) {
  return tip.kofi_transaction_id === TEST_TRANSACTION || /@example\.(com|org)$/i.test(String(tip.email ?? ""));
}

// Plain one-line text: no control characters, no line breaks, trimmed and cut to `max`.
function clean(value, max) {
  const flat = String(value ?? "").replace(/[\u0000-\u001f\u007f\u2028\u2029]+/g, " ").replace(/\s+/g, " ").trim();
  return [...flat].slice(0, max).join("");
}

// Compares two secrets without stopping at the first different character.
function sameSecret(a, b) {
  const enc = new TextEncoder();
  const x = enc.encode(a), y = enc.encode(b);
  let diff = x.length ^ y.length;
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    diff |= (x[i] ?? 0) ^ (y[i] ?? 0);
  }
  return diff === 0;
}

function text(status, body) {
  return new Response(body, { status, headers: { "Content-Type": "text/plain; charset=utf-8" } });
}
