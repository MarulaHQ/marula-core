// Transport result unwrapping for SDK calls.
//
// Purpose: openapi-fetch returns `{ data, error, response }` instead of
// throwing. The engine needs one honest failure object carrying the IF3
// envelope plus the HTTP status, so classifyFailure can do its job. This is
// the only place that turns a transport result into an exception.
//
// Non-purpose: no retries, no queueing, no logging.

export interface TransportResult<T> {
  data?: T;
  error?: unknown;
  response: Response;
}

export class HttpFailure extends Error {
  readonly status: number;
  readonly body: unknown;

  constructor(status: number, message: string, body: unknown) {
    super(message);
    this.name = "HttpFailure";
    this.status = status;
    this.body = body;
  }
}

export function unwrap<T>(result: TransportResult<T>): T {
  const { data, error, response } = result;
  if (error !== undefined) {
    const message = envelopeMessage(error) ?? response.statusText;
    throw new HttpFailure(response.status, message, error);
  }
  if (data === undefined) {
    if (response.status === 204) return undefined as T;
    throw new HttpFailure(response.status, response.statusText, undefined);
  }
  return data;
}

function envelopeMessage(error: unknown): string | undefined {
  const message = (error as { error?: { message?: unknown } } | null | undefined)?.error
    ?.message;
  return typeof message === "string" ? message : undefined;
}
