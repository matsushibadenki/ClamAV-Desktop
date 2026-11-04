import { test } from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPairSync } from 'node:crypto';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, rmSync, existsSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { manifest, checksum } from './release-utils.mjs';
import { validateUpdaterTrust, modernEndpoint } from './updater-trust.mjs';
import { createFeeds, verifiedRelease, trustReceipt } from './release-feeds.mjs';

const sourceConfig = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url)));
const sourcePolicy = JSON.parse(readFileSync(new URL('../src-tauri/updater-trust.json', import.meta.url)));
const legacy = readFileSync(new URL('../src-tauri/updater-legacy.json', import.meta.url));
const json = value => Buffer.from(JSON.stringify(value, null, 2) + '\n');
function migration() {
  const config = structuredClone(sourceConfig);
  const lines = Buffer.from(sourcePolicy.publicKey, 'base64').toString().trim().split('\n');
  const packet = Buffer.from(lines[1], 'base64');
  const { publicKey } = generateKeyPairSync('ed25519');
  publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).copy(packet, 10);
  lines[1] = packet.toString('base64');
  const key = Buffer.from(lines.join('\n') + '\n').toString('base64');
  const policy = { ...sourcePolicy, schemaVersion: 2, mode: 'manual', legacyFeedSha256: checksum(legacy), successor: { publicKey: key, endpoint: modernEndpoint, firstVersion: '0.4.0' } };
  config.version = '0.4.0'; // Fixture version, not a release reservation.
  config.plugins.updater.pubkey = key;
  config.plugins.updater.endpoints = [modernEndpoint];
  return { config, policy };
}
function fixture(t, manual = true) {
  const root = mkdtempSync(join(tmpdir(), 'clamav-release-feeds-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const { config, policy } = manual ? migration() : { config: structuredClone(sourceConfig), policy: structuredClone(sourcePolicy) };
  const bundle = join(root, 'src-tauri/target/release/bundle');
  const verify = join(bundle, 'verification');
  for (const dir of ['scripts', 'src-tauri/core', 'src-tauri/target/release/bundle/verification', 'src-tauri/target/release/bundle/macos', 'src-tauri/target/release/bundle/dmg']) mkdirSync(join(root, dir), { recursive: true });
  for (const file of ['collect-release.mjs', 'merge-updater.mjs', 'draft-release.mjs', 'release-feeds.mjs', 'release-utils.mjs', 'updater-trust.mjs', 'check-updater-trust.mjs', 'release-macos.mjs']) copyFileSync(new URL(file, import.meta.url), join(root, 'scripts', file));
  writeFileSync(join(root, 'package.json'), json({ version: config.version }));
  for (const file of ['src-tauri/Cargo.toml', 'src-tauri/core/Cargo.toml']) writeFileSync(join(root, file), `version = "${config.version}"\n`);
  writeFileSync(join(root, 'src-tauri/tauri.conf.json'), json(config));
  writeFileSync(join(root, 'src-tauri/updater-trust.json'), json(policy));
  writeFileSync(join(root, 'src-tauri/updater-legacy.json'), legacy);
  const trust = validateUpdaterTrust(config, policy);
  const archive = `ClamAV-Desktop_${config.version}_aarch64.app.tar.gz`;
  const dmg = `ClamAV-Desktop_${config.version}_aarch64.dmg`;
  const bytes = Buffer.from('synthetic packaging fixture, not executable software');
  const signature = Buffer.from('fixture-signature\n');
  writeFileSync(join(bundle, 'macos', archive), bytes);
  writeFileSync(join(bundle, 'macos', archive + '.sig'), signature);
  writeFileSync(join(bundle, 'dmg', dmg), bytes);
  const feed = manifest(config.version, 'darwin-aarch64', archive, signature.toString().trim(), '2026-11-02T00:00:00Z');
  const info = { version: config.version, architecture: 'arm64', development: false, appNotarizationVerified: true, dmgNotarizationVerified: true, updaterTrust: trustReceipt(trust), artifacts: [{ file: dmg, sha256: checksum(bytes) }, { file: archive, sha256: checksum(bytes) }] };
  writeFileSync(join(verify, `${trust.manifestPrefix}aarch64.json`), json(feed));
  writeFileSync(join(verify, 'release.json'), json(info));
  writeFileSync(join(verify, 'SHA256SUMS'), info.artifacts.map(a => `${a.sha256}  ${a.file}`).join('\n') + '\n');
  const stage = join(root, 'staging');
  const env = { ...process.env, GITHUB_SHA: 'a'.repeat(40) };
  const run = (file, args = [], extra = {}) => spawnSync(process.execPath, [join(root, 'scripts', file), ...args], { cwd: root, encoding: 'utf8', env, ...extra });
  const pass = (file, args) => { const r = run(file, args); assert.equal(r.status, 0, r.stderr); return r; };
  const collect = () => pass('collect-release.mjs', [stage]);
  const merge = () => pass('merge-updater.mjs', [stage]);
  return { root, config, policy, trust, stage, verify, bundle, archive, dmg, feed, info, run, pass, collect, merge };
}

test('manual migration requires distinct key, dedicated endpoint, pinned feed and a valid boundary', () => {
  const { config, policy } = migration();
  assert.equal(validateUpdaterTrust(config, policy).feedName, 'latest-v2.json');
  for (const mutate of [p => p.successor.publicKey = p.publicKey, p => p.successor.endpoint = p.endpoint, p => p.successor.firstVersion = p.baselineVersion, p => p.successor.firstVersion = '0.5.0', p => p.legacyFeedSha256 = 'bad']) {
    const changed = structuredClone(policy); mutate(changed);
    assert.throws(() => validateUpdaterTrust(config, changed));
  }
  const oldClient = structuredClone(config); oldClient.plugins.updater.pubkey = policy.publicKey;
  assert.throws(() => validateUpdaterTrust(oldClient, policy), /public key changed/);
});

test('manual public preflight verifies the frozen feed without signing credentials', t => {
  const f = fixture(t);
  const result = f.pass('check-updater-trust.mjs', []);
  assert.match(result.stdout, /manual \/ latest-v2.json/);
  writeFileSync(join(f.root, 'src-tauri/updater-legacy.json'), '{}');
  const failed = f.run('check-updater-trust.mjs');
  assert.notEqual(failed.status, 0);
  assert.match(failed.stderr, /legacy feed hash/);
});

test('new-channel feed advances while the reviewed legacy bytes remain unchanged', () => {
  const { config, policy } = migration();
  for (const version of ['0.4.0', '0.5.0']) {
    const next = { ...config, version };
    const feed = manifest(version, 'darwin-aarch64', `ClamAV-Desktop_${version}_aarch64.app.tar.gz`, 'new-signature');
    const files = createFeeds(next, policy, [feed], legacy);
    assert.deepEqual(files.get('latest.json'), legacy);
    assert.equal(JSON.parse(files.get('latest-v2.json')).version, version);
  }
  const bad = Buffer.from(legacy); bad[20] ^= 1;
  assert.throws(() => createFeeds(config, policy, [manifest(config.version, 'darwin-aarch64', 'next.app.tar.gz', 's')], bad), /legacy feed hash/);
});

test('collect → merge → draft preflight carries exactly the two feeds without an upload', t => {
  const f = fixture(t);
  f.collect(); f.merge();
  const result = f.pass('draft-release.mjs', [f.stage, '--check']);
  const report = JSON.parse(result.stdout);
  assert.equal(report.mode, 'manual');
  assert.equal(report.files.length, 7);
  assert(report.files.includes('latest.json') && report.files.includes('latest-v2.json'));
  assert(!report.files.some(file => file.includes('latest-v2-aarch64')));
  assert.deepEqual(readFileSync(join(f.stage, 'latest.json')), legacy);
  assert.equal(JSON.parse(readFileSync(join(f.stage, 'latest-v2.json'))).version, f.config.version);
});

test('legacy collection and draft remain compatible with a six-file single feed release', t => {
  const f = fixture(t, false);
  delete f.info.updaterTrust; // Existing verified 0.3.1 metadata predates receipts.
  writeFileSync(join(f.verify, 'release.json'), json(f.info));
  f.collect(); f.merge();
  const report = JSON.parse(f.pass('draft-release.mjs', [f.stage, '--check']).stdout);
  assert.equal(report.files.length, 6);
  assert(!report.files.includes('latest-v2.json'));
});

test('stale or missing trust receipt cannot relabel an old build as a new-key release', t => {
  const f = fixture(t);
  for (const receipt of [undefined, { ...f.info.updaterTrust, fingerprint: '0'.repeat(64) }, { ...f.info.updaterTrust, mode: 'legacy' }]) {
    writeFileSync(join(f.verify, 'release.json'), json({ ...f.info, updaterTrust: receipt }));
    const result = f.run('collect-release.mjs', [f.stage]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /trust receipt/);
    assert(!existsSync(f.stage));
  }
});

test('every artifact is checked before collection starts copying', t => {
  const f = fixture(t);
  writeFileSync(join(f.bundle, 'macos', f.archive), 'changed');
  const result = f.run('collect-release.mjs', [f.stage]);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /checksum mismatch/);
  assert(!existsSync(f.stage));
});

test('mixed channel manifests and changed frozen legacy feed fail before merged output is written', t => {
  const f = fixture(t); f.collect();
  writeFileSync(join(f.stage, 'latest-aarch64.json'), json(f.feed));
  assert.match(f.run('merge-updater.mjs', [f.stage]).stderr, /Mixed updater channels/);
  assert(!existsSync(join(f.stage, 'latest-v2.json')));
  rmSync(join(f.stage, 'latest-aarch64.json'));
  writeFileSync(join(f.stage, 'latest.json'), '{}');
  assert.match(f.run('merge-updater.mjs', [f.stage]).stderr, /legacy feed hash/);
  assert(!existsSync(join(f.stage, 'latest-v2.json')));
});

test('signature, checksum and current-version mismatches stop draft validation', t => {
  const f = fixture(t); f.collect(); f.merge();
  const original = readFileSync(join(f.stage, 'latest-v2.json'));
  for (const feed of [{ ...f.feed, version: '0.3.1' }, { ...f.feed, platforms: { 'darwin-aarch64': { ...f.feed.platforms['darwin-aarch64'], signature: 'wrong-key-signature' } } }]) {
    writeFileSync(join(f.stage, 'latest-v2.json'), json(feed));
    assert.throws(() => verifiedRelease(f.stage, f.config, f.policy), /version|signature/);
  }
  writeFileSync(join(f.stage, 'latest-v2.json'), original);
  writeFileSync(join(f.stage, 'SHA256SUMS'), 'bad checksums');
  assert.throws(() => verifiedRelease(f.stage, f.config, f.policy), /Checksum manifest/);
});

test('stale artifacts and symlink feeds cannot be uploaded or overwritten', t => {
  const f = fixture(t); f.collect(); f.merge();
  writeFileSync(join(f.stage, 'old.app.tar.gz'), 'old artifact');
  assert.match(f.run('draft-release.mjs', [f.stage, '--check']).stderr, /Unexpected existing release file/);
  rmSync(join(f.stage, 'old.app.tar.gz'));
  const target = join(f.root, 'untouched.json'); writeFileSync(target, legacy);
  rmSync(join(f.stage, 'latest.json')); symlinkSync(target, join(f.stage, 'latest.json'));
  assert.throws(() => verifiedRelease(f.stage, f.config, f.policy), /regular release file/);
  assert.match(f.run('merge-updater.mjs', [f.stage]).stderr, /regular release file/);
  assert.deepEqual(readFileSync(target), legacy);
});

test('manual release preflight never falls back to the legacy local private key', { skip: process.platform !== 'darwin' }, t => {
  const f = fixture(t);
  const env = { ...process.env };
  for (const name of ['TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PATH', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD']) delete env[name];
  const guard = join(f.root, 'guard.mjs');
  writeFileSync(guard, `import cp from 'node:child_process'; import {syncBuiltinESMExports} from 'node:module'; cp.spawnSync=()=>{throw Error('Unexpected external process');}; syncBuiltinESMExports();`);
  const result = spawnSync(process.execPath, ['--import', guard, join(f.root, 'scripts/release-macos.mjs'), '--signed', '--check'], { cwd: f.root, env, encoding: 'utf8' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /explicit new signing key/);
  assert.doesNotMatch(result.stderr, /Unexpected external process/);
});

test('the actual Draft command uploads both feeds and only verified public files', t => {
  const f = fixture(t); f.collect(); f.merge();
  const capture = join(f.root, 'gh-args.json');
  const guard = join(f.root, 'capture.mjs');
  writeFileSync(guard, `import cp from 'node:child_process'; import {syncBuiltinESMExports} from 'node:module'; import {writeFileSync} from 'node:fs'; cp.spawnSync=(command,args)=>{if(command!=='gh')throw Error('Unexpected process'); writeFileSync(${JSON.stringify(capture)}, JSON.stringify(args)); return {status:0};}; syncBuiltinESMExports();`);
  const invoke = () => spawnSync(process.execPath, ['--import', guard, join(f.root, 'scripts/draft-release.mjs'), f.stage], { cwd: f.root, encoding: 'utf8', env: { ...process.env, GITHUB_SHA: 'a'.repeat(40) } });
  const result = invoke();
  assert.equal(result.status, 0, result.stderr);
  const args = JSON.parse(readFileSync(capture));
  assert(args.includes('--draft'));
  assert.equal(args[args.indexOf('--target') + 1], 'a'.repeat(40));
  assert.deepEqual(args.filter(arg => arg.startsWith(f.stage + '/')).sort(), [...verifiedRelease(f.stage, f.config, f.policy).files.keys()].map(file => join(f.stage, file)).sort());
  assert(args.includes(join(f.stage, 'latest.json')) && args.includes(join(f.stage, 'latest-v2.json')));
  rmSync(capture);
  writeFileSync(join(f.stage, 'latest.json'), '{}');
  assert.notEqual(invoke().status, 0);
  assert(!existsSync(capture), 'GitHub was invoked despite invalid legacy bytes');
});
