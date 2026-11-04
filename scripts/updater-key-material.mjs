import { lstatSync, mkdirSync, readFileSync, writeFileSync, chmodSync, rmSync, existsSync, renameSync, openSync, fsyncSync, closeSync } from 'node:fs';
import { join, isAbsolute } from 'node:path';
import { timingSafeEqual, randomUUID } from 'node:crypto';
import { publicKeyFingerprint, compareVersions, modernEndpoint, validateUpdaterTrust } from './updater-trust.mjs';
import { checksum } from './release-utils.mjs';
import { frozenLegacyFeed } from './release-feeds.mjs';

function owned(path, directory, mode) {
  const stat = lstatSync(path);
  if ((directory ? !stat.isDirectory() : !stat.isFile()) || stat.uid !== process.getuid() || (stat.mode & 0o777) !== mode || (!directory && stat.nlink !== 1)) throw new Error('Key material must be owner-only, regular, and unlinked.');
}
function exclusive(path, bytes) { writeFileSync(path, bytes, { flag: 'wx', mode: 0o600, flush: true }); }
function syncDirectory(path) { const fd = openSync(path, 'r'); try { fsyncSync(fd); } finally { closeSync(fd); } }
function sameKey(a, b) {
  const left = readFileSync(a), right = readFileSync(b);
  if (left.length !== right.length || !timingSafeEqual(left, right)) throw new Error('Local backup differs from the prepared key.');
}
export function inspectKey(directory, legacyPublicKey) {
  owned(directory, true, 0o700);
  owned(join(directory, 'backup'), true, 0o700);
  for (const file of ['updater.key', 'updater.key.pub', 'backup/updater.key', 'backup/updater.key.pub']) owned(join(directory, file), false, 0o600);
  const fingerprint = publicKeyFingerprint(readFileSync(join(directory, 'updater.key.pub'), 'utf8').trim());
  if (fingerprint === publicKeyFingerprint(legacyPublicKey)) throw new Error('Prepared key must differ from the legacy key.');
  sameKey(join(directory, 'updater.key'), join(directory, 'backup/updater.key'));
  sameKey(join(directory, 'updater.key.pub'), join(directory, 'backup/updater.key.pub'));
  return fingerprint;
}
export function prepareKey({ secretRoot, config, policy, legacyFeed, version, generate, prove }) {
  validateUpdaterTrust(config, policy);
  if (policy.mode !== 'legacy') throw new Error('Prepare the successor before activating manual migration.');
  if (compareVersions(version, config.version) <= 0) throw new Error('Candidate version must be newer than the current app.');
  frozenLegacyFeed(legacyFeed, { ...policy, legacyFeedSha256: checksum(legacyFeed) });
  if (!existsSync(secretRoot)) mkdirSync(secretRoot, { mode: 0o700 });
  owned(secretRoot, true, 0o700);
  const directory = join(secretRoot, 'updater-v2');
  // Exclusive reservation: existing material is never overwritten or regenerated.
  mkdirSync(directory, { mode: 0o700 });
  try {
    generate(join(directory, 'updater.key'));
    for (const file of ['updater.key', 'updater.key.pub']) {
      const path = join(directory, file);
      const stat = lstatSync(path);
      if (!stat.isFile() || stat.nlink !== 1 || stat.uid !== process.getuid()) throw new Error('Generator produced unsafe key material.');
      chmodSync(path, 0o600);
    }
    mkdirSync(join(directory, 'backup'), { mode: 0o700 });
    for (const file of ['updater.key', 'updater.key.pub']) exclusive(join(directory, 'backup', file), readFileSync(join(directory, file)));
    const fingerprint = inspectKey(directory, policy.publicKey);
    prove({ directory, version, legacyPublicKey: policy.publicKey });
    const key = readFileSync(join(directory, 'updater.key.pub'), 'utf8').trim();
    const candidatePolicy = { ...policy, schemaVersion: 2, mode: 'manual', legacyFeedSha256: checksum(legacyFeed), successor: { publicKey: key, endpoint: modernEndpoint, firstVersion: version } };
    const candidateConfig = structuredClone(config);
    candidateConfig.version = version;
    candidateConfig.plugins.updater.pubkey = key;
    candidateConfig.plugins.updater.endpoints = [modernEndpoint];
    validateUpdaterTrust(candidateConfig, candidatePolicy);
    const receipt = { schemaVersion: 1, version, fingerprint, localRestoreVerified: true, independentBackup: 'pending' };
    for (const [file, value] of [['candidate-tauri.conf.json', candidateConfig], ['candidate-updater-trust.json', candidatePolicy], ['preparation.json', receipt]]) exclusive(join(directory, file), JSON.stringify(value, null, 2) + '\n');
    return receipt;
  } catch (error) {
    // Only the directory exclusively created by THIS call can be removed here.
    rmSync(directory, { recursive: true, force: true });
    throw error;
  }
}
export function verifyPreparedKey({ secretRoot, policy, legacyFeed, prove }) {
  owned(secretRoot, true, 0o700);
  const directory = join(secretRoot, 'updater-v2');
  owned(directory, true, 0o700);
  for (const file of ['preparation.json', 'candidate-tauri.conf.json', 'candidate-updater-trust.json']) owned(join(directory, file), false, 0o600);
  const receipt = JSON.parse(readFileSync(join(directory, 'preparation.json')));
  if (receipt.schemaVersion !== 1 || receipt.localRestoreVerified !== true || !['pending', 'verified'].includes(receipt.independentBackup)) throw new Error('Invalid key preparation receipt.');
  if (receipt.independentBackup === 'verified' && (receipt.externalRestoreVerified !== true || !isAbsolute(receipt.backupFile ?? '') || receipt.encryption !== 'AES-256' || !/^[a-f0-9]{64}$/.test(receipt.backupSha256 ?? '') || !Number.isFinite(Date.parse(receipt.lastBackupVerifiedAt)))) throw new Error('Invalid backup verification receipt.');
  const fingerprint = inspectKey(directory, policy.publicKey);
  if (receipt.fingerprint !== fingerprint) throw new Error('Prepared key fingerprint differs from its receipt.');
  const config = JSON.parse(readFileSync(join(directory, 'candidate-tauri.conf.json')));
  const candidatePolicy = JSON.parse(readFileSync(join(directory, 'candidate-updater-trust.json')));
  frozenLegacyFeed(legacyFeed, candidatePolicy);
  const trust = validateUpdaterTrust(config, candidatePolicy);
  if (trust.fingerprint !== fingerprint || config.version !== receipt.version || candidatePolicy.publicKey !== policy.publicKey || candidatePolicy.baselineVersion !== policy.baselineVersion) throw new Error('Candidate configuration differs from the prepared key.');
  prove({ directory, version: receipt.version, legacyPublicKey: policy.publicKey });
  return receipt;
}
export function validateBackupDestination(secretRoot, destination, deviceOf = path => lstatSync(path).dev) {
  if (!isAbsolute(destination ?? '')) throw new Error('Backup destination must be an absolute path.');
  const target = lstatSync(destination);
  if (!target.isDirectory() || target.uid !== process.getuid()) throw new Error('Backup destination must be an owned directory, not a link.');
  if (deviceOf(destination) === deviceOf(secretRoot)) throw new Error('Independent backup requires a different filesystem.');
}
export function backupPreparedKey({ secretRoot, policy, legacyFeed, destination, prove, archive, deviceOf }) {
  validateBackupDestination(secretRoot, destination, deviceOf);
  const receipt = verifyPreparedKey({ secretRoot, policy, legacyFeed, prove });
  const directory = join(secretRoot, 'updater-v2');
  // The archive adapter writes only encrypted bytes to the external filesystem.
  const saved = archive({ directory, destination, receipt, prove, legacyPublicKey: policy.publicKey });
  if (saved.encryption !== 'AES-256' || saved.fingerprint !== receipt.fingerprint || saved.version !== receipt.version || saved.file !== join(destination, `ClamAV-updater-${receipt.fingerprint.slice(0,16)}.dmg`) || !/^[a-f0-9]{64}$/.test(saved.sha256 ?? '')) throw new Error('Invalid encrypted backup verification result.');
  const result = { ...receipt, independentBackup: 'verified', externalRestoreVerified: true, backupFile: saved.file, backupSha256: saved.sha256, encryption: saved.encryption, lastBackupVerifiedAt: new Date().toISOString() };
  const temporary = join(directory, `.preparation-${randomUUID()}.tmp`);
  try {
    exclusive(temporary, JSON.stringify(result, null, 2) + '\n');
    renameSync(temporary, join(directory, 'preparation.json'));
    syncDirectory(directory);
  } finally { if (existsSync(temporary)) rmSync(temporary); }
  return result;
}
