const fs = require('node:fs');
const receipt = process.argv[2];
fs.writeFileSync(receipt, Buffer.alloc(0));
if (process.stdin.isTTY) process.stdin.setRawMode(true);
process.stdin.on('data', chunk => fs.appendFileSync(receipt, chunk));
process.stdout.write('\x1b[?2004hCLIPBOARD_FIXTURE_READY\r\n');
