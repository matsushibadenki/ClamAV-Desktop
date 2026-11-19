// Read-only capture for the installed macOS app. Save stdout privately if needed.
import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';

const app = process.argv[2] ?? '/Applications/ClamAV Desktop.app';
const executable = app + '/Contents/MacOS/clamav-desktop';
const label = 'app.clamav.desktop.worker';
function run(file, args, input) {
  return execFileSync(file, args, {
    input, encoding: 'utf8', timeout: 45000, maxBuffer: 16 * 1024 * 1024,
    stdio: ['pipe', 'pipe', 'pipe'],
  });
}

try {
  if (process.platform !== 'darwin' || process.argv.length > 3 || !app.startsWith('/')) {
    throw new Error('usage');
  }
  const plan = JSON.parse(run(executable, ['--login-service-check']));
  const report = {
    capturedAt: new Date().toISOString(),
    registration: plan.registration,
    permission: plan.permission,
    loaded: plan.loaded,
    worker: null,
    scheduledRun: null,
    collectionErrors: plan.osQueryFailed ? ['launchd_status_query_failed'] : [],
  };
  if (plan.registration === 'matches' && plan.loaded === true) {
    const target = 'gui/' + process.getuid() + '/' + label;
    const pid = () => /^\s*pid = ([1-9]\d*)\s*$/m.exec(run('/bin/launchctl', ['print', target]))?.[1];
    try {
      const before = pid();
      if (!before) throw new Error('worker not running');
      const processInfo = run('/bin/ps', ['-p', before, '-o', 'ppid=', '-o', 'lstart=']).trim();
      const id = randomUUID();
      const response = JSON.parse(run(executable, ['--worker', 'request', '--data-dir', plan.dataDirectory], JSON.stringify({
        version: 1, id, issuedAt: new Date().toISOString(), body: { command: 'get_snapshot' },
      })));
      if (response.id !== id || response.error || response.payload?.type !== 'snapshot' || !processInfo || pid() !== before) {
        throw new Error('inconsistent capture');
      }
      const [parent, ...started] = processInfo.split(/\s+/);
      report.worker = { pid: Number(before), parentPid: Number(parent), processStartLocal: started.join(' '), ipcResponded: true };
      const scheduledRun = response.payload.scheduledRun;
      if (scheduledRun) {
        const fields = ['localDate', 'scheduledAt', 'startedAt', 'finishedAt', 'status', 'scanId'];
        report.scheduledRun = Object.fromEntries(fields.map(key => [key, scheduledRun[key]]));
      }
    } catch {
      report.collectionErrors.push('worker_observation_failed');
    }
  }
  console.log(JSON.stringify(report, null, 2));
  if (report.collectionErrors.length) process.exitCode = 1;
} catch {
  console.error('Capture failed. Use an absolute path to an installed, signed ClamAV Desktop.app.');
  process.exitCode = 1;
}
