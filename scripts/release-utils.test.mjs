import { test } from 'node:test';
import assert from 'node:assert/strict';
import { manifest, mergeManifests } from './release-utils.mjs';
test('merges signed per-platform manifests and preserves signatures', () => {
  const a = manifest('0.2.0', 'darwin-aarch64', 'ClamAV-Desktop_0.2.0_aarch64.app.tar.gz', 'sig-a');
  const b = manifest('0.2.0', 'darwin-x86_64', 'ClamAV-Desktop_0.2.0_x86_64.app.tar.gz', 'sig-b');
  const result = mergeManifests([a,b]);
  assert.equal(result.platforms['darwin-x86_64'].signature, 'sig-b');
  assert.match(result.platforms['darwin-aarch64'].url, /\/v0.2.0\//);
  assert.throws(() => mergeManifests([a,a]), /Duplicate/);
  assert.throws(() => mergeManifests([a,manifest('0.3.0','darwin-x86_64','other.app.tar.gz','s')]), /versions/);
});
test('rejects missing signatures, unexpected hosts, names and platforms', () => {
  assert.throws(() => manifest('0.2.0','darwin-aarch64','../test.app.tar.gz','s'));
  assert.throws(() => manifest('0.2.0','darwin-aarch64','test.app.tar.gz',''));
  assert.throws(() => manifest('0.2.0','windows-x86_64','test.app.tar.gz','s'));
  const a = manifest('0.2.0','darwin-aarch64','test.app.tar.gz','s');
  a.platforms['darwin-aarch64'].url = 'https://example.com/test.app.tar.gz';
  assert.throws(() => mergeManifests([a]), /Untrusted/);
});
