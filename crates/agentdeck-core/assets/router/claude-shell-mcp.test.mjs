import assert from 'node:assert/strict';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { PassThrough } from 'node:stream';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { scopedConnection } from './t3-mcp-call.mjs';
import { isStreamingArgs, extractConnection, updateConnection, seedFromArgs, formatResponse,
  createSnoop, serveOnce, launchStreaming, SOCKET_ENV } from './claude-shell-mcp.mjs';

const frame = (servers, type = 'control_request') =>
  JSON.stringify({ type, request: { subtype: 'mcp_set_servers', servers } });
const servers = (url, auth) =>
  ({ 't3-code': { type: 'http', url, headers: { Authorization: auth } } });
const good = servers('http://127.0.0.1:3773/mcp', 'Bearer first-secret');
const rotated = servers('http://127.0.0.1:3773/mcp', 'Bearer second-secret');

// Streaming-flag detection leaves production passthrough behavior unchanged.
assert.equal(isStreamingArgs(['--input-format', 'stream-json']), true);
assert.equal(isStreamingArgs(['--input-format=stream-json']), true);
assert.equal(isStreamingArgs(['--print', 'hi']), false);
assert.equal(isStreamingArgs(['--input-format', 'text']), false);

// Scope validation mirrors t3-mcp-call; invalid entries clear, never store.
assert.deepEqual(extractConnection(good), { endpoint: good['t3-code'].url, authorization: 'Bearer first-secret' });
for (const bad of [null, {}, { 't3-code': null },
  servers('https://127.0.0.1:1/mcp', 'Bearer x'),
  servers('http://evil.test/mcp', 'Bearer x'),
  servers('http://127.0.0.1:1/mcp?token=x', 'Bearer x'),
  servers('http://127.0.0.1:1/mcp', ''),
  servers('http://127.0.0.1:1/mcp', null),
  { 't3-code': { type: 'http', url: 'http://127.0.0.1:1/mcp', headers: {} } }])
  assert.equal(extractConnection(bad), null, JSON.stringify(bad));

// Legacy --mcp-config seeds from this launch's OWN args; set_servers later overrides.
const legacy = { mcpServers: good };
assert.deepEqual(seedFromArgs(['--mcp-config', JSON.stringify(legacy)]), extractConnection(good));
assert.deepEqual(seedFromArgs(['--mcp-config=' + JSON.stringify(legacy)]), extractConnection(good));
assert.deepEqual(seedFromArgs(['--mcp-config', '/fixture/config.json'],
  name => { assert.equal(name, '/fixture/config.json'); return JSON.stringify(legacy); }), extractConnection(good));
assert.equal(seedFromArgs(['--print']), null);
assert.equal(seedFromArgs(['--mcp-config', '{bad']), null);
assert.equal(seedFromArgs(['--mcp-config', '/missing.json', '--input-format', 'stream-json'],
  () => { throw new Error('private'); }), null);

// Token rotation replaces, removal clears, unrelated lines pass through.
let state = seedFromArgs(['--mcp-config', JSON.stringify(legacy)]);
assert.deepEqual(state, extractConnection(good));
state = updateConnection(state, frame(rotated));
assert.equal(state.authorization, 'Bearer second-secret');
state = updateConnection(state, frame({}));
assert.equal(state, null);
state = updateConnection(state, JSON.stringify({ type: 'user', message: { content: 'hi' } }));
assert.equal(state, null);

// Only a real control_request with request.subtype=mcp_set_servers applies.
// In particular, another control_request carrying a servers field must NOT
// clear stored credentials.
state = updateConnection(null, frame(good));
for (const decoy of [
  JSON.stringify({ type: 'control_request', request: { subtype: 'other', servers: {} } }),
  JSON.stringify({ type: 'control_request', request: { subtype: 'other', servers: { 't3-code': null } } }),
  JSON.stringify({ type: 'control_request', request: {} }),
  JSON.stringify({ type: 'control_request' }),
  JSON.stringify({ type: 'control_request', request: { subtype: 'mcp_set_servers' } }),
  JSON.stringify({ subtype: 'mcp_set_servers', servers: {} }),
])
  assert.deepEqual(updateConnection(state, decoy), state, decoy);

