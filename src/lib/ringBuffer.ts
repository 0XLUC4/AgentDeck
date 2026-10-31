/** Keeps the most recent `cap` characters of terminal output, stored as chunks. */
export class RingBuffer {
  private chunks: string[] = [];
  /** Index of the oldest live chunk: dropping is O(1), the array is compacted now and then. */
  private head = 0;
  private size = 0;

  // 512K chars (~1 MB UTF-16) per pane: plenty to repaint xterm's scrollback on remount or detach.
  constructor(private readonly cap = 512_000) {}

  push(data: string): void {
    this.chunks.push(data);
    this.size += data.length;
    while (this.size > this.cap && this.chunks.length - this.head > 1) {
      this.size -= this.chunks[this.head++].length;
    }
    if (this.head > 1024 && this.head * 2 > this.chunks.length) {
      this.chunks = this.chunks.slice(this.head);
      this.head = 0;
    }
    if (this.size > this.cap) {
      // A single chunk larger than cap: keep its tail.
      this.chunks = [this.text().slice(-this.cap)];
      this.head = 0;
      this.size = this.cap;
    }
  }

  text(): string {
    return this.head === 0 ? this.chunks.join("") : this.chunks.slice(this.head).join("");
  }

  /** Last `n` characters, joining only the chunks needed. */
  tail(n: number): string {
    const parts: string[] = [];
    let length = 0;
    for (let i = this.chunks.length - 1; i >= this.head && length < n; i--) {
      parts.push(this.chunks[i]);
      length += this.chunks[i].length;
    }
    return parts.reverse().join("").slice(-n);
  }

  clear(): void {
    this.chunks = [];
    this.head = 0;
    this.size = 0;
  }
}
