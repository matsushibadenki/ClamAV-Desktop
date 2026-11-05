import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdtempSync, chmodSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { prepareKey, verifyPreparedKey, backupPreparedKey, validateBackupDestination } from './updater-key-material.mjs';
import { encryptedKeyBackup, readBackupPassword } from './encrypted-key-backup.mjs';
import { readJson, readRegular } from './release-feeds.mjs';
import { validateUpdaterTrust } from './updater-trust.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const args = process.argv.slice(2);
if (args.length === 1 && args[0] === '--help') {
  console.log('prepare-updater-key.mjs --generate --version X.Y.Z | --verify | --backup ABSOLUTE_DIRECTORY\nBackup creates an AES-256 encrypted image on another filesystem and prompts locally for its passphrase. Never overwrites existing keys.');
  process.exit(0);
}
const generate = args.length === 3 && args[0] === '--generate' && args[1] === '--version';
const verify = args.length === 1 && args[0] === '--verify';
const backup = args.length === 2 && args[0] === '--backup';
if ((!generate && !verify && !backup) || process.platform !== 'darwin' || Number(process.versions.node.split('.')[0]) < 22) {
  console.error('Use Node.js 22+ on macOS: --generate --version X.Y.Z, --verify, or --backup ABSOLUTE_DIRECTORY.');
  process.exit(1);
}
// Captured CLI output may contain PRIVATE keys. Never forward it, including on failure.
const environment = { ...process.env, CI: 'true', CARGO_BUILD_JOBS: '2', CARGO_PROFILE_DEV_DEBUG: '0', CARGO_INCREMENTAL: '0', CARGO_TARGET_DIR: join(root, 'src-tauri/target') };
for (const key of Object.keys(environment)) if (key.startsWith('APPLE_') || key.startsWith('TAURI_SIGNING_')) delete environment[key];
function step(command, argv, env = environment) {
  const result = spawnSync(command, argv, { cwd: root, env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 4 * 1024 * 1024 });
  return !result.error && result.status === 0;
}
const cli = join(root, 'node_modules/.bin/tauri');
const verifier = join(root, 'src-tauri/target/debug/examples/verify_update_signature');
function prove({ directory, version, legacyPublicKey, restorePath }) {
  const probe = mkdtempSync(join(directory, '.probe-'));
  chmodSync(probe, 0o700);
  const payload = join(probe, 'key-check.txt');
  const restored = join(probe, 'restored.key');
  const legacy = join(probe, 'legacy.pub');
  const publicKey = join(directory, 'updater.key.pub');
  const bytes = Buffer.from('ClamAV Desktop offline key preparation; not an application archive.\n');
  const put = (path, value) => writeFileSync(path, value, { flag: 'wx', mode: 0o600 });
  try {
    put(payload, bytes);
    put(restored, readFileSync(restorePath ?? join(directory, 'backup/updater.key')));
    put(legacy, legacyPublicKey);
    for (const key of [join(directory, 'updater.key'), restored]) {
      const env = { ...environment, TAURI_SIGNING_PRIVATE_KEY_PATH: key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '' };
      if (!step(cli, ['signer', 'sign', '--app-version', version, payload], env)) throw new Error('Key proof failed.');
      const signature = payload + '.sig';
      if (!step(verifier, [payload, signature, publicKey, version])) throw new Error('Key proof failed.');
      if (step(verifier, [payload, signature, legacy, version])) throw new Error('Legacy key unexpectedly accepted the new signature.');
      if (step(verifier, [payload, signature, publicKey, version === '0.0.0' ? '1.0.0' : '0.0.0'])) throw new Error('Signed-version mismatch was accepted.');
      writeFileSync(payload, Buffer.from('changed probe\n'), { mode: 0o600 });
      if (step(verifier, [payload, signature, publicKey, version])) throw new Error('Changed payload was accepted.');
      writeFileSync(payload, bytes, { mode: 0o600 });
    }
  } finally { rmSync(probe, { recursive: true, force: true }); }
}

let stage = 'verifier-build';
try {
  // Compile the public-only verifier before generating any long-lived private material.
  if (!step('cargo', ['build', '--quiet', '--manifest-path', 'src-tauri/Cargo.toml', '--example', 'verify_update_signature', '--locked', '--offline'])) throw new Error('Verifier build failed.');
  stage = 'active-trust-check';
  const config = readJson(join(root, 'src-tauri/tauri.conf.json'));
  const policy = readJson(join(root, 'src-tauri/updater-trust.json'));
  const activeTrust = validateUpdaterTrust(config, policy);
  const legacyFeed = readRegular(join(root, 'src-tauri/updater-legacy.json'));
  const secretRoot = join(root, '.secrets');
  let receipt;
  if (backup) {
    stage = 'backup-destination';
    validateBackupDestination(secretRoot, args[1]);
    stage = 'password-input';
    const password = await readBackupPassword();
    try {
      stage = 'encrypted-backup';
      receipt = backupPreparedKey({ secretRoot, policy, legacyFeed, prove, destination: args[1], archive: options => encryptedKeyBackup({ ...options, password }) });
    } finally { password.fill(0); }
  } else {
    stage = generate ? 'key-generation' : 'local-key-proof';
    receipt = generate ? prepareKey({ secretRoot, config, policy, legacyFeed, version: args[2], prove, generate: path => {
    if (!step(cli, ['signer', 'generate', '--ci', '--password', '', '--write-keys', path])) throw new Error('Generation failed.');
  } }) : verifyPreparedKey({ secretRoot, policy, legacyFeed, prove });
  }
  console.log(JSON.stringify({ schemaVersion: receipt.schemaVersion, version: receipt.version, fingerprint: receipt.fingerprint, localRestoreVerified: receipt.localRestoreVerified, independentBackup: receipt.independentBackup, externalRestoreVerified: receipt.externalRestoreVerified ?? false, lastBackupVerifiedAt: receipt.lastBackupVerifiedAt ?? null, privateDirectory: '.secrets/updater-v2', activeConfigurationUsesPreparedKey: activeTrust.fingerprint === receipt.fingerprint }, null, 2));
} catch {
  // Even a third-party tool error may contain secret output. Emit fixed text only.
  console.error(`Key preparation/verification failed [${stage}]. Private output was suppressed. Existing keys are never overwritten; use --verify for previously prepared material.`);
  if (stage === 'backup-destination') console.error('Connect the external volume and check that the backup directory exists on a different filesystem, is owned by this account, and is not a link. No password was requested and no backup was written.');
  process.exit(1);
}