// Malformed frames never throw and never echo secrets into results.
for (const line of ['{', 'not json', '[1,2]', '"Bearer first-secret"', 'null',
  frame(servers('https://evil.test/mcp', 'Bearer evil-secret'))]) {
  const next = updateConnection(null, line);
  assert.equal(next, null);
  assert.ok(!String(next).includes('secret'));
}
for (const line of ['{', 'garbage']) assert.deepEqual(updateConnection(state, line), state);
assert.equal(formatResponse(null), '{"error":"not_configured"}\n');
assert.ok(!formatResponse(null).includes('secret'));
assert.equal(formatResponse({ endpoint: 'http://127.0.0.1:1/mcp', authorization: 'Bearer s' }),
  '{"endpoint":"http://127.0.0.1:1/mcp","authorization":"Bearer s"}\n');

// Snoop Transform: split UTF-8 + chunking reassembles; emitted bytes identical.
{
  const unicodeFrame = JSON.stringify({ type: 'user', message: { content: 'броня ✓' } }) + '\n';
  const raw = Buffer.concat([Buffer.from(frame(good) + '\n', 'utf8'), Buffer.from(unicodeFrame, 'utf8')]);
  const seen = [], out = [];
  const snoop = createSnoop(line => seen.push(updateConnection(null, line)));
  snoop.on('data', chunk => out.push(chunk));
  for (let i = 0; i < raw.length; i += 7) snoop.write(raw.slice(i, i + 7)); // splits multibyte chars
  snoop.end();
  await new Promise(resolve => snoop.on('end', resolve));
  assert.deepEqual(seen[0], extractConnection(good));
  assert.equal(seen[1], null);
  assert.equal(Buffer.compare(Buffer.concat(out), raw), 0, 'forwarded bytes must equal input bytes');
}

// Socket contract: push one JSON line on connect, no request, then end.
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'shell-mcp-test-'));
const sock = path.join(dir, 't.sock');
const readPush = () => new Promise((resolve, reject) => {
  const client = net.connect(sock);
  let data = '';
  client.on('data', c => { data += c; });
  client.on('end', () => resolve(data));
  client.on('error', reject);
});
let current = { endpoint: 'http://127.0.0.1:9/mcp', authorization: 'Bearer push-secret' };
const server = await serveOnce(sock, () => current);
assert.equal((fs.statSync(sock).mode & 0o777), 0o600);
assert.equal(await readPush(), '{"endpoint":"http://127.0.0.1:9/mcp","authorization":"Bearer push-secret"}\n');
current = null;
assert.equal(await readPush(), '{"error":"not_configured"}\n');
assert.ok(!(await readPush()).includes('push-secret'));
await new Promise(resolve => server.close(resolve));
fs.rmSync(dir, { recursive: true, force: true });

// Fake-child bridge: byte-identical stdin via pipe, socket rotation/removal.
// Provider spawn is injected; the socket belongs to this test launch.
{
  const stdin = new PassThrough();
  const writes = [];
  const fakeChild = { stdin: new PassThrough(), on: (ev, fn) => ev === 'exit' && setTimeout(() => fn(0), 50) };
  fakeChild.stdin.on('data', c => writes.push(c));
  let childEnv;
  const launched = await launchStreaming(['--input-format', 'stream-json'],
    { original: '/bin/fake-claude', stdin, env: { XDG_RUNTIME_DIR: os.tmpdir() }, spawnImpl: (bin, argv, opts) => {
      assert.equal(bin, '/bin/fake-claude');
      childEnv = opts.env;
      return fakeChild;
    } });
  assert.ok(childEnv[SOCKET_ENV] === launched.socketPath);
  const line = frame(good) + '\n';
  stdin.write(Buffer.from(line).slice(0, 10));
  stdin.write(Buffer.from(line).slice(10));
  await new Promise(r => setTimeout(r, 20));
  assert.deepEqual(launched.connection(), extractConnection(good));
  const got = await new Promise((resolve, reject) => {
    const client = net.connect(launched.socketPath);
    let data = '';
    client.on('data', c => { data += c; });
    client.on('end', () => resolve(data));
    client.on('error', reject);
  });
  assert.equal(got, formatResponse(extractConnection(good)));
  assert.deepEqual(await scopedConnection({ env: childEnv }), extractConnection(good));
  stdin.write(frame({}) + '\n'); // removal clears in-memory connection
  await new Promise(r => setTimeout(r, 20));
  assert.equal(launched.connection(), null);
  stdin.end(line); // trailing bytes still forwarded byte-identical
  await launched.wait;
  assert.equal(Buffer.concat(writes).toString('utf8'), line + frame({}) + '\n' + line);
  launched.cleanup();
  assert.ok(!fs.existsSync(launched.socketPath) && !fs.existsSync(launched.dir));
}

