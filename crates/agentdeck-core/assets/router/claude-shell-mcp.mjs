#!/usr/bin/env node
// Launcher proxy for JSON-file shell MCP delegation (T3 Claude command).
// Forwards args/stdin/stdout/stderr to the original Claude binary unchanged,
// snoops only stdin NDJSON control_request mcp_set_servers into memory, and
// serves it on a private per-launch Unix socket named by T3_SHELL_MCP_SOCKET.
// No credential files, dumps, logs, or command-line secret values.
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { StringDecoder } from 'node:string_decoder';
import { Transform } from 'node:stream';

export const SOCKET_ENV = 'T3_SHELL_MCP_SOCKET';
export const ORIGINAL_BIN =
  process.env.CLAUDE_SHELL_MCP_ORIGINAL ?? path.join(os.homedir(), '.local/bin/claude');

export function isStreamingArgs(argv = []) {
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--input-format') return argv[i + 1] === 'stream-json';
    if (argv[i].startsWith('--input-format=')) return argv[i].slice(15) === 'stream-json';
  }
  return false;
}

// Same scope rule as t3-mcp-call: loopback http, no userinfo/query/hash, bearer set.
export function extractConnection(servers) {
  const entry = servers?.['t3-code'];
  if (!entry || typeof entry !== 'object') return null;
  const authorization = entry.headers && typeof entry.headers === 'object'
    ? Object.entries(entry.headers).find(([k]) => k.toLowerCase() === 'authorization')?.[1]
    : undefined;
  let url;
  try { url = new URL(entry.url); } catch { return null; }
  if (url.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
    || url.username || url.password || url.search || url.hash
    || typeof authorization !== 'string' || !authorization.trim()) return null;
  return { endpoint: entry.url, authorization };
}

// Accepts only a real control_request with request.subtype=mcp_set_servers;
// anything else (including other control_request types carrying a servers
// field) leaves the stored connection untouched. Never throws, never leaks.
export function updateConnection(current, text) {
  let obj;
  try { obj = JSON.parse(text); } catch { return current; }
  if (!obj || typeof obj !== 'object' || obj.type !== 'control_request') return current;
  const request = obj.request;
  if (!request || typeof request !== 'object' || request.subtype !== 'mcp_set_servers') return current;
  const servers = request.servers;
  if (!servers || typeof servers !== 'object') return current;
  return extractConnection(servers);
}

export function seedFromArgs(argv = [], readFile = fs.readFileSync) {
  // Legacy path: seed from this launch's OWN --mcp-config only; later
  // mcp_set_servers frames are authoritative and can rotate/remove it.
  // Never throws and never echoes secrets.
  try {
    const index = argv.indexOf('--mcp-config');
    const value = index >= 0 ? argv[index + 1]
      : argv.find(a => a.startsWith('--mcp-config='))?.slice(13);
    if (!value) return null;
    const text = value.trim().startsWith('{') ? value : readFile(value, 'utf8');
    const config = JSON.parse(text);
    if (!config || typeof config !== 'object' || !config.mcpServers) return null;
    return extractConnection(config.mcpServers);
  } catch { return null; }
}

export function formatResponse(connection) {
  if (connection && typeof connection.endpoint === 'string'
    && typeof connection.authorization === 'string')
    return JSON.stringify({ endpoint: connection.endpoint, authorization: connection.authorization }) + '\n';
  return JSON.stringify({ error: 'not_configured' }) + '\n';
}

// Pass-through Transform: snoops complete `\n`-terminated lines for control
// frames while emitting the original bytes unchanged. Piping through it
// preserves backpressure; StringDecoder keeps split UTF-8 sequences intact.
export function createSnoop(onLine) {
  const decoder = new StringDecoder('utf8');
  let buffer = '';
  return new Transform({
    transform(chunk, _encoding, callback) {
      buffer += decoder.write(chunk);
      let index;
      while ((index = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, index);
        buffer = buffer.slice(index + 1);
        onLine(line.replace(/\r$/, ''));
      }
      callback(null, chunk);
    },
    flush(callback) {
      buffer += decoder.end();
      if (buffer.length) { const line = buffer; buffer = ''; onLine(line.replace(/\r$/, '')); }
      callback();
    },
  });
}

export async function serveOnce(socketPath, getConnection) {
  const dir = path.dirname(socketPath);
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  fs.chmodSync(dir, 0o700);
  try { fs.unlinkSync(socketPath); } catch {}
  // Root socket-client contract: server pushes one JSON line on connect,
  // {endpoint, authorization} or {error:"not_configured"}, then ends. No request.
  const server = net.createServer(socket => {
    socket.on('error', () => {});
    socket.end(formatResponse(getConnection()));
  });
  await new Promise((resolve, reject) => {
    server.on('error', reject);
    server.listen(socketPath, () => { server.off('error', reject); resolve(); });
  });
  fs.chmodSync(socketPath, 0o600);
  return server;
}

// Testable streaming launch: fake `original` executable and `stdin` allowed;
// production flags and forwarding behavior unchanged.
export async function launchStreaming(argv, { original = ORIGINAL_BIN, stdin = process.stdin, env = process.env, spawnImpl = spawn } = {}) {
  let connection = seedFromArgs(argv);
  const runtime = env.XDG_RUNTIME_DIR ?? os.tmpdir();
  const dir = fs.mkdtempSync(path.join(runtime, 'claude-shell-mcp-'));
  fs.chmodSync(dir, 0o700);
  const socketPath = path.join(dir, 't3-shell-mcp.sock');
  const server = await serveOnce(socketPath, () => connection);
  const child = spawnImpl(original, argv, {
    stdio: ['pipe', 'inherit', 'inherit'],
    env: { ...env, [SOCKET_ENV]: socketPath },
  });
  const snoop = createSnoop(line => { connection = updateConnection(connection, line); });
  child.stdin.on('error', () => {});
  stdin.pipe(snoop).pipe(child.stdin);
  const wait = new Promise(resolve => { child.on('exit', resolve); child.on('error', () => resolve(127)); });
  const cleanup = () => {
    server.close();
    try { fs.unlinkSync(socketPath); } catch {}
    try { fs.rmdirSync(dir); } catch {}
  };
  return { socketPath, dir, server, child, connection: () => connection, wait, cleanup };
}

async function main() {
  const args = process.argv.slice(2);
  if (!isStreamingArgs(args)) {
    const child = spawn(ORIGINAL_BIN, args, { stdio: 'inherit' });
    const code = await new Promise(resolve => { child.on('exit', resolve); child.on('error', () => resolve(127)); });
    process.exit(code ?? 127);
    return;
  }
  const launched = await launchStreaming(args, {});
  // Termination forwards only to the captured child PID, never a group/name kill.
  for (const signal of ['SIGTERM', 'SIGINT', 'SIGHUP'])
    process.on(signal, () => { try { launched.child.kill(signal); } catch {} });
  const code = await launched.wait;
  launched.cleanup();
  process.exit(code ?? 127);
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === new URL(import.meta.url).pathname)
  main().catch(error => { console.error('claude-shell-mcp failed.'); process.exitCode = 1; });
