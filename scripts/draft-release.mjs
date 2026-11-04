import { spawnSync } from 'node:child_process';
import { readJson, verifiedRelease, assertStagingFiles } from './release-feeds.mjs';
import { validateUpdaterTrust } from './updater-trust.mjs';
const config = readJson('src-tauri/tauri.conf.json');
const policy = readJson('src-tauri/updater-trust.json');
validateUpdaterTrust(config, policy);
const folder = process.argv[2];
const checkOnly = process.argv[3] === '--check';
if (!folder || process.argv.length > 4 || (process.argv[3] && !checkOnly) || !/^[a-f0-9]{40}$/.test(process.env.GITHUB_SHA ?? '')) throw new Error('A reviewed GitHub commit and artifact folder are required.');
if (readJson('package.json').version !== config.version) throw new Error('Release versions differ.');
const { files, trust, info } = verifiedRelease(folder, config, policy);
const arch = info.architecture === 'arm64' ? 'aarch64' : 'x86_64';
assertStagingFiles(folder, [...files.keys(), `${trust.manifestPrefix}${arch}.json`]);
if (checkOnly) {
  console.log(JSON.stringify({ version: config.version, mode: trust.mode, files: [...files.keys()].sort(), target: process.env.GITHUB_SHA }));
} else {
  const result = spawnSync('gh', ['release', 'create', `v${config.version}`, '--draft', '--target', process.env.GITHUB_SHA, '--title', `ClamAV Desktop ${config.version}`, '--notes', 'Signed and notarized macOS release. Review artifacts before publishing.', ...[...files.keys()].map(file => `${folder}/${file}`)], { stdio: 'inherit' });
  if (result.status !== 0) throw new Error('Draft creation failed; existing releases were not overwritten.');
}
