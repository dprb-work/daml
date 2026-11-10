import { ErrorCodes, Message, MessageWriter, ResponseError } from 'vscode-jsonrpc/node';

type Phase = 'bootstrap' | 'initializing' | 'delivering-initialized' | 'initialized' | 'shutdown' | 'exited' | 'closed';

/** The final, synchronous admission boundary for one RPC connection. */
export class SessionMessageWriter implements MessageWriter {
  private phase: Phase = 'bootstrap';
  private retired = false;

  constructor(private readonly writer: MessageWriter, private readonly isCurrent: () => boolean) {}

  get onError(): MessageWriter['onError'] { return this.writer.onError; }
  get onClose(): MessageWriter['onClose'] { return this.writer.onClose; }

  retire(): void {
    this.retired = true;
  }

  write(message: Message): Promise<void> {
    const method = 'method' in message ? message.method : undefined;
    const request = 'id' in message && typeof method === 'string';
    const notification = !('id' in message) && typeof method === 'string';
    const response = !('method' in message) && 'id' in message;
    let admitted = false;
    if (this.phase === 'initialized' && request && method === 'shutdown') {
      this.retire();
      this.phase = 'shutdown';
      admitted = true;
    } else if (this.phase === 'shutdown' && notification && method === 'exit') {
      this.phase = 'exited';
      admitted = true;
    } else if (!this.retired && this.isCurrent()) {
      if (this.phase === 'bootstrap' && request && method === 'initialize') {
        this.phase = 'initializing';
        admitted = true;
      } else if (this.phase === 'initializing' && notification && method === 'initialized') {
        this.phase = 'delivering-initialized';
        return this.writer.write(message).then(() => {
          // Stock feature initialization awaits this delivery. Retirement or
          // staleness while it was pending must never reopen ordinary admission.
          if (!this.retired && this.isCurrent() && this.phase === 'delivering-initialized') {
            this.phase = 'initialized';
          } else {
            this.retire();
          }
        }, error => {
          this.retire();
          throw error;
        });
      } else if ((this.phase === 'initializing' || this.phase === 'delivering-initialized') && response) {
        admitted = true;
      } else if (this.phase === 'initialized' && method !== 'initialize'
          && method !== 'initialized' && method !== 'exit') {
        admitted = true;
      }
    }
    if (!admitted) {
      return Promise.reject(new ResponseError(ErrorCodes.ConnectionInactive, 'The DAML language server session is inactive.'));
    }
    // The stock writer owns FIFO, including writes admitted before retirement.
    return this.writer.write(message);
  }

  end(): void {
    this.retire();
    this.phase = 'closed';
    this.writer.end();
  }

  dispose(): void {
    this.retire();
    this.phase = 'closed';
    this.writer.dispose();
  }
}
