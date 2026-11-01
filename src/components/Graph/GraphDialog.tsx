import { useEffect, useMemo, useRef, useState } from "react";
import { type NotesGraph, noteRead, notesGraph } from "../../lib/ipc";
import { useDeck } from "../../store/deck";
import { Modal } from "../Usage/Modal";
import { initialPositions, type SimNode, step } from "./forceLayout";

const css = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

interface View {
  x: number;
  y: number;
  k: number;
}

/**
 * Canvas map of the notes: drag the background to pan, scroll to zoom, drag a note to move it, click it to read it.
 * Hovering a note lights up its links.
 */
function GraphCanvas({
  graph,
  selected,
  onSelect,
}: {
  graph: NotesGraph;
  selected: string | null;
  onSelect: (id: string) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const onSelectRef = useRef(onSelect);
  onSelectRef.current = onSelect;
  const redrawRef = useRef<() => void>(() => undefined);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    const index = new Map(graph.nodes.map((n, i) => [n.id, i]));
    const edges = graph.edges.map(([a, b]) => [index.get(a) ?? 0, index.get(b) ?? 0] as [number, number]);
    const neighbors = graph.nodes.map(() => new Set<number>());
    for (const [a, b] of edges) {
      neighbors[a].add(b);
      neighbors[b].add(a);
    }
    const radius = neighbors.map((n) => 3 + Math.sqrt(n.size) * 2);
    const nodes: SimNode[] = initialPositions(graph.nodes.length);
    const colors = {
      node: css("--color-subtle"),
      hub: css("--color-fg"),
      edge: css("--color-line-strong"),
      label: css("--color-muted"),
      active: css("--color-ok"),
    };
    let view: View = { x: 0, y: 0, k: 1 };
    let hovered = -1;
    let alpha = 1;
    let frame = 0;

    const draw = () => {
      const dpr = window.devicePixelRatio;
      const { clientWidth: w, clientHeight: h } = canvas;
      if (canvas.width !== w * dpr) canvas.width = w * dpr;
      if (canvas.height !== h * dpr) canvas.height = h * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      ctx.translate(w / 2 + view.x, h / 2 + view.y);
      ctx.scale(view.k, view.k);
      const focus = hovered >= 0 ? hovered : (index.get(selectedRef.current ?? "") ?? -1);
      const lit = (i: number) => focus < 0 || i === focus || neighbors[focus].has(i);

      ctx.lineWidth = 1 / view.k;
      for (const [a, b] of edges) {
        const on = focus >= 0 && (a === focus || b === focus);
        ctx.strokeStyle = on ? colors.active : colors.edge;
        ctx.globalAlpha = focus < 0 || on ? 0.7 : 0.15;
        ctx.beginPath();
        ctx.moveTo(nodes[a].x, nodes[a].y);
        ctx.lineTo(nodes[b].x, nodes[b].y);
        ctx.stroke();
      }
      ctx.font = `${12 / view.k}px ${css("--font-sans")}`;
      ctx.textAlign = "center";
      nodes.forEach((node, i) => {
        ctx.globalAlpha = lit(i) ? 1 : 0.2;
        ctx.fillStyle = i === focus ? colors.active : neighbors[i].size > 3 ? colors.hub : colors.node;
        ctx.beginPath();
        ctx.arc(node.x, node.y, radius[i], 0, Math.PI * 2);
        ctx.fill();
        // Labels when zoomed in, or for the focused note and its neighbors.
        if (view.k > 1.2 || (focus >= 0 && lit(i))) {
          ctx.fillStyle = colors.label;
          ctx.fillText(graph.nodes[i].title, node.x, node.y + radius[i] + 12 / view.k);
        }
      });
      ctx.globalAlpha = 1;
    };
    redrawRef.current = draw;

    const tick = () => {
      step(nodes, edges, alpha);
      alpha *= 0.985;
      draw();
      frame = alpha > 0.02 ? requestAnimationFrame(tick) : 0;
    };
    const reheat = () => {
      alpha = Math.max(alpha, 0.3);
      if (!frame) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);

    const toWorld = (e: PointerEvent | WheelEvent) => {
      const rect = canvas.getBoundingClientRect();
      return {
        x: (e.clientX - rect.left - rect.width / 2 - view.x) / view.k,
        y: (e.clientY - rect.top - rect.height / 2 - view.y) / view.k,
      };
    };
    const nodeAt = (e: PointerEvent) => {
      const p = toWorld(e);
      return nodes.findIndex((n, i) => Math.hypot(n.x - p.x, n.y - p.y) <= radius[i] + 3 / view.k);
    };

    let drag: { node: number; startX: number; startY: number; moved: boolean } | null = null;
    const onDown = (e: PointerEvent) => {
      canvas.setPointerCapture(e.pointerId);
      drag = { node: nodeAt(e), startX: e.clientX, startY: e.clientY, moved: false };
      if (drag.node >= 0) nodes[drag.node].pinned = true;
    };
    const onMove = (e: PointerEvent) => {
      if (!drag) {
        const next = nodeAt(e);
        canvas.style.cursor = next >= 0 ? "pointer" : "grab";
        if (next !== hovered) {
          hovered = next;
          draw();
        }
        return;
      }
      drag.moved ||= Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) > 3;
      if (drag.node >= 0) {
        Object.assign(nodes[drag.node], toWorld(e));
        reheat();
      } else {
        view = { ...view, x: view.x + e.movementX, y: view.y + e.movementY };
        draw();
      }
    };
    const onUp = () => {
      if (drag && drag.node >= 0) {
        nodes[drag.node].pinned = false;
        if (!drag.moved) onSelectRef.current(graph.nodes[drag.node].id);
      }
      drag = null;
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const before = toWorld(e);
      const k = Math.min(6, Math.max(0.15, view.k * Math.exp(-e.deltaY * 0.0015)));
      // Zoom around the cursor: the world point under it stays put.
      view = { k, x: view.x + before.x * (view.k - k), y: view.y + before.y * (view.k - k) };
      draw();
    };
    const onLeave = () => {
      hovered = -1;
      draw();
    };
    const resize = new ResizeObserver(draw);
    resize.observe(canvas);
    canvas.addEventListener("pointerdown", onDown);
    canvas.addEventListener("pointermove", onMove);
    canvas.addEventListener("pointerup", onUp);
    canvas.addEventListener("pointerleave", onLeave);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      cancelAnimationFrame(frame);
      resize.disconnect();
      canvas.removeEventListener("pointerdown", onDown);
      canvas.removeEventListener("pointermove", onMove);
      canvas.removeEventListener("pointerup", onUp);
      canvas.removeEventListener("pointerleave", onLeave);
      canvas.removeEventListener("wheel", onWheel);
    };
  }, [graph]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: selected only triggers the redraw; draw reads it from the ref
  useEffect(() => redrawRef.current(), [selected]);

  return <canvas ref={canvasRef} className="size-full cursor-grab touch-none" aria-label="Notes graph" />;
}

