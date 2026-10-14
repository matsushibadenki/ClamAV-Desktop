import { spawnSync } from 'node:child_process';
import { readFileSync, readdirSync, writeFileSync, existsSync, mkdirSync, rmSync, symlinkSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join, resolve } from 'node:path';
import { manifest, checksum } from './release-utils.mjs';
const root = fileURLToPath(new URL('../', import.meta.url));
const config = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
const version = config.version;
const versions = [JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version,
  ...['src-tauri/Cargo.toml','src-tauri/core/Cargo.toml'].map(file => readFileSync(join(root,file),'utf8').match(/^version = "([^"]+)"/m)?.[1])];
if (versions.some(value => value !== version)) throw new Error('Synchronize all application versions.');
const args = process.argv.slice(2);
const signed = args.includes('--signed');
const check = args.includes('--check');
if (args.some(a => !['--signed','--development','--check'].includes(a)) || signed === args.includes('--development')) throw new Error('Choose --signed or --development.');
if (process.platform !== 'darwin') throw new Error('macOS is required.');
const environment = { ...process.env, CI: 'true' };
function run(command, argv, capture = false, env = environment) {
  const result = spawnSync(command, argv, {cwd:root, env, encoding:'utf8', stdio:capture?'pipe':'inherit'});
  if (result.error || result.status !== 0) throw new Error(`Failed: ${command} ${argv[0] || ''}`);
  return result.stdout || '';
}
const profile = environment.APPLE_KEYCHAIN_PROFILE || 'clamav-desktop';
const notaryCredentials = ['--keychain-profile', profile, ...(environment.APPLE_NOTARY_KEYCHAIN ? ['--keychain', environment.APPLE_NOTARY_KEYCHAIN] : [])];
if (signed) {
  const identities = run('/usr/bin/security',['find-identity','-v','-p','codesigning'],true);
  const available = [...identities.matchAll(/([A-Fa-f0-9]{40}) "(Developer ID Application: [^"]+)"/g)];
  const requested = environment.APPLE_SIGNING_IDENTITY;
  let identity = requested && available.find(item => item[1] === requested || item[2] === requested);
  if (!requested && available.length && available.every(item => item[2] === available[0][2])) identity = available[0];
  if (!identity) throw new Error('Select a valid Developer ID Application certificate with APPLE_SIGNING_IDENTITY.');
  environment.APPLE_SIGNING_IDENTITY = identity[1]; // Hash avoids ambiguous duplicate certificate names.
  run('/usr/bin/xcrun',['notarytool','history',...notaryCredentials,'--output-format','json'],true);
  if (!environment.TAURI_SIGNING_PRIVATE_KEY && !environment.TAURI_SIGNING_PRIVATE_KEY_PATH) {
    const localKey = join(root,'.secrets/updater.key');
    if (!existsSync(localKey)) throw new Error('Updater signing key is required.');
    environment.TAURI_SIGNING_PRIVATE_KEY_PATH = localKey;
  }
  environment.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= '';
  // Notarize explicitly using the stored profile. Never ask Tauri to read raw Apple passwords.
  for (const key of ['APPLE_ID','APPLE_PASSWORD','APPLE_TEAM_ID','APPLE_API_ISSUER','APPLE_API_KEY','APPLE_API_KEY_PATH']) delete environment[key];
} else {
  for (const key of Object.keys(environment)) if (key.startsWith('APPLE_') || key.startsWith('TAURI_SIGNING_')) delete environment[key];
}
if (check) { console.log(`Preflight passed: ${signed?'signed/keychain-profile':'development'} build.`); process.exit(0); }
run('npm',['test']);
run('node',['--test','scripts/release-utils.test.mjs']);
run('cargo',['test','--manifest-path','src-tauri/Cargo.toml','--workspace','--locked']);
const signing = {bundle:{createUpdaterArtifacts:false,macOS:{signingIdentity:signed?environment.APPLE_SIGNING_IDENTITY:'-'}}};
run('npm',['run','tauri','--','build','--bundles',signed?'app':'app,dmg','--config',JSON.stringify(signing)]);
const bundle = join(root,'src-tauri/target/release/bundle');
const app = join(bundle,'macos/ClamAV Desktop.app');
const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
const dmgName = `ClamAV-Desktop_${version}_${arch === 'aarch64'?'aarch64':'x64'}.dmg`;
const dmg = join(bundle,'dmg',dmgName);
const output = join(bundle,'verification'); mkdirSync(output,{recursive:true});
function notarize(file) {
  const response = JSON.parse(run('/usr/bin/xcrun',['notarytool','submit',file,...notaryCredentials,'--wait','--output-format','json'],true));
  if (response.status !== 'Accepted') throw new Error(`Notarization did not succeed: ${response.status}. Submission: ${response.id}`);
  console.log(`Notarization accepted: ${response.id}`);
}
let updater;
if (signed) {
  const zip = join(output,'notarization.zip');
  run('/usr/bin/ditto',['-c','-k','--keepParent',app,zip]);
  notarize(zip); rmSync(zip);
  run('/usr/bin/xcrun',['stapler','staple',app]);
  run('/usr/bin/xcrun',['stapler','validate',app]);
  run('/usr/sbin/spctl',['--assess','--type','execute',app]);
  const stage = join(output,'dmg-stage'); rmSync(stage,{recursive:true,force:true}); mkdirSync(stage);
  try {
    run('/usr/bin/ditto',[app,join(stage,'ClamAV Desktop.app')]);
    symlinkSync('/Applications',join(stage,'Applications'));
    mkdirSync(join(bundle,'dmg'),{recursive:true});
    run('/usr/bin/hdiutil',['create','-srcfolder',stage,'-volname','ClamAV Desktop','-format','UDZO','-ov','-o',dmg]);
  } finally { rmSync(stage,{recursive:true,force:true}); }
  notarize(dmg);
  run('/usr/bin/xcrun',['stapler','staple',dmg]);
  run('/usr/bin/xcrun',['stapler','validate',dmg]);
  const name = `ClamAV-Desktop_${version}_${arch}.app.tar.gz`;
  const archive = join(bundle,'macos',name);
  run('/usr/bin/tar',['-czf',archive,'-C',join(bundle,'macos'),'ClamAV Desktop.app']);
  // CLI reads the private key from the environment, never from a printed command argument.
  run('npm',['run','tauri','--','signer','sign','--app-version',version,archive],true);
  const signature = readFileSync(archive+'.sig','utf8').trim();
  run('cargo',['test','--manifest-path','src-tauri/Cargo.toml','-p','clamav-desktop','app_updater::tests::release_signature_matches_public_key','--locked','--','--ignored'],false,{...environment,CLAMAV_UPDATE_TEST_ARCHIVE:archive});
  updater = {file:name,signature,sha256:checksum(readFileSync(archive))};
  writeFileSync(join(output,`latest-${arch}.json`),JSON.stringify(manifest(version,`darwin-${arch}`,name,signature),null,2)+'\n');
}
run('/usr/bin/codesign',['--verify','--deep','--strict',app]);
run('/usr/bin/hdiutil',['verify',dmg]);
const artifacts = [{file:dmgName,sha256:checksum(readFileSync(dmg))},...(updater?[{file:updater.file,sha256:updater.sha256}]:[])];
writeFileSync(join(output,'release.json'),JSON.stringify({version,architecture:process.arch,development:!signed,appNotarizationVerified:signed,dmgNotarizationVerified:signed,artifacts},null,2)+'\n');
writeFileSync(join(output,'SHA256SUMS'),artifacts.map(a=>`${a.sha256}  ${a.file}`).join('\n')+'\n');
console.log(`Verified ${signed?'signed/notarized':'development'} artifacts. No GitHub release was published.`);
