
const fs = require('node:fs');
const target = process.argv[process.argv.indexOf('--session') + 1];
let incoming = '', body = null, sequence = 0;
if (process.stdin.isTTY) process.stdin.setRawMode(true);
process.stdin.setEncoding('utf8');
process.stdout.write('CONTENT_FIXTURE_READY\r\n');
process.stdin.on('data', chunk => {
  incoming += chunk;
  while (incoming.length) {
    if (incoming.startsWith('\x1b[200~')) {
      const end = incoming.indexOf('\x1b[201~');
      if (end < 0) return;
      body = incoming.slice(6, end);
      incoming = incoming.slice(end + 6);
      fs.writeFileSync(target + '.body', body);
    } else if (incoming[0] === '\r') {
      incoming = incoming.slice(1);
      if (body !== null) {
        for (const [role, content] of [['user', body], ['assistant', 'Echo: ' + body]]) {
          fs.appendFileSync(target, JSON.stringify({type:'message', id:String(++sequence), message:{role, content}}) + '\n');
        }
        process.stdout.write('CONTENT_SUBMITTED\r\n');
        body = null;
      }
    } else { incoming = incoming.slice(1); }
  }
});
