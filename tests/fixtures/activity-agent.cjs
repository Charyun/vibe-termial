const fs = require('node:fs');
if (process.stdin.isTTY) process.stdin.setRawMode(true);
const sessionIndex = process.argv.indexOf('--session');
const tape = sessionIndex >= 0 ? process.argv[sessionIndex + 1] : undefined;
if (tape) fs.writeFileSync(tape, '');
let incoming = '', ready = false, sequence = 0;
function status(state) {
  process.stdout.write(`\x1b]7501;state=${state}:app=fixture\x1b\\`);
  if (!tape) return;
  const message = state === 'working' ? {role:'user',content:'run ' + (++sequence)}
    : state === 'done' ? {role:'assistant',content:'completed',stopReason:'stop'}
    : state === 'error' ? {role:'assistant',content:[],stopReason:'error'}
    : state === 'idle' && sequence > 0 ? {role:'assistant',content:[],stopReason:'aborted'} : null;
  if (message) fs.appendFileSync(tape, JSON.stringify({type:'message',message}) + '\n');
}
function start() {
  if (ready) return;
  ready = true; incoming = '';
  status('idle');
  process.stdout.write('ACTIVITY_FIXTURE_READY\r\n');
}
process.stdin.setEncoding('utf8');
process.stdin.on('data', data => {
  incoming += data;
  if (!ready) {
    if (!/\x1b\]7501;\?:[^\x1b]*\x1b\\/.test(incoming)) return;
    start(); return;
  }
  for (const key of incoming) {
    if (key === 'W') status('working');
    else if (key === 'D') status('done');
    else if (key === 'B') status('blocked');
    else if (key === 'E') status('error');
    else if (key === 'I') status('idle');
  }
  incoming = '';
});
process.stdout.write('\x1b]7501;?\x1b\\\x1b[c');
// Current Windows ConPTY consumes unknown OSCs; the session-log channel still works.
setTimeout(start, 150);