/** Map of the workspace's Markdown notes and the links between them, with the selected note beside it. */
export function GraphDialog({ workspaceId, onClose }: { workspaceId: string; onClose: () => void }) {
  const workspace = useDeck((s) => s.workspaces.find((w) => w.id === workspaceId));
  const cwd = workspace?.path ?? "";
  const [graph, setGraph] = useState<NotesGraph | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [text, setText] = useState("");

  useEffect(() => {
    if (!cwd) return;
    notesGraph(cwd)
      .then(setGraph)
      .catch((err) => setError(String(err)));
  }, [cwd]);

  useEffect(() => {
    if (!selected) return;
    setText("");
    noteRead(cwd, selected)
      .then(setText)
      .catch((err) => setText(String(err)));
  }, [cwd, selected]);

  const linked = useMemo(
    () =>
      graph?.edges
        .filter(([a, b]) => a === selected || b === selected)
        .map(([a, b]) => (a === selected ? b : a))
        .sort() ?? [],
    [graph, selected],
  );

  if (!workspace) return null;
  const empty = graph?.nodes.length === 0;
  const counts = graph ? ` · ${graph.nodes.length} notes, ${graph.edges.length} links` : "";
  return (
    <Modal title={`Graph · ${workspace.name}${counts}`} onClose={onClose} className="w-[min(80rem,96vw)]">
      {!cwd ? (
        <p className="text-muted">Link a folder to this workspace to map its notes.</p>
      ) : (
        <div className="flex h-[72vh] gap-4">
          <div className="relative min-w-0 flex-1 overflow-hidden rounded-xl border border-line bg-base">
            {graph && !empty && <GraphCanvas graph={graph} selected={selected} onSelect={setSelected} />}
            <p className="pointer-events-none absolute inset-x-0 top-1/2 text-center text-subtle">
              {error ?? (graph === null ? "Reading notes…" : empty ? "No Markdown note in this folder." : "")}
            </p>
          </div>
          <aside className="flex w-80 shrink-0 flex-col gap-2" aria-label="Selected note">
            {selected ? (
              <>
                <h3 className="truncate font-medium" title={selected}>
                  {selected}
                </h3>
                {linked.length > 0 && (
                  <div className="flex flex-wrap gap-1">
                    {linked.map((id) => (
                      <button
                        key={id}
                        type="button"
                        onClick={() => setSelected(id)}
                        title={id}
                        className="max-w-full truncate rounded-lg border border-line px-2 py-0.5 text-xs text-muted transition-colors duration-150 hover:bg-hover hover:text-fg"
                      >
                        {id.split("/").pop()?.replace(/\.md$/, "")}
                      </button>
                    ))}
                  </div>
                )}
                <pre className="scroll-thin min-h-0 flex-1 overflow-auto rounded-xl border border-line bg-base p-3 font-sans text-xs whitespace-pre-wrap text-muted">
                  {text}
                </pre>
              </>
            ) : (
              <p className="text-subtle">
                Click a note to read it. Drag the background to pan, scroll to zoom, drag a note to move it.
              </p>
            )}
          </aside>
        </div>
      )}
    </Modal>
  );
}
