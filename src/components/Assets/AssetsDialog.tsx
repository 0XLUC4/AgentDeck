import { ArrowLeft, Box, ImageIcon, Send } from "lucide-react";
import { type FormEvent, useEffect, useMemo, useState } from "react";
import { AGENTS } from "../../lib/agents";
import { type AssetFile, assetSource, assetsList } from "../../lib/ipc";
import { deck, type Pane, useDeck } from "../../store/deck";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { sendPrompt } from "../Broadcast/send";
import { IconButton } from "../IconButton/IconButton";
import { Modal, Segmented } from "../Usage/Modal";
import { BACKDROP, fileUrl, ModelViewer, renderThumbnail } from "./ModelViewer";

const MODEL_EXTS = new Set(["glb", "gltf", "obj", "fbx", "stl", "blend"]);
const isModel = (file: AssetFile) => MODEL_EXTS.has(file.ext);

type Filter = "all" | "models" | "images";

/** Rendered thumbnails for the app's lifetime (key: path + size, so an edited file renders again). */
const thumbnails = new Map<string, string | null>();

/** Grid tile: an image shows itself, a model a thumbnail rendered here. */
function Tile({ cwd, file, images, onOpen }: { cwd: string; file: AssetFile; images: string[]; onOpen: () => void }) {
  const key = `${file.path}:${file.size}`;
  const ready = isModel(file) ? thumbnails.get(key) : fileUrl(file.path);
  const [thumb, setThumb] = useState(ready);

  useEffect(() => {
    if (!isModel(file) || thumbnails.has(key)) return;
    let alive = true;
    // Queued: a tile scrolled away or a closed dialog skips its turn instead of running Blender for nothing.
    const source = () => (alive ? assetSource(cwd, file.path) : Promise.reject(new Error("skipped")));
    renderThumbnail(source, file.ext, images)
      .then((url) => {
        thumbnails.set(key, url);
        if (alive) setThumb(url);
      })
      .catch(() => {
        if (!alive) return;
        thumbnails.set(key, null);
        setThumb(null);
      });
    return () => {
      alive = false;
    };
  }, [cwd, file, key, images]);

  const Icon = isModel(file) ? Box : ImageIcon;
  return (
    <button
      type="button"
      onClick={onOpen}
      title={file.rel}
      className="group flex min-w-0 flex-col gap-1.5 rounded-xl p-1.5 text-left transition-colors duration-150 hover:bg-hover"
    >
      <span
        className="relative flex aspect-square w-full items-center justify-center overflow-hidden rounded-xl border border-line"
        style={{ background: BACKDROP }}
      >
        {thumb ? (
          <img src={thumb} alt="" loading="lazy" className="size-full object-contain" />
        ) : (
          <Icon
            size={28}
            strokeWidth={1.25}
            className={`text-black/25 ${thumb === undefined ? "animate-pulse" : ""}`}
            aria-hidden="true"
          />
        )}
        <span className="absolute top-1.5 right-1.5 rounded-md bg-black/50 px-1.5 text-[10px] uppercase text-white/80">
          {file.ext}
        </span>
      </span>
      <span className="truncate px-0.5 text-xs text-muted group-hover:text-fg">{file.rel.split(/[\\/]/).pop()}</span>
    </button>
  );
}

const field =
  "rounded-xl border border-line bg-base px-3 text-fg outline-none placeholder:text-subtle focus:border-line-strong disabled:opacity-50";

