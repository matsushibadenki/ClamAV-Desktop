import { readFileSync } from 'node:fs';
import { validateUpdaterTrust } from './updater-trust.mjs';
import { frozenLegacyFeed, readRegular } from './release-feeds.mjs';
import { fileURLToPath } from 'node:url';

if (process.argv.length !== 2) throw new Error('This check uses the committed application config and trust policy; no overrides are accepted.');
const read = file => JSON.parse(readFileSync(new URL(`../src-tauri/${file}`, import.meta.url), 'utf8'));
const policy = read('updater-trust.json');
const result = validateUpdaterTrust(read('tauri.conf.json'), policy);
if (result.mode === 'manual') frozenLegacyFeed(readRegular(fileURLToPath(new URL('../src-tauri/updater-legacy.json', import.meta.url))), policy);
console.log(`Updater trust matches ${result.baselineVersion}; ${result.mode} / ${result.feedName}; key SHA-256 ${result.fingerprint}.`);
