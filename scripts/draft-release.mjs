import { spawnSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { checksum, mergeManifests } from './release-utils.mjs';
const folder=process.argv[2];
if (!folder || !process.env.GITHUB_SHA || !/^[a-f0-9]{40}$/.test(process.env.GITHUB_SHA)) throw new Error('A reviewed GitHub commit is required.');
const version=JSON.parse(readFileSync('package.json','utf8')).version;
const info=JSON.parse(readFileSync(join(folder,'release.json'),'utf8'));
const feed=JSON.parse(readFileSync(join(folder,'latest.json'),'utf8'));
mergeManifests([feed]);
for (const artifact of Object.values(feed.platforms)) {
  const file=decodeURIComponent(new URL(artifact.url).pathname.split('/').pop());
  if (readFileSync(join(folder,file+'.sig'),'utf8').trim() !== artifact.signature) throw new Error('Feed signature differs from the verified file.');
}
if (info.development || !info.appNotarizationVerified || !info.dmgNotarizationVerified || info.version!==version || feed.version!==version) throw new Error('Only this verified release can be uploaded.');
for (const artifact of info.artifacts) if (checksum(readFileSync(join(folder,artifact.file)))!==artifact.sha256) throw new Error('Artifact checksum mismatch.');
const files=readdirSync(folder).filter(f=>f.endsWith('.dmg')||f.endsWith('.app.tar.gz')||f.endsWith('.sig')||['latest.json','SHA256SUMS','release.json'].includes(f));
const r=spawnSync('gh',['release','create',`v${version}`,'--draft','--target',process.env.GITHUB_SHA,'--title',`ClamAV Desktop ${version}`,'--notes','Signed and notarized macOS release. Review artifacts before publishing.',...files.map(f=>join(folder,f))],{stdio:'inherit'});
if (r.status!==0) throw new Error('Draft creation failed; existing releases were not overwritten.');