/** One model (or image) in the viewer: click a part, say what to change, pick the agent that should do it. */
function Detail({
  cwd,
  file,
  images,
  agents,
  onBack,
}: {
  cwd: string;
  file: AssetFile;
  images: string[];
  agents: Pane[];
  onBack: () => void;
}) {
  const model = isModel(file);
  const [source, setSource] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [part, setPart] = useState<string | null>(null);
  const [instruction, setInstruction] = useState("");
  const [target, setTarget] = useState("");
  const targetId = agents.some((p) => p.id === target) ? target : (agents[0]?.id ?? "");

  useEffect(() => {
    if (!model) return;
    assetSource(cwd, file.path)
      .then(setSource)
      .catch((err) => setError(String(err)));
  }, [cwd, file, model]);

  const send = (e: FormEvent) => {
    e.preventDefault();
    if (!targetId || !instruction.trim()) return;
    const whole = model ? `the 3D model ${file.path}` : `the image ${file.path}`;
    const subject = part ? `the object "${part}" of ${whole}` : whole;
    const name = agents.find((p) => p.id === targetId)?.title || "the agent";
    sendPrompt(targetId, `In ${subject}: ${instruction.trim()}`)
      .then(() => {
        setInstruction("");
        deck.toast(`Sent to ${name}`);
      })
      .catch((err) => deck.toast(`Could not send: ${String(err)}`));
  };

  return (
    <div className="flex h-[72vh] flex-col gap-3">
      <div className="flex items-center gap-2">
        <IconButton icon={ArrowLeft} label="Back to all assets" onClick={onBack} />
        <span className="truncate font-medium">{file.rel}</span>
      </div>
      <div className="relative min-h-0 flex-1 overflow-hidden rounded-xl border border-line">
        {!model ? (
          <div className="flex h-full items-center justify-center p-4" style={{ background: BACKDROP }}>
            <img src={fileUrl(file.path)} alt={file.rel} className="max-h-full max-w-full object-contain" />
          </div>
        ) : source ? (
          <ModelViewer
            key={source}
            path={source}
            ext={file.ext}
            images={images}
            onSelect={setPart}
            onError={setError}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-neutral-600" style={{ background: BACKDROP }}>
            {error ? null : file.ext === "blend" ? "Converting with Blender…" : "Loading…"}
          </div>
        )}
        {error && (
          <p className="absolute inset-x-0 top-1/2 px-6 text-center text-bad" role="alert">
            {error}
          </p>
        )}
        {model && (
          <span className="absolute top-2 left-2 rounded-lg bg-black/55 px-2 py-1 text-xs text-white">
            {part ? (
              <>
                Selected: <span className="font-medium">{part}</span>
              </>
            ) : (
              "Drag to orbit, scroll to zoom, click a part to select it"
            )}
          </span>
        )}
      </div>

      <form onSubmit={send} className="flex flex-col gap-2">
        <fieldset className="flex flex-wrap items-center gap-1.5" aria-label="Send to">
          {agents.length === 0 && (
            <span className="text-subtle">Start an agent in this workspace to send changes.</span>
          )}
          {agents.map((p) => (
            <button
              key={p.id}
              type="button"
              aria-pressed={p.id === targetId}
              onClick={() => setTarget(p.id)}
              className="inline-flex h-8 max-w-56 items-center gap-2 rounded-xl border border-line px-2.5 text-muted transition-colors duration-150 hover:bg-hover hover:text-fg aria-pressed:border-line-strong aria-pressed:bg-raised aria-pressed:text-fg"
            >
              <AgentLogo kind={p.agent} size={15} />
              <span className="truncate">{p.title || AGENTS[p.agent].name}</span>
            </button>
          ))}
        </fieldset>
        <div className="flex gap-2">
          <input
            value={instruction}
            onChange={(e) => setInstruction(e.target.value)}
            aria-label="What to change"
            placeholder={
              part ? `What should change on "${part}"?` : `What should change on this ${model ? "model" : "image"}?`
            }
            className={`${field} h-10 min-w-0 flex-1`}
          />
          <button
            type="submit"
            disabled={!targetId || !instruction.trim()}
            className="inline-flex h-10 shrink-0 items-center gap-1.5 rounded-xl bg-accent px-4 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:opacity-40"
          >
            <Send size={14} strokeWidth={1.5} aria-hidden="true" />
            Send
          </button>
        </div>
      </form>
    </div>
  );
}

/** Explorer-like grid of the workspace's 3D models and images with thumbnails; a click opens one. */
export function AssetsDialog({ workspaceId, onClose }: { workspaceId: string; onClose: () => void }) {
  const workspace = useDeck((s) => s.workspaces.find((w) => w.id === workspaceId));
  const tabs = useDeck((s) => s.tabs);
  const panes = useDeck((s) => s.panes);
  const [files, setFiles] = useState<AssetFile[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [opened, setOpened] = useState<AssetFile | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const images = useMemo(() => files?.filter((f) => !isModel(f)).map((f) => f.path) ?? [], [files]);
  const shown = files?.filter((f) => filter === "all" || (filter === "models") === isModel(f));
  const agents = useMemo(
    () =>
      tabs
        .filter((t) => t.workspaceId === workspaceId)
        .flatMap((t) => t.paneIds.map((id) => panes[id]))
        .filter((p): p is Pane => Boolean(p) && !p.exited),
    [tabs, panes, workspaceId],
  );
  const cwd = workspace?.path ?? "";

  useEffect(() => {
    if (!cwd) return;
    assetsList(cwd)
      .then(setFiles)
      .catch((err) => setListError(String(err)));
  }, [cwd]);

  if (!workspace) return null;
  return (
    <Modal
      title={`Assets · ${workspace.name}`}
      onClose={onClose}
      className="w-[min(72rem,96vw)]"
      actions={
        !opened && (
          <Segmented
            label="Show"
            value={filter}
            onChange={setFilter}
            options={[
              { id: "all", label: "All" },
              { id: "models", label: "3D" },
              { id: "images", label: "Images" },
            ]}
          />
        )
      }
    >
      {!cwd ? (
        <p className="text-muted">Link a folder to this workspace to browse its 3D assets.</p>
      ) : opened ? (
        <Detail cwd={cwd} file={opened} images={images} agents={agents} onBack={() => setOpened(null)} />
      ) : (
        <div className="min-h-[40vh]">
          {listError && <p className="text-bad">{listError}</p>}
          {files === null && !listError && <p className="text-subtle">Scanning…</p>}
          {shown?.length === 0 && <p className="text-subtle">No 3D model or image here.</p>}
          <div className="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-2">
            {shown?.map((file) => (
              <Tile key={file.path} cwd={cwd} file={file} images={images} onOpen={() => setOpened(file)} />
            ))}
          </div>
        </div>
      )}
    </Modal>
  );
}
