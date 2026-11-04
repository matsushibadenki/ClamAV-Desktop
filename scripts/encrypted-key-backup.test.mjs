import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,readFileSync,writeFileSync,readdirSync,rmSync,existsSync,symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {encryptedKeyBackup} from './encrypted-key-backup.mjs';
function fixture(t){
 const root=mkdtempSync(join(tmpdir(),'clamav-encrypted-backup-'));
 t.after(()=>rmSync(root,{recursive:true,force:true}));
 const directory=join(root,'private'),destination=join(root,'external');
 mkdirSync(directory,{mode:0o700});mkdirSync(destination);
 const files=['updater.key','updater.key.pub','candidate-tauri.conf.json','candidate-updater-trust.json'];
 for(const file of files)writeFileSync(join(directory,file),`private-fixture-${file}`,{mode:0o600});
 const receipt={fingerprint:'e'.repeat(64),version:'0.4.0'};
 const password=Buffer.from('test-only-passphrase-never-production');
 const calls=[];let contents;
 const command=(args,pass)=>{
  calls.push(args);
  if(args[0]==='create'){
   assert.equal(args[args.indexOf('-encryption')+1],'AES-256');assert(args.includes('-stdinpass'));assert(!args.includes(password.toString()));assert.deepEqual(pass,password);
   const stage=args[args.indexOf('-srcfolder')+1];contents=new Map(readdirSync(stage).map(n=>[n,readFileSync(join(stage,n))]));
   writeFileSync(args[args.indexOf('-o')+1],'encrypted-fixture-bytes');return '';
  }
  if(args[0]==='isencrypted')return '<plist><dict><key>encrypted</key><true/></dict></plist>';
  if(args[0]==='attach'){
   assert(args.includes('-readonly')&&args.includes('-stdinpass'));assert.deepEqual(pass,password);
   const mount=args[args.indexOf('-mountpoint')+1];for(const[n,b]of contents)writeFileSync(join(mount,n),b,{mode:0o600});return '';
  }
  if(args[0]==='detach')return '';
  throw Error('unexpected command');
 };
 return {root,directory,destination,receipt,password,calls,command,files};
}
test('only an AES-256 image is exported, then restored bytes and key proof are checked',t=>{
 const f=fixture(t);let proved=false;
 const result=encryptedKeyBackup({...f,prove:p=>{proved=true;assert.equal(readFileSync(p.restorePath,'utf8'),'private-fixture-updater.key');}});
 assert(proved);assert.equal(result.encryption,'AES-256');
 assert.deepEqual(readdirSync(f.destination),['ClamAV-updater-eeeeeeeeeeeeeeee.dmg']);
 assert(!readFileSync(result.file,'utf8').includes('private-fixture'));
 assert(f.calls.some(a=>a[0]==='detach'));assert(!readdirSync(f.directory).some(n=>n.startsWith('.encrypted-backup-')));
});
test('an unencrypted image never produces a successful backup',t=>{
 const f=fixture(t);
 const command=(args,pass)=>args[0]==='isencrypted'?'<plist><key>encrypted</key><false/></plist>':f.command(args,pass);
 assert.throws(()=>encryptedKeyBackup({...f,command,prove:()=>assert.fail('proof before encryption check')}),/not encrypted/);
 assert(!f.calls.some(a=>a[0]==='attach'));
});
test('changed restored key bytes fail before the signing proof',t=>{
 const f=fixture(t);
 const command=(args,pass)=>{const result=f.command(args,pass);if(args[0]==='attach')writeFileSync(join(args[args.indexOf('-mountpoint')+1],'updater.key'),'changed key');return result;};
 assert.throws(()=>encryptedKeyBackup({...f,command,prove:()=>assert.fail('proof before restored-byte check')}),/content differs/);
 assert(f.calls.some(a=>a[0]==='detach'));
});
test('failed restored-key proof detaches only the private image and retains ciphertext',t=>{
 const f=fixture(t);
 assert.throws(()=>encryptedKeyBackup({...f,prove:()=>{throw Error('restored-key refused');}}),/restored-key refused/);
 assert(f.calls.some(a=>a[0]==='detach'));assert.equal(readdirSync(f.destination).length,1);
 assert(!readdirSync(f.directory).some(n=>n.startsWith('.encrypted-backup-')));
});
test('unknown existing images and dangling symlinks are never overwritten',t=>{
 const f=fixture(t);const path=join(f.destination,'ClamAV-updater-eeeeeeeeeeeeeeee.dmg');
 writeFileSync(path,'existing image');assert.throws(()=>encryptedKeyBackup({...f}),/will not be overwritten/);assert.equal(readFileSync(path,'utf8'),'existing image');
 rmSync(path);symlinkSync(join(f.root,'missing-target'),path);
 assert.throws(()=>encryptedKeyBackup({...f}),/will not be overwritten/);assert(!existsSync(join(f.root,'missing-target')));
 assert.equal(f.calls.length,0);
});
test('a detach failure preserves the private workspace instead of recursing into a mounted image',t=>{
 const f=fixture(t);
 const command=(args,pass)=>{if(args[0]==='detach')throw Error('detach refused');return f.command(args,pass);};
 assert.throws(()=>encryptedKeyBackup({...f,command,prove:()=>{}}),/detach refused/);
 assert(readdirSync(f.directory).some(n=>n.startsWith('.encrypted-backup-')));
});
