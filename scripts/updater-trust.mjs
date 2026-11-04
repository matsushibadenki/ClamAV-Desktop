import { createHash } from 'node:crypto';

export const legacyEndpoint = 'https://github.com/matsushibadenki/ClamAV-Desktop/releases/latest/download/latest.json';
export const modernEndpoint = legacyEndpoint.replace('latest.json', 'latest-v2.json');

function decodeBase64(value) {
  if (typeof value !== 'string' || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value) || !value) {
    throw new Error('Invalid updater public key encoding.');
  }
  const bytes = Buffer.from(value, 'base64');
  if (bytes.toString('base64') !== value) throw new Error('Invalid updater public key encoding.');
  return bytes;
}

export function publicKeyFingerprint(value) {
  const lines = decodeBase64(value).toString('utf8').trim().split(/\r?\n/);
  if (lines.length !== 2 || !lines[0].startsWith('untrusted comment: ')) throw new Error('Invalid minisign public key.');
  const packet = decodeBase64(lines[1]);
  if (packet.length !== 42 || packet.subarray(0, 2).toString('ascii') !== 'Ed') throw new Error('Invalid minisign public key.');
  // Pin the entire algorithm/key-ID/public-key packet, not the untrusted comment.
  return createHash('sha256').update(packet).digest('hex');
}

function versionParts(version) {
  if (typeof version !== 'string' || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) throw new Error('Expected a stable release version.');
  return version.split('.').map(BigInt);
}
export function compareVersions(a, b) {
  const left = versionParts(a), right = versionParts(b);
  for (let i = 0; i < 3; i++) if (left[i] !== right[i]) return left[i] > right[i] ? 1 : -1;
  return 0;
}

export function validateUpdaterTrust(config, policy) {
  const legacy = policy?.schemaVersion === 1 && policy.mode === 'legacy';
  const manual = policy?.schemaVersion === 2 && policy.mode === 'manual';
  if (!legacy && !manual) throw new Error('Unsupported updater trust policy. Review the migration plan before changing modes.');
  if (policy.endpoint !== legacyEndpoint) throw new Error('Legacy clients require the pinned GitHub latest.json endpoint.');
  if (compareVersions(config?.version, policy.baselineVersion) < 0) throw new Error('Release version predates the updater trust baseline.');
  const oldFingerprint = publicKeyFingerprint(policy.publicKey);
  let fingerprint = oldFingerprint;
  let endpoint = policy.endpoint;
  if (manual) {
    if (!/^[a-f0-9]{64}$/.test(policy.legacyFeedSha256 ?? '')) throw new Error('Manual migration requires a pinned legacy feed hash.');
    if (policy.successor?.endpoint !== modernEndpoint) throw new Error('Manual migration requires the dedicated latest-v2.json endpoint.');
    if (compareVersions(policy.successor.firstVersion, policy.baselineVersion) <= 0 || compareVersions(config.version, policy.successor.firstVersion) < 0) throw new Error('Invalid manual migration version boundary.');
    fingerprint = publicKeyFingerprint(policy.successor.publicKey);
    if (fingerprint === oldFingerprint) throw new Error('Manual migration requires a different public key.');
    endpoint = modernEndpoint;
  }
  const updater = config?.plugins?.updater;
  if (updater?.requireSignedVersion !== true) throw new Error('Signed-version verification must remain enabled.');
  if (!Array.isArray(updater.endpoints) || updater.endpoints.length !== 1 || updater.endpoints[0] !== endpoint) {
    throw new Error('Updater endpoint changed without a compatible bridge. See docs/updater-key-migration.md.');
  }
  if (publicKeyFingerprint(updater.pubkey) !== fingerprint) {
    throw new Error('Updater public key changed without a compatible bridge. See docs/updater-key-migration.md.');
  }
  return { baselineVersion: policy.baselineVersion, fingerprint, mode: policy.mode,
    feedName: manual ? 'latest-v2.json' : 'latest.json',
    manifestPrefix: manual ? 'latest-v2-' : 'latest-',
    legacyFeedSha256: manual ? policy.legacyFeedSha256 : null };
}
