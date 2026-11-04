import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, openSync, fsyncSync, closeSync, lstatSync } from 'node:fs';
import { join } from 'node:path';
import { timingSafeEqual } from 'node:crypto';
import { checksum } from './release-utils.mjs';

// Passwords go through stdin, never argv/environment/logs. Child output is private.
export function diskImageCommand(args, password) {
  const input = password ? Buffer.concat([password, Buffer.from([0])]) : undefined;
  try {
    const result = spawnSync('/usr/bin/hdiutil', args, { input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], timeout: 120000, maxBuffer: 1024 * 1024 });
    if (result.error || result.status !== 0) throw new Error('Encrypted disk-image operation failed.');
    return result.stdout;
  } finally { input?.fill(0); }
}
export function encryptedKeyBackup({ directory, destination, receipt, prove, legacyPublicKey, password, command = diskImageCommand }) {
  if (!Buffer.isBuffer(password) || password.length < 16 || password.includes(0) || password.includes(10) || password.includes(13)) throw new Error('A local backup passphrase is required.');
  const file = join(destination, `ClamAV-updater-${receipt.fingerprint.slice(0,16)}.dmg`);
  const existing = !!lstatSync(file, { throwIfNoEntry: false });
  if (existing && (!lstatSync(file).isFile() || receipt.backupFile !== file || checksum(readFileSync(file)) !== receipt.backupSha256)) throw new Error('Existing backup is not the previously verified encrypted image; it will not be overwritten.');
  const work = mkdtempSync(join(directory, '.encrypted-backup-'));
  const stage = join(work, 'stage'), mount = join(work, 'mount');
  mkdirSync(stage, { mode: 0o700 }); mkdirSync(mount, { mode: 0o700 });
  const files = ['updater.key', 'updater.key.pub', 'candidate-tauri.conf.json', 'candidate-updater-trust.json'];
  let attached = false;
  try {
    if (!existing) {
      for (const name of files) writeFileSync(join(stage, name), readFileSync(join(directory, name)), { flag: 'wx', mode: 0o600 });
      writeFileSync(join(stage, 'backup.json'), JSON.stringify({ fingerprint: receipt.fingerprint, version: receipt.version }) + '\n', { flag: 'wx', mode: 0o600 });
      command(['create', '-srcfolder', stage, '-format', 'UDZO', '-encryption', 'AES-256', '-stdinpass', '-volname', 'ClamAV Key Backup', '-o', file], password);
    }
    const encrypted = command(['isencrypted', file, '-plist']);
    if (!/<key>encrypted<\/key>\s*<true\s*\/>/i.test(encrypted)) throw new Error('Backup image is not encrypted.');
    command(['attach', file, '-readonly', '-noautoopen', '-mountpoint', mount, '-stdinpass'], password);
    attached = true;
    for (const name of files) {
      if (!lstatSync(join(mount, name)).isFile()) throw new Error('Unexpected backup file.');
      const expected = readFileSync(join(directory, name)), restored = readFileSync(join(mount, name));
      if (expected.length !== restored.length || !timingSafeEqual(expected, restored)) throw new Error('Encrypted backup content differs from prepared material.');
    }
    const info = JSON.parse(readFileSync(join(mount, 'backup.json')));
    if (info.fingerprint !== receipt.fingerprint || info.version !== receipt.version) throw new Error('Encrypted backup metadata mismatch.');
    prove({ directory, version: receipt.version, legacyPublicKey, restorePath: join(mount, 'updater.key') });
    command(['detach', mount]); attached = false;
    const fd = openSync(file, 'r+'); try { fsyncSync(fd); } finally { closeSync(fd); }
    return { file, sha256: checksum(readFileSync(file)), fingerprint: receipt.fingerprint, version: receipt.version, encryption: 'AES-256' };
  } finally {
    // An attach can fail after mounting. Check the filesystem boundary before cleanup.
    if (!attached && lstatSync(mount).dev !== lstatSync(work).dev) attached = true;
    if (attached) { command(['detach', mount]); attached = false; }
    // Never recurse into a volume whose detach failed.
    if (!attached) rmSync(work, { recursive: true, force: true });
  }
}

export async function readBackupPassword() {
  if (!process.stdin.isTTY) throw new Error('Backup passphrase must be entered locally in a terminal.');
  async function hidden(prompt) {
    process.stdout.write(prompt);
    process.stdin.setRawMode(true); process.stdin.resume();
    return await new Promise((resolve, reject) => {
      const bytes = [];
      const finish = (error) => {
        process.stdin.off('data', data); process.stdin.setRawMode(false); process.stdin.pause(); process.stdout.write('\n');
        if (error) { bytes.fill(0); reject(error); } else { const value = Buffer.from(bytes); bytes.fill(0); resolve(value); }
      };
      const data = input => {
        for (const byte of input) {
          if (byte === 3 || byte === 4) { finish(new Error('Local input cancelled.')); return; }
          if (byte === 10 || byte === 13) { finish(); return; }
          if (byte === 21) { bytes.fill(0); bytes.length = 0; }
          else if (byte === 127 || byte === 8) bytes.pop();
          else if (byte >= 32 && byte <= 126 && bytes.length < 1024) bytes.push(byte);
        }
      };
      process.stdin.on('data', data);
    });
  }
  const first = await hidden('Backup passphrase (16+ ASCII characters; input hidden): ');
  let second;
  try {
    second = await hidden('Confirm passphrase: ');
    if (first.length < 16 || first.length !== second.length || !timingSafeEqual(first, second)) throw new Error('Passphrases did not match.');
    return first;
  } catch (error) { first.fill(0); throw error; }
  finally { second?.fill(0); }
}
