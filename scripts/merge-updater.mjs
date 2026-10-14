import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { mergeManifests } from './release-utils.mjs';
const folder=process.argv[2];
if (!folder) throw new Error('Artifact folder required.');
const values=readdirSync(folder).filter(f=>/^latest-(aarch64|x86_64)\.json$/.test(f)).map(f=>JSON.parse(readFileSync(join(folder,f),'utf8')));
writeFileSync(join(folder,'latest.json'),JSON.stringify(mergeManifests(values),null,2)+'\n');
