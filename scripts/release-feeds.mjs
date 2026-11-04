import { readFileSync, lstatSync, existsSync, readdirSync, mkdirSync, writeFileSync, renameSync, unlinkSync } from 'node:fs';
import { join, basename } from 'node:path';
import { randomUUID } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { checksum, mergeManifests } from './release-utils.mjs';
import { validateUpdaterTrust } from './updater-trust.mjs';

export const readJson = path => JSON.parse(readRegular(path).toString('utf8'));
export function readRegular(path) {
  if (!lstatSync(path).isFile()) throw new Error(`Expected a regular release file: ${basename(path)}`);
  return readFileSync(path);
}
export function trustReceipt(trust) {
  return { mode: trust.mode, fingerprint: trust.fingerprint, baselineVersion: trust.baselineVersion, legacyFeedSha256: trust.legacyFeedSha256 };
}
export function frozenLegacyFeed(bytes, policy) {
  if (!bytes || checksum(bytes) !== policy.legacyFeedSha256) throw new Error('Frozen legacy feed hash mismatch.');
  const feed = JSON.parse(bytes.toString('utf8'));
  checkManifest(feed, policy.baselineVersion);
  return bytes; // Preserve the exact reviewed bytes, not a JSON reserialization.
}
function checkManifest(feed, version) {
  if (feed?.version !== version || !feed.platforms || !Object.keys(feed.platforms).length) throw new Error('Feed version/platforms differ from the verified release.');
  mergeManifests([feed]);
}
export function createFeeds(config, policy, manifests, legacyBytes) {
  const trust = validateUpdaterTrust(config, policy);
  const feed = mergeManifests(manifests);
  checkManifest(feed, config.version);
  const result = new Map([[trust.feedName, Buffer.from(JSON.stringify(feed, null, 2) + '\n')]]);
  if (trust.mode === 'manual') result.set('latest.json', frozenLegacyFeed(legacyBytes, policy));
  return result;
}
export function assertStagingFiles(folder, allowedExisting) {
  if (existsSync(folder)) {
    if (!lstatSync(folder).isDirectory()) throw new Error('Release directory must not be a symlink.');
    for (const name of readdirSync(folder)) {
      const path = join(folder, name);
      if (lstatSync(path).isDirectory()) continue; // Archived previous releases stay private.
      if (!allowedExisting.includes(name) || !lstatSync(path).isFile()) throw new Error(`Unexpected existing release file: ${name}. Use a fresh staging directory.`);
    }
  }
}
export function writeReleaseFiles(folder, files, allowedExisting = [...files.keys()]) {
  for (const name of files.keys()) if (basename(name) !== name) throw new Error('Invalid release filename.');
  assertStagingFiles(folder, allowedExisting);
  mkdirSync(folder, { recursive: true });
  for (const [name, bytes] of files) {
    const temporary = join(folder, `.release-${randomUUID()}.tmp`);
    try {
      writeFileSync(temporary, bytes, { flag: 'wx', mode: 0o644 });
      renameSync(temporary, join(folder, name));
    } finally { if (existsSync(temporary)) unlinkSync(temporary); }
  }
}

// Validate the COMPLETE release before copying it or starting a GitHub process.
// A retired feed references its original public archive, not a newly uploaded copy.
export function verifiedRelease(folder, config, policy, { merged = true, bundle = false, legacyBytes } = {}) {
  const trust = validateUpdaterTrust(config, policy);
  const info = readJson(join(folder, 'release.json'));
  if (info.development || info.appNotarizationVerified !== true || info.dmgNotarizationVerified !== true || info.version !== config.version) throw new Error('Only this verified release can be uploaded.');
  if (trust.mode === 'manual' || info.updaterTrust) {
    if (!isDeepStrictEqual(info.updaterTrust, trustReceipt(trust))) throw new Error('Release trust receipt does not match the application configuration.');
  }
  const arch = { arm64: 'aarch64', x64: 'x86_64' }[info.architecture];
  if (!arch) throw new Error('Unsupported release architecture.');
  const archive = `ClamAV-Desktop_${config.version}_${arch}.app.tar.gz`;
  const dmg = `ClamAV-Desktop_${config.version}_${arch === 'aarch64' ? 'aarch64' : 'x64'}.dmg`;
  if (!Array.isArray(info.artifacts) || info.artifacts.length !== 2 || new Set(info.artifacts.map(a => a.file)).size !== 2 || !info.artifacts.every(a => [archive, dmg].includes(a.file))) throw new Error('Unexpected release artifact set.');
  const files = new Map();
  for (const artifact of info.artifacts) {
    const bytes = readRegular(bundle ? join(folder, '..', artifact.file === dmg ? 'dmg' : 'macos', artifact.file) : join(folder, artifact.file));
    if (checksum(bytes) !== artifact.sha256) throw new Error('Artifact checksum mismatch.');
    files.set(artifact.file, bytes);
  }
  const signature = readRegular(bundle ? join(folder, '..', 'macos', archive + '.sig') : join(folder, archive + '.sig'));
  files.set(archive + '.sig', signature);
  const feedFile = merged ? trust.feedName : `${trust.manifestPrefix}${arch}.json`;
  const feedBytes = readRegular(join(folder, feedFile));
  const feed = JSON.parse(feedBytes.toString('utf8'));
  checkManifest(feed, config.version);
  const platform = feed.platforms[`darwin-${arch}`];
  if (Object.keys(feed.platforms).length !== 1 || !platform || new URL(platform.url).pathname.split('/').pop() !== archive || platform.signature !== signature.toString('utf8').trim()) throw new Error('Feed signature/platform differs from the verified artifact.');
  files.set(feedFile, feedBytes);
  if (trust.mode === 'manual') files.set('latest.json', frozenLegacyFeed(legacyBytes ?? readRegular(join(folder, 'latest.json')), policy));
  const sums = readRegular(join(folder, 'SHA256SUMS'));
  const expectedSums = info.artifacts.map(a => `${a.sha256}  ${a.file}`).sort();
  if (!isDeepStrictEqual(sums.toString('utf8').trim().split('\n').sort(), expectedSums)) throw new Error('Checksum manifest differs from the verified artifacts.');
  files.set('SHA256SUMS', sums);
  files.set('release.json', readRegular(join(folder, 'release.json')));
  return { files, trust, info };
}
