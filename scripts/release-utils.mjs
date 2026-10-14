import { createHash } from 'node:crypto';
export const repository = 'matsushibadenki/ClamAV-Desktop';
export function manifest(version, platform, file, signature, date = new Date().toISOString()) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Invalid release version');
  if (!['darwin-aarch64', 'darwin-x86_64'].includes(platform)) throw new Error('Invalid release platform');
  if (!/^[A-Za-z0-9._-]+\.app\.tar\.gz$/.test(file) || !signature.trim()) throw new Error('Invalid updater artifact');
  return { version, notes: `ClamAV Desktop ${version}`, pub_date: date, platforms: { [platform]: { signature: signature.trim(), url: `https://github.com/${repository}/releases/download/v${version}/${file}` } } };
}
export function mergeManifests(values) {
  if (!values.length) throw new Error('No platform manifests');
  const result = { ...values[0], platforms: {} };
  for (const value of values) {
    if (value.version !== result.version) throw new Error('Release versions differ');
    for (const [platform, artifact] of Object.entries(value.platforms)) {
      if (result.platforms[platform]) throw new Error('Duplicate release platform');
      const file = decodeURIComponent(new URL(artifact.url).pathname.split('/').pop());
      const checked = manifest(value.version, platform, file, artifact.signature, value.pub_date);
      if (checked.platforms[platform].url !== artifact.url) throw new Error('Untrusted release URL');
      result.platforms[platform] = artifact;
    }
  }
  return result;
}
export function checksum(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