// Run the executable proxy and a real shell client in its own fake child.
// This checks SDK frame capture, inherited env, CLI response and exit cleanup.
{
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 't3-shell-cli-'));
  const fake = path.join(fixture, 'fake-claude');
  const helper = new URL('./t3-mcp-call.mjs', import.meta.url).pathname;
  fs.writeFileSync(fake, `#!/usr/bin/env node
import fs from 'node:fs';
import { spawnSync } from 'node:child_process';
process.stdin.once('data', () => {
  fs.writeFileSync(process.env.TEST_SOCKET_FILE, process.env.T3_SHELL_MCP_SOCKET);
  const r = spawnSync(process.execPath, [process.env.TEST_HELPER, 'orchestrator_capabilities'], { encoding: 'utf8' });
  process.stdout.write(r.stdout); process.stderr.write(r.stderr);
  process.exit(r.status ?? 1);
});
`, { mode: 0o700 });
  const methods = [];
  const httpServer = http.createServer(async (req, res) => {
    let raw = ''; for await (const chunk of req) raw += chunk;
    const body = JSON.parse(raw); methods.push(body.method);
    assert.equal(req.headers.authorization, 'Bearer fixture-secret');
    if (!body.id) { res.writeHead(202); res.end(); return; }
    res.setHeader('Content-Type', 'application/json');
    const result = body.method === 'initialize' ? { protocolVersion: '2025-06-18' }
      : { content: [], structuredContent: { parentThreadId: 'fixture-parent' } };
    res.end(JSON.stringify({ jsonrpc: '2.0', id: body.id, result }));
  });
  httpServer.listen(0, '127.0.0.1'); await once(httpServer, 'listening');
  let child;
  try {
    child = spawn(new URL('./claude-shell-mcp.mjs', import.meta.url).pathname,
      ['--input-format', 'stream-json'], { env: { ...process.env,
        CLAUDE_SHELL_MCP_ORIGINAL: fake, XDG_RUNTIME_DIR: fixture,
        TEST_SOCKET_FILE: path.join(fixture, 'socket-path'), TEST_HELPER: helper }, stdio: ['pipe', 'pipe', 'pipe'] });
    const out = [], err = [];
    child.stdout.on('data', c => out.push(c)); child.stderr.on('data', c => err.push(c));
    const exit = once(child, 'exit');
    child.stdin.end(frame(servers(`http://127.0.0.1:${httpServer.address().port}/mcp`, 'Bearer fixture-secret')) + '\n');
    const [code] = await exit;
    assert.equal(code, 0, Buffer.concat(err).toString());
    assert.equal(JSON.parse(Buffer.concat(out)).structuredContent.parentThreadId, 'fixture-parent');
    assert.deepEqual(methods, ['initialize', 'notifications/initialized', 'tools/call']);
    const socketPath = fs.readFileSync(path.join(fixture, 'socket-path'), 'utf8');
    assert.ok(!fs.existsSync(path.dirname(socketPath)), 'proxy removes its own session socket directory');
  } finally {
    httpServer.close();
    fs.rmSync(fixture, { recursive: true, force: true });
  }
}
console.log('claude-shell-mcp tests passed');
