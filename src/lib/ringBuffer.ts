/** Keeps the most recent `cap` characters of terminal output, stored as chunks. */
export class RingBuffer {
  private chunks: string[] = [];
  private size = 0;

  constructor(private readonly cap = 1_000_000) {}

  push(data: string): void {
    this.chunks.push(data);
    this.size += data.length;
    while (this.size > this.cap && this.chunks.length > 1) {
      this.size -= this.chunks.shift()?.length ?? 0;
    }
    if (this.size > this.cap) {
      // A single chunk larger than cap: keep its tail.
      this.chunks = [this.text().slice(-this.cap)];
      this.size = this.cap;
    }
  }

  text(): string {
    return this.chunks.join("");
  }

  clear(): void {
    this.chunks = [];
    this.size = 0;
  }
}
