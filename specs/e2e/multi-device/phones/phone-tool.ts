// A phone tool starts and stops a group of virtual phones that share one
// virtual Bluetooth medium (ADR-0012 D1, D7).

export interface PhoneTool {
  readonly name: string;
  /** Starts `count` phones in one group and gives their adb serials when all have booted. */
  up(count: number): Promise<string[]>;
  /** Stops the phones that `up` started. */
  down(serials: string[]): Promise<void>;
}

/** A phone did not start. This is a failure of the test machine, not of Fini. */
export class PhoneBootError extends Error {
  constructor(message: string) {
    super(`[infrastructure] ${message}`);
    this.name = 'PhoneBootError';
  }
}
