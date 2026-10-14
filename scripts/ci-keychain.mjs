import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { writeFileSync, appendFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';
const folder = process.env.RUNNER_TEMP;
if (!folder) throw new Error('RUNNER_TEMP is required.');
const keychain = join(folder,'clamav-signing.keychain-db');
const certificate = join(folder,'clamav-certificate.p12');
const searchList = join(folder,'clamav-keychains.json');
const run = (command,args) => {
  const r=spawnSync(command,args,{encoding:'utf8',stdio:'pipe'});
  if (r.status!==0) throw new Error(`Signing setup failed: ${command} ${args[0]}`);
  return r.stdout || '';
};
if (process.argv.includes('--cleanup')) {
  if (existsSync(searchList)) run('/usr/bin/security',['list-keychains','-d','user','-s',...JSON.parse(readFileSync(searchList,'utf8'))]);
  if (existsSync(keychain)) run('/usr/bin/security',['delete-keychain',keychain]);
  for (const file of [certificate,searchList]) rmSync(file,{force:true});
} else {
  for (const name of ['APPLE_CERTIFICATE','APPLE_CERTIFICATE_PASSWORD','APPLE_ID','APPLE_PASSWORD','APPLE_TEAM_ID']) if (!process.env[name]) throw new Error(`Required release Secret is missing: ${name}`);
  const old=[...run('/usr/bin/security',['list-keychains','-d','user']).matchAll(/"([^"]+)"/g)].map(m=>m[1]);
  writeFileSync(searchList,JSON.stringify(old),{mode:0o600});
  writeFileSync(certificate,Buffer.from(process.env.APPLE_CERTIFICATE,'base64'),{mode:0o600});
  const password=randomBytes(32).toString('hex');
  run('/usr/bin/security',['create-keychain','-p',password,keychain]);
  run('/usr/bin/security',['unlock-keychain','-p',password,keychain]);
  run('/usr/bin/security',['set-keychain-settings','-lut','21600',keychain]);
  run('/usr/bin/security',['list-keychains','-d','user','-s',...old,keychain]);
  run('/usr/bin/security',['import',certificate,'-k',keychain,'-P',process.env.APPLE_CERTIFICATE_PASSWORD,'-T','/usr/bin/codesign']);
  run('/usr/bin/security',['set-key-partition-list','-S','apple-tool:,apple:,codesign:','-s','-k',password,keychain]);
  run('/usr/bin/xcrun',['notarytool','store-credentials','clamav-desktop','--keychain',keychain,'--apple-id',process.env.APPLE_ID,'--team-id',process.env.APPLE_TEAM_ID,'--password',process.env.APPLE_PASSWORD]);
  rmSync(certificate,{force:true});
  appendFileSync(process.env.GITHUB_ENV, `APPLE_NOTARY_KEYCHAIN=${keychain}\n`);
  console.log('Temporary signing keychain and notarization profile are ready.');
}
