import { mkdirSync, readFileSync, copyFileSync, existsSync } from 'node:fs';
import { join, basename } from 'node:path';
import { checksum } from './release-utils.mjs';
const bundle='src-tauri/target/release/bundle';
const verify=join(bundle,'verification');
const info=JSON.parse(readFileSync(join(verify,'release.json'),'utf8'));
if (info.development || !info.appNotarizationVerified || !info.dmgNotarizationVerified) throw new Error('Release is not verified for distribution.');
mkdirSync('release-output',{recursive:true});
for (const entry of info.artifacts) {
  if (entry.file !== basename(entry.file) || !(/\.dmg$/.test(entry.file) || /\.app\.tar\.gz$/.test(entry.file))) throw new Error('Unexpected release filename.');
  const source=join(bundle,entry.file.endsWith('.dmg')?'dmg':'macos',entry.file);
  if (checksum(readFileSync(source))!==entry.sha256) throw new Error('Release checksum mismatch.');
  copyFileSync(source,join('release-output',entry.file));
  if (entry.file.endsWith('.app.tar.gz')) copyFileSync(source+'.sig',join('release-output',entry.file+'.sig'));
}
for (const file of ['SHA256SUMS','release.json',`latest-${info.architecture==='arm64'?'aarch64':'x86_64'}.json`]) copyFileSync(join(verify,file),join('release-output',file));
console.log('Only verified public release artifacts were collected.');
