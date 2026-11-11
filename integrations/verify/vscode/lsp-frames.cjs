// Decode frames for observation while forwarding the exact original bytes.
module.exports = function frames(accept) {
  let buffered = Buffer.alloc(0);
  return chunk => {
    buffered = Buffer.concat([buffered, chunk]);
    for (;;) {
      const split = buffered.indexOf('\r\n\r\n');
      if (split === -1) return;
      const match = /^Content-Length:\s*(\d+)\s*$/im.exec(buffered.subarray(0, split).toString('ascii'));
      if (!match) throw new Error('Missing Content-Length in captured LSP stream');
      const end = split + 4 + Number(match[1]);
      if (buffered.length < end) return;
      const frame = buffered.subarray(0, end);
      const message = JSON.parse(buffered.subarray(split + 4, end).toString('utf8'));
      buffered = buffered.subarray(end);
      accept(message, frame);
    }
  };
};
