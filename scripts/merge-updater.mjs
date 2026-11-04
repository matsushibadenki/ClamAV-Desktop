import { readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { readJson, readRegular, createFeeds, frozenLegacyFeed, writeReleaseFiles } from './release-feeds.mjs';
import { validateUpdaterTrust } from './updater-trust.mjs';
const config = readJson('src-tauri/tauri.conf.json');
const policy = readJson('src-tauri/updater-trust.json');
const trust = validateUpdaterTrust(config, policy);
const folder = process.argv[2];
if (!folder || process.argv.length !== 3) throw new Error('Artifact folder required.');
const names = readdirSync(folder);
const sources = names.filter(name => /^latest-(?:v2-)?(?:aarch64|x86_64)\.json$/.test(name));
if (sources.some(name => !name.startsWith(trust.manifestPrefix) || (trust.mode === 'legacy' && name.startsWith('latest-v2-')))) throw new Error('Mixed updater channels in staging.');
const manifests = sources.map(name => {
  const value = readJson(join(folder, name));
  const platform = 'darwin-' + name.slice(trust.manifestPrefix.length, -5);
  if (Object.keys(value.platforms ?? {}).length !== 1 || !value.platforms[platform]) throw new Error('Per-platform manifest filename mismatch.');
  return value;
});
const legacy = trust.mode === 'manual' ? readRegular('src-tauri/updater-legacy.json') : undefined;
if (legacy && existsSync(join(folder, 'latest.json'))) frozenLegacyFeed(readRegular(join(folder, 'latest.json')), policy);
const feeds = createFeeds(config, policy, manifests, legacy);
const allowed = [...sources, ...feeds.keys(), 'release.json', 'SHA256SUMS'];
for (const arch of ['aarch64', 'x86_64']) {
  allowed.push(`ClamAV-Desktop_${config.version}_${arch}.app.tar.gz`, `ClamAV-Desktop_${config.version}_${arch}.app.tar.gz.sig`, `ClamAV-Desktop_${config.version}_${arch === 'aarch64' ? 'aarch64' : 'x64'}.dmg`);
}
writeReleaseFiles(folder, feeds, allowed);
console.log(`Prepared ${trust.mode} updater feed${legacy ? 's' : ''}.`);
