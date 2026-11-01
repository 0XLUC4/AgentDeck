// Small force-directed layout for the notes graph: links pull, every pair repels, a weak pull to the center.

export interface SimNode {
  x: number;
  y: number;
  vx: number;
  vy: number;
  /** Held by the pointer: not moved by the forces. */
  pinned?: boolean;
}

const LINK_LENGTH = 60;
const REPULSION = 1800;
const CENTER_PULL = 0.004;
const DAMPING = 0.6;

/** Nodes on a spiral, so the first frames already look spread out. */
export function initialPositions(count: number): SimNode[] {
  return Array.from({ length: count }, (_, i) => {
    const angle = i * 2.4;
    const radius = 12 * Math.sqrt(i);
    return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius, vx: 0, vy: 0 };
  });
}

/**
 * One simulation step scaled by `alpha` (cooling from 1 to 0). Edges are index pairs.
 * shortcut: repulsion is O(n²); fine for a few hundred notes, switch to Barnes-Hut if workspaces get much bigger.
 */
export function step(nodes: SimNode[], edges: [number, number][], alpha: number): void {
  for (let i = 0; i < nodes.length; i++) {
    const a = nodes[i];
    for (let j = i + 1; j < nodes.length; j++) {
      const b = nodes[j];
      const dx = b.x - a.x || 0.01;
      const dy = b.y - a.y || 0.01;
      const dist2 = Math.max(dx * dx + dy * dy, 25);
      const force = (REPULSION / dist2) * alpha;
      const dist = Math.sqrt(dist2);
      a.vx -= (dx / dist) * force;
      a.vy -= (dy / dist) * force;
      b.vx += (dx / dist) * force;
      b.vy += (dy / dist) * force;
    }
  }
  for (const [i, j] of edges) {
    const a = nodes[i];
    const b = nodes[j];
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const dist = Math.sqrt(dx * dx + dy * dy) || 1;
    const force = ((dist - LINK_LENGTH) / dist) * 0.08 * alpha;
    a.vx += dx * force;
    a.vy += dy * force;
    b.vx -= dx * force;
    b.vy -= dy * force;
  }
  for (const node of nodes) {
    node.vx = (node.vx - node.x * CENTER_PULL * alpha) * DAMPING;
    node.vy = (node.vy - node.y * CENTER_PULL * alpha) * DAMPING;
    if (node.pinned) {
      node.vx = 0;
      node.vy = 0;
      continue;
    }
    node.x += node.vx;
    node.y += node.vy;
  }
}
