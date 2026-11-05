import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, copyFileSync, rmSync } from 'node:fs';
import { generateKeyPairSync } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { publicKeyFingerprint, validateUpdaterTrust } from './updater-trust.mjs';

const read = file => JSON.parse(readFileSync(new URL(`../src-tauri/${file}`, import.meta.url), 'utf8'));
const activeConfig = read('tauri.conf.json');
const activePolicy = read('updater-trust.json');
// Keep frozen-client regressions independent from the active migration mode.
const policy = { schemaVersion: 1, mode: 'legacy', baselineVersion: activePolicy.baselineVersion, publicKey: activePolicy.publicKey, endpoint: activePolicy.endpoint };
const config = structuredClone(activeConfig);
config.plugins.updater.pubkey = policy.publicKey;
config.plugins.updater.endpoints = [policy.endpoint];
const copy = value => structuredClone(value);

test('active trust validates and frozen clients retain their legacy trust', () => {
  assert.doesNotThrow(() => validateUpdaterTrust(activeConfig, activePolicy));
  assert.equal(validateUpdaterTrust(config, policy).baselineVersion, '0.3.1');
  const later = copy(config);
  later.version = '0.10.0';
  assert.doesNotThrow(() => validateUpdaterTrust(later, policy));
  later.version = '1.0.0';
  assert.doesNotThrow(() => validateUpdaterTrust(later, policy));
});

test('a valid replacement key cannot silently strand installed clients', () => {
  const changed = copy(config);
  const lines = Buffer.from(changed.plugins.updater.pubkey, 'base64').toString('utf8').trim().split('\n');
  const packet = Buffer.from(lines[1], 'base64');
  const { publicKey } = generateKeyPairSync('ed25519');
  // Even a replacement with the SAME key ID must fail: key IDs aren't trust.
  publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).copy(packet, 10);
  lines[1] = packet.toString('base64');
  changed.plugins.updater.pubkey = Buffer.from(lines.join('\n') + '\n').toString('base64');
  assert.match(publicKeyFingerprint(changed.plugins.updater.pubkey), /^[a-f0-9]{64}$/);
  assert.throws(() => validateUpdaterTrust(changed, policy), /key changed without a compatible bridge/);
});

test('changing the key comment does not change the pinned trust packet', () => {
  const changed = copy(config);
  const lines = Buffer.from(changed.plugins.updater.pubkey, 'base64').toString('utf8').trim().split('\n');
  lines[0] = 'untrusted comment: harmless display label';
  changed.plugins.updater.pubkey = Buffer.from(lines.join('\n') + '\n').toString('base64');
  assert.doesNotThrow(() => validateUpdaterTrust(changed, policy));
});

test('endpoint changes and fallback endpoints require an explicit migration', () => {
  for (const endpoints of [[], ['https://example.com/latest.json'], [policy.endpoint, 'https://example.com/latest.json'], [policy.endpoint.replace('latest.json', 'latest-v2.json')]]) {
    const changed = copy(config);
    changed.plugins.updater.endpoints = endpoints;
    assert.throws(() => validateUpdaterTrust(changed, policy), /endpoint changed/);
  }
});

test('signed-version verification cannot be removed or disabled', () => {
  for (const value of [false, undefined, 'true']) {
    const changed = copy(config);
    changed.plugins.updater.requireSignedVersion = value;
    assert.throws(() => validateUpdaterTrust(changed, policy), /Signed-version/);
  }
});

test('malformed or unsupported key packets are rejected', () => {
  const outer = text => Buffer.from(text).toString('base64');
  for (const value of ['', 'not base64!', outer('untrusted comment: x\nAAAA\n'), outer('untrusted comment: x\n' + Buffer.alloc(42).toString('base64')), policy.publicKey + '!']) {
    assert.throws(() => publicKeyFingerprint(value), /public key/);
  }
});

test('a downgrade, invalid version or unimplemented policy cannot pass the release gate', () => {
  for (const version of ['0.2.9', '0.3.0', '0.3.2-beta.1', '00.3.2', null]) {
    const changed = copy(config);
    changed.version = version;
    assert.throws(() => validateUpdaterTrust(changed, policy), /version/);
  }
  for (const changed of [{ ...policy, mode: 'bridge' }, { ...policy, schemaVersion: 2 }, { ...policy, endpoint: 'https://example.com/latest.json' }]) {
    assert.throws(() => validateUpdaterTrust(config, changed), /policy|pinned/);
  }
});

