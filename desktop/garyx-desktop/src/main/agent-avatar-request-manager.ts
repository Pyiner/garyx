export type AvatarRequestContext = {
  signal: AbortSignal;
  userSignal: AbortSignal;
  timeoutSignal: AbortSignal;
};

export class AgentAvatarRequestManager {
  readonly #controllers = new Map<string, AbortController>();

  async run<T>(
    requestId: string,
    timeoutMs: number,
    operation: (context: AvatarRequestContext) => Promise<T>,
  ): Promise<T> {
    const normalizedRequestId = requestId.trim();
    if (!normalizedRequestId) {
      throw new Error("Avatar generation request ID is required.");
    }
    if (this.#controllers.has(normalizedRequestId)) {
      throw new Error("Avatar generation request ID is already active.");
    }

    const userController = new AbortController();
    const timeoutController = new AbortController();
    const timeout = setTimeout(() => {
      timeoutController.abort(new DOMException("Avatar generation timed out.", "TimeoutError"));
    }, timeoutMs);
    const signal = AbortSignal.any([
      userController.signal,
      timeoutController.signal,
    ]);
    this.#controllers.set(normalizedRequestId, userController);

    try {
      return await operation({
        signal,
        userSignal: userController.signal,
        timeoutSignal: timeoutController.signal,
      });
    } finally {
      clearTimeout(timeout);
      if (this.#controllers.get(normalizedRequestId) === userController) {
        this.#controllers.delete(normalizedRequestId);
      }
    }
  }

  cancel(requestId: string): boolean {
    const controller = this.#controllers.get(requestId.trim());
    if (!controller) {
      return false;
    }
    controller.abort(new DOMException("Avatar generation cancelled.", "AbortError"));
    return true;
  }

  has(requestId: string): boolean {
    return this.#controllers.has(requestId.trim());
  }
}
