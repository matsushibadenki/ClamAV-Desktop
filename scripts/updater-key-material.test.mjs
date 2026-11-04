import {test} from 'node:test';
import assert from 'node:assert/strict';
import {generateKeyPairSync} from 'node:crypto';
import {mkdtempSync,mkdirSync,readFileSync,writeFileSync,chmodSync,lstatSync,existsSync,rmSync,symlinkSync,linkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {prepareKey,verifyPreparedKey,backupPreparedKey} from './updater-key-material.mjs';
import {spawnSync} from 'node:child_process';
import {copyFileSync} from 'node:fs';
const read=file=>JSON.parse(readFileSync(new URL(`../src-tauri/${file}`,import.meta.url)));
const config=read('tauri.conf.json'),policy=read('updater-trust.json');
const legacyFeed=readFileSync(new URL('../src-tauri/updater-legacy.json',import.meta.url));
function fixture(t){
 const root=mkdtempSync(join(tmpdir(),'clamav-key-material-'));
 t.after(()=>rmSync(root,{recursive:true,force:true}));
 const secretRoot=join(root,'.secrets');mkdirSync(secretRoot,{mode:0o700});
 const dir=join(secretRoot,'updater-v2');
 const {publicKey}=generateKeyPairSync('ed25519');
 const packet=Buffer.from(Buffer.from(policy.publicKey,'base64').toString().trim().split('\n')[1],'base64');
 publicKey.export({type:'spki',format:'der'}).subarray(-32).copy(packet,10);
 const pub=Buffer.from('untrusted comment: fixture\n'+packet.toString('base64')+'\n').toString('base64');
 let generated=0,proved=0;
 const options={secretRoot,config,policy,legacyFeed,version:'0.4.0',generate:path=>{generated++;writeFileSync(path,'private-fixture-do-not-log');writeFileSync(path+'.pub',pub);},prove:()=>{proved++;}};
 return {root,secretRoot,dir,options,generated:()=>generated,proved:()=>proved};
}
test('key preparation is private, makes a local restore copy and leaves active config unchanged',t=>{
 const f=fixture(t);const original=JSON.stringify(config);
 const receipt=prepareKey(f.options);
 assert.equal(receipt.independentBackup,'pending');assert.equal(receipt.localRestoreVerified,true);
 assert.equal(f.proved(),1);assert.equal(JSON.stringify(config),original);
 for(const dir of [f.secretRoot,f.dir,join(f.dir,'backup')])assert.equal(lstatSync(dir).mode&0o777,0o700);
 for(const file of ['updater.key','updater.key.pub','backup/updater.key','backup/updater.key.pub','preparation.json','candidate-tauri.conf.json','candidate-updater-trust.json'])assert.equal(lstatSync(join(f.dir,file)).mode&0o777,0o600);
 assert.deepEqual(readFileSync(join(f.dir,'updater.key')),readFileSync(join(f.dir,'backup/updater.key')));
 assert(!JSON.stringify(receipt).includes('private-fixture'));
 const verified=verifyPreparedKey(f.options);assert.deepEqual(verified,receipt);assert.equal(f.proved(),2);
});
test('existing keys are never overwritten even when a new generation is requested',t=>{
 const f=fixture(t);prepareKey(f.options);const original=readFileSync(join(f.dir,'updater.key'));
 assert.throws(()=>prepareKey(f.options),/EEXIST/);
 assert.equal(f.generated(),1);assert.deepEqual(readFileSync(join(f.dir,'updater.key')),original);
});
test('failed proof removes only this incomplete candidate and retains unrelated legacy material',t=>{
 const f=fixture(t);const old=join(f.secretRoot,'updater.key');writeFileSync(old,'existing-legacy-key',{mode:0o600});
 assert.throws(()=>prepareKey({...f.options,prove:()=>{throw Error('proof refused');}}),/proof refused/);
 assert(!existsSync(f.dir));assert.equal(readFileSync(old,'utf8'),'existing-legacy-key');
});
test('backup mismatch and unsafe permissions stop before proof and do not repair or replace data',t=>{
 const f=fixture(t);prepareKey(f.options);
 const backup=join(f.dir,'backup/updater.key');writeFileSync(backup,'changed backup');
 assert.throws(()=>verifyPreparedKey(f.options),/backup differs/);assert.equal(f.proved(),1);
 writeFileSync(backup,'private-fixture-do-not-log');chmodSync(backup,0o644);
 assert.throws(()=>verifyPreparedKey(f.options),/owner-only/);assert.equal(lstatSync(backup).mode&0o777,0o644);
});
test('symlink key directories and hard-linked key files are refused',t=>{
 const f=fixture(t);const other=join(f.root,'other');mkdirSync(other,{mode:0o700});symlinkSync(other,f.dir);
 assert.throws(()=>prepareKey(f.options),/EEXIST/);assert.equal(f.generated(),0);
 rmSync(f.dir);prepareKey(f.options);linkSync(join(f.dir,'updater.key'),join(f.root,'key-alias'));
 assert.throws(()=>verifyPreparedKey(f.options),/unlinked/);
});
test('candidate configuration must agree with the key, legacy baseline and frozen feed',t=>{
 const f=fixture(t);prepareKey(f.options);
 const path=join(f.dir,'candidate-tauri.conf.json');const c=JSON.parse(readFileSync(path));c.plugins.updater.pubkey=policy.publicKey;writeFileSync(path,JSON.stringify(c));
 assert.throws(()=>verifyPreparedKey(f.options),/public key changed/);
});
test('invalid versions or legacy feed do not create a key directory',t=>{
 const f=fixture(t);
 assert.throws(()=>prepareKey({...f.options,version:config.version}),/newer/);
 assert.throws(()=>prepareKey({...f.options,legacyFeed:Buffer.from('{}')}),/Feed version/);
 assert.equal(f.generated(),0);assert(!existsSync(f.dir));
});
test('CLI failures never forward private generator stdout or stderr', {skip:process.platform!=='darwin'}, t=>{
 const root=mkdtempSync(join(tmpdir(),'clamav-key-cli-'));
 t.after(()=>rmSync(root,{recursive:true,force:true}));
 mkdirSync(join(root,'scripts'));mkdirSync(join(root,'src-tauri'));
 for(const file of ['prepare-updater-key.mjs','updater-key-material.mjs','encrypted-key-backup.mjs','updater-trust.mjs','release-feeds.mjs','release-utils.mjs'])copyFileSync(new URL(file,import.meta.url),join(root,'scripts',file));
 writeFileSync(join(root,'src-tauri/tauri.conf.json'),JSON.stringify(config));
 writeFileSync(join(root,'src-tauri/updater-trust.json'),JSON.stringify(policy));
 writeFileSync(join(root,'src-tauri/updater-legacy.json'),legacyFeed);
 const guard=join(root,'guard.mjs');
 writeFileSync(guard,`import cp from 'node:child_process';import {syncBuiltinESMExports} from 'node:module';cp.spawnSync=(cmd)=>cmd==='cargo'?{status:0}:{status:1,stdout:'PRIVATE_SENTINEL',stderr:'PRIVATE_SENTINEL'};syncBuiltinESMExports();`);
 const result=spawnSync(process.execPath,['--import',guard,join(root,'scripts/prepare-updater-key.mjs'),'--generate','--version','0.4.0'],{encoding:'utf8'});
 assert.notEqual(result.status,0);assert.match(result.stderr,/Private output was suppressed/);
 assert(!result.stdout.includes('PRIVATE_SENTINEL')&&!result.stderr.includes('PRIVATE_SENTINEL'));
 assert(!existsSync(join(root,'.secrets/updater-v2')));
});
test('backup receipt changes only after a verified encrypted archive on a separate filesystem',t=>{
 const f=fixture(t);const receipt=prepareKey(f.options);
 const destination=join(f.root,'external');mkdirSync(destination,{mode:0o700});
 const archive=()=>({file:join(destination,`ClamAV-updater-${receipt.fingerprint.slice(0,16)}.dmg`),sha256:'a'.repeat(64),encryption:'AES-256',fingerprint:receipt.fingerprint,version:receipt.version});
 assert.throws(()=>backupPreparedKey({...f.options,destination,archive}),/different filesystem/);
 const device=lstatSync(f.secretRoot).dev;
 const options={...f.options,destination,archive,deviceOf:path=>path===destination?device+1:device};
 assert.throws(()=>backupPreparedKey({...options,archive:()=>{throw Error('encrypted proof refused');}}),/encrypted proof refused/);
 assert.equal(JSON.parse(readFileSync(join(f.dir,'preparation.json'))).independentBackup,'pending');
 assert.throws(()=>backupPreparedKey({...options,archive:()=>({...archive(),encryption:'none'})}),/Invalid encrypted backup/);
 const result=backupPreparedKey(options);assert.equal(result.independentBackup,'verified');assert.equal(result.externalRestoreVerified,true);
 assert.equal(verifyPreparedKey(f.options).backupSha256,'a'.repeat(64));
});
