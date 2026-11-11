import {
  CancellationToken, CloseAction, CloseHandlerResult, ConnectionError, ConnectionErrors,
  ErrorCodes, ErrorHandler, LanguageClient, LanguageClientOptions, MessageSignature,
  MessageTransports, ResponseError, ServerOptions
} from 'vscode-languageclient/node';
import { SessionMessageWriter } from './session-writer';

type SessionOptions = {
  isCurrent: () => boolean;
  errorHandler?: ErrorHandler;
  onClosed: (decision: CloseHandlerResult) => void;
  onStop: (stopping: Promise<void>) => void;
};

/** Retains stock transports, providers and process ownership, but never stock restart effects. */
export class SessionLanguageClient extends LanguageClient {
  private writer?: SessionMessageWriter;
  private retired = false;
  private readonly takeCloseDecision: () => CloseHandlerResult | undefined;
  readonly restartPolicy: ErrorHandler;

  constructor(id: string, name: string, serverOptions: ServerOptions,
      clientOptions: LanguageClientOptions, private readonly sessionOptions: SessionOptions) {
    let delegate: ErrorHandler;
    let intentional = () => false;
    let decision: CloseHandlerResult | undefined;
    const errorHandler: ErrorHandler = {
      error: (error, message, count) => delegate.error(error, message, count),
      closed: async () => {
        if (intentional()) return { action: CloseAction.DoNotRestart, handled: true, message: 'DAML session retired.' };
        decision = await delegate.closed();
        // Do not let BaseLanguageClient invoke start(), or announce a restart
        // before the lifecycle owner has admitted its effect.
        return {
          action: CloseAction.DoNotRestart,
          handled: decision.action === CloseAction.Restart ? true : decision.handled,
          message: decision.action === CloseAction.Restart ? 'DAML connection closed; lifecycle reconciliation requested.' : decision.message
        };
      }
    };
    super(id, name, serverOptions, { ...clientOptions, errorHandler });
    delegate = sessionOptions.errorHandler ?? this.createDefaultErrorHandler(clientOptions.connectionOptions?.maxRestartCount);
    this.restartPolicy = delegate;
    intentional = () => this.retired;
    this.takeCloseDecision = () => {
      const result = decision;
      decision = undefined;
      return result;
    };
  }

  retire(): void {
    this.retired = true;
    this.writer?.retire();
  }

  override stop(timeout?: number): Promise<void> {
    this.retire();
    const stopping = super.stop(timeout);
    this.sessionOptions.onStop(stopping);
    return stopping;
  }

  override dispose(timeout?: number): Promise<void> {
    this.retire();
    return super.dispose(timeout);
  }

  override handleFailedRequest<T>(type: MessageSignature, token: CancellationToken | undefined,
      error: unknown, defaultValue: T, showNotification?: boolean): T {
    // These local connection outcomes are not successful empty language results.
    // In particular, the SDK otherwise consumes both as a provider's null value.
    if (error instanceof ResponseError && error.code === ErrorCodes.ConnectionInactive) throw error;
    if ((error instanceof ResponseError && error.code === ErrorCodes.PendingResponseRejected)
        || (error instanceof ConnectionError
          && (error.code === ConnectionErrors.Closed || error.code === ConnectionErrors.Disposed))) {
      const inactive = new ResponseError(ErrorCodes.ConnectionInactive,
        'The DAML language request was cancelled because its connection closed.');
      inactive.cause = error;
      throw inactive;
    }
    return super.handleFailedRequest(type, token, error, defaultValue, showNotification);
  }

  protected override async createMessageTransports(encoding: string, cancellationToken?: CancellationToken): Promise<MessageTransports> {
    const transports = await super.createMessageTransports(encoding, cancellationToken);
    const writer = new SessionMessageWriter(transports.writer, this.sessionOptions.isCurrent);
    this.writer = writer;
    if (this.retired) writer.retire();
    return { ...transports, writer };
  }

  protected override async handleConnectionClosed(): Promise<void> {
    this.writer?.retire();
    await super.handleConnectionClosed();
    const decision = this.takeCloseDecision();
    if (decision && !this.retired) this.sessionOptions.onClosed(decision);
  }
}
