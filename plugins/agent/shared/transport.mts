import { MAX_BYTES, errorResult } from './core.mts';

export async function serve(handler: (request: unknown) => unknown): Promise<void> {
  try {
    const chunks: Buffer[] = [];
    let size = 0;
    for await (const chunk of process.stdin) {
      const bytes = Buffer.from(chunk);
      size += bytes.length;
      if (size > MAX_BYTES) throw new Error('Core request exceeds 1 MiB.');
      chunks.push(bytes);
    }
    const text = new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks));
    const response = JSON.stringify(handler(JSON.parse(text)));
    if (Buffer.byteLength(response) > MAX_BYTES) throw new Error('Core response exceeds 1 MiB.');
    process.stdout.write(`${response}\n`);
  } catch (error) {
    process.stdout.write(`${JSON.stringify(errorResult(error))}\n`);
  }
}