test('the standalone check reads committed trust regardless of the caller directory', () => {
  const result = spawnSync(process.execPath, [new URL('./check-updater-trust.mjs', import.meta.url).pathname], { cwd: tmpdir(), encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Updater trust matches 0\.3\.1/);
  const override = spawnSync(process.execPath, [new URL('./check-updater-trust.mjs', import.meta.url).pathname, '--skip'], { encoding: 'utf8' });
  assert.notEqual(override.status, 0);
  assert.match(override.stderr, /no overrides/);
});

test('collection and draft upload reject incompatible config before reading artifacts or calling GitHub', t => {
  const root = mkdtempSync(join(tmpdir(), 'clamav-trust-gate-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'scripts'));
  mkdirSync(join(root, 'src-tauri'));
  for (const file of ['collect-release.mjs', 'draft-release.mjs', 'release-utils.mjs', 'updater-trust.mjs', 'release-feeds.mjs']) {
    copyFileSync(new URL(`./${file}`, import.meta.url), join(root, 'scripts', file));
  }
  const changed = copy(config);
  changed.plugins.updater.endpoints = ['https://example.com/changed.json'];
  writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify(changed));
  writeFileSync(join(root, 'src-tauri/updater-trust.json'), JSON.stringify(policy));
  for (const file of ['collect-release.mjs', 'draft-release.mjs']) {
    const result = spawnSync(process.execPath, [join(root, 'scripts', file)], { cwd: root, encoding: 'utf8' });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /endpoint changed without a compatible bridge/);
    assert.doesNotMatch(result.stderr, /ENOENT|reviewed GitHub commit|Draft creation failed/);
  }
});

test('signed preflight rejects drift before any credential or notarization subprocess', { skip: process.platform !== 'darwin' }, t => {
  const root = mkdtempSync(join(tmpdir(), 'clamav-trust-preflight-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const directory of ['scripts', 'src-tauri/core']) mkdirSync(join(root, directory), { recursive: true });
  for (const file of ['release-macos.mjs', 'release-utils.mjs', 'updater-trust.mjs', 'release-feeds.mjs']) {
    copyFileSync(new URL(`./${file}`, import.meta.url), join(root, 'scripts', file));
  }
  const changed = copy(config);
  changed.plugins.updater.requireSignedVersion = false;
  writeFileSync(join(root, 'package.json'), JSON.stringify({ version: config.version }));
  for (const file of ['src-tauri/Cargo.toml', 'src-tauri/core/Cargo.toml']) writeFileSync(join(root, file), `version = "${config.version}"\n`);
  writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify(changed));
  writeFileSync(join(root, 'src-tauri/updater-trust.json'), JSON.stringify(policy));
  // Never touch the real keychain or network, even if a future edit moves the gate.
  const guard = join(root, 'guard.mjs');
  writeFileSync(guard, `import childProcess from 'node:child_process';\nimport {syncBuiltinESMExports} from 'node:module';\nchildProcess.spawnSync = () => {throw new Error('Unexpected external process before trust check');};\nsyncBuiltinESMExports();\n`);
  const result = spawnSync(process.execPath, ['--import', guard, join(root, 'scripts/release-macos.mjs'), '--signed', '--check'], { cwd: root, encoding: 'utf8' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Signed-version verification must remain enabled/);
  assert.doesNotMatch(result.stderr, /Unexpected external process/);
  changed.plugins.updater.requireSignedVersion = true;
  changed.version = '0.3.2';
  writeFileSync(join(root, 'package.json'), JSON.stringify({ version: changed.version }));
  for (const file of ['src-tauri/Cargo.toml', 'src-tauri/core/Cargo.toml']) writeFileSync(join(root, file), `version = "${changed.version}"\n`);
  writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify(changed));
  const retired = spawnSync(process.execPath, ['--import', guard, join(root, 'scripts/release-macos.mjs'), '--signed', '--check'], { cwd: root, encoding: 'utf8' });
  assert.notEqual(retired.status, 0);
  assert.match(retired.stderr, /legacy updater key is retired/);
  assert.doesNotMatch(retired.stderr, /Unexpected external process/);
});
