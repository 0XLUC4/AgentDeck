import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";
import { FBXLoader } from "three/examples/jsm/loaders/FBXLoader.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { MTLLoader } from "three/examples/jsm/loaders/MTLLoader.js";
import { OBJLoader } from "three/examples/jsm/loaders/OBJLoader.js";
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
import { TGALoader } from "three/examples/jsm/loaders/TGALoader.js";

/** Models without materials (OBJ without .mtl, STL) get a warm clay color instead of flat white. */
const CLAY = new THREE.MeshStandardMaterial({ color: 0xc8a27a, roughness: 0.6, metalness: 0.05 });
const SELECTED = 0x4ade80;
/** Clean light-grey studio backdrop (viewer and thumbnails). */
export const BACKDROP = "radial-gradient(circle at 50% 35%, #f4f4f5 0%, #dcdcdf 100%)";

/**
 * Asset-protocol URL that keeps the folder structure, so loaders resolve textures, .bin and .mtl files
 * next to the model. (convertFileSrc encodes the whole path as one segment, which breaks relative URLs.)
 */
export function fileUrl(path: string): string {
  const segments = path.replace(/\\/g, "/").split("/").map(encodeURIComponent);
  return convertFileSrc("") + segments.join("/");
}

const fileName = (path: string) => (path.split(/[\\/]/).pop() ?? "").toLowerCase();

/**
 * Loading manager that finds textures the model points at but that are not there (absolute paths from the
 * author's machine, another folder layout) by file name among the workspace's images, and reads .tga.
 */
function textureManager(images: string[]): THREE.LoadingManager {
  const known = new Set(images.map((p) => fileUrl(p).toLowerCase()));
  const byName = new Map(images.map((p) => [fileName(p), p]));
  const manager = new THREE.LoadingManager();
  manager.setURLModifier((url) => {
    if (url.startsWith("data:") || url.startsWith("blob:") || known.has(url.toLowerCase())) return url;
    const found = byName.get(fileName(decodeURIComponent(url)));
    return found ? fileUrl(found) : url;
  });
  manager.addHandler(/\.tga$/i, new TGALoader(manager));
  return manager;
}

/**
 * Generated models often carry no material but name their parts `Wall__6b4a32`: paint those with that color.
 * Parts that already have a texture keep their material.
 */
function colorFromNames(model: THREE.Object3D): void {
  model.traverse((o) => {
    if (!(o instanceof THREE.Mesh)) return;
    let named: THREE.Object3D | null = o;
    let hex: string | undefined;
    while (named && !hex) {
      hex = /__([0-9a-f]{6})$/i.exec(named.name)?.[1];
      named = named.parent;
    }
    const textured = [o.material].flat().some((m) => "map" in m && m.map);
    if (hex && !textured) o.material = new THREE.MeshStandardMaterial({ color: `#${hex}`, roughness: 0.7 });
  });
}

async function loadObj(url: string, manager: THREE.LoadingManager): Promise<THREE.Object3D> {
  const text = await (await fetch(url)).text();
  const loader = new OBJLoader(manager);
  const library = /^mtllib\s+(.+)$/m.exec(text)?.[1]?.trim();
  const materials = library
    ? await new MTLLoader(manager)
        .setPath(url.slice(0, url.lastIndexOf("/") + 1))
        .loadAsync(library)
        .catch(() => null)
    : null;
  if (materials) {
    materials.preload();
    loader.setMaterials(materials);
  }
  const model = loader.parse(text);
  if (!materials) {
    model.traverse((o) => {
      if (o instanceof THREE.Mesh) o.material = CLAY;
    });
  }
  return model;
}

/**
 * Loads a model file (`path` from assetSource: a .blend arrives as its GLB export). `images` are the workspace's
 * image files, searched for textures the model cannot find.
 */
export async function loadModel(path: string, ext: string, images: string[]): Promise<THREE.Object3D> {
  const model = await parseFile(fileUrl(path), ext, textureManager(images));
  colorFromNames(model);
  return model;
}

async function parseFile(url: string, ext: string, manager: THREE.LoadingManager): Promise<THREE.Object3D> {
  switch (ext) {
    case "glb":
    case "gltf":
    case "blend":
      return (await new GLTFLoader(manager).loadAsync(url)).scene;
    case "fbx":
      return new FBXLoader(manager).loadAsync(url);
    case "obj":
      return loadObj(url, manager);
    case "stl": {
      const geometry = await new STLLoader(manager).loadAsync(url);
      geometry.computeVertexNormals();
      return new THREE.Mesh(geometry, CLAY);
    }
    default:
      throw new Error(`Unsupported format: .${ext}`);
  }
}

/** Lit scene + camera; `show` centers a model over a faint grid and frames it at a 3/4 angle. */
function createStage(renderer: THREE.WebGLRenderer) {
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  const scene = new THREE.Scene();
  const pmrem = new THREE.PMREMGenerator(renderer);
  const environment = pmrem.fromScene(new RoomEnvironment(), 0.04);
  pmrem.dispose();
  scene.environment = environment.texture;
  scene.add(new THREE.HemisphereLight(0xffffff, 0x444466, 0.6));
  const sun = new THREE.DirectionalLight(0xffffff, 1.6);
  sun.position.set(3, 5, 4);
  scene.add(sun);
  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 1000);

  const show = (model: THREE.Object3D) => {
    scene.add(model);
    const box = new THREE.Box3().setFromObject(model);
    const size = box.getSize(new THREE.Vector3()).length() || 1;
    const center = box.getCenter(new THREE.Vector3());
    model.position.sub(center);
    const grid = new THREE.GridHelper(size * 2, 20, 0x888888, 0x555555);
    grid.position.y = box.min.y - center.y;
    const gridMaterial = grid.material as THREE.Material;
    gridMaterial.opacity = 0.35;
    gridMaterial.transparent = true;
    scene.add(grid);
    camera.near = size / 100;
    camera.far = size * 100;
    camera.position.set(size * 0.7, size * 0.5, size * 0.9);
    camera.lookAt(0, 0, 0);
    camera.updateProjectionMatrix();
  };

  const dispose = () => {
    scene.traverse((o) => {
      if (o instanceof THREE.Mesh || o instanceof THREE.LineSegments) {
        o.geometry.dispose();
        for (const m of [o.material].flat()) {
          if (m === CLAY) continue;
          for (const value of Object.values(m)) if (value instanceof THREE.Texture) value.dispose();
          m.dispose();
        }
      }
    });
    environment.dispose();
  };
  return { scene, camera, show, dispose };
}

// One offscreen renderer for all thumbnails, one model at a time: WebGL contexts are scarce (terminals use them).
const THUMB_SIZE = 256;
let thumbRenderer: THREE.WebGLRenderer | null = null;
let thumbQueue: Promise<unknown> = Promise.resolve();
let thumbPending = 0;

/** PNG data URL of the model on a transparent background (the grid tile shows BACKDROP behind it). */
export function renderThumbnail(source: () => Promise<string>, ext: string, images: string[]): Promise<string> {
  thumbPending++;
  const job = thumbQueue.then(async () => {
    thumbRenderer ??= new THREE.WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true });
    thumbRenderer.setSize(THUMB_SIZE, THUMB_SIZE);
    const stage = createStage(thumbRenderer);
    try {
      stage.show(await loadModel(await source(), ext, images));
      thumbRenderer.render(stage.scene, stage.camera);
      return thumbRenderer.domElement.toDataURL("image/png");
    } finally {
      stage.dispose();
    }
  });
  thumbQueue = job
    .catch(() => undefined)
    .finally(() => {
      if (--thumbPending > 0) return;
      thumbRenderer?.dispose();
      thumbRenderer = null;
    });
  return job;
}

/** Nearest ancestor with a name (meshes exported from Blender are often unnamed children of the object). */
function namedObject(object: THREE.Object3D): THREE.Object3D {
  let current: THREE.Object3D | null = object;
  while (current && !current.name) current = current.parent;
  return current ?? object;
}

/**
 * Lit, colored preview of one model with orbit controls. Clicking a part selects it (green box) and reports its
 * name; clicking empty space clears the selection. Remount (key) to show another file.
 */
export function ModelViewer({
  path,
  ext,
  images,
  onSelect,
  onError,
}: {
  path: string;
  ext: string;
  images: string[];
  onSelect: (name: string | null) => void;
  onError: (message: string) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  // Callbacks via ref so a parent re-render never rebuilds the scene.
  const callbacks = useRef({ onSelect, onError });
  callbacks.current = { onSelect, onError };

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(window.devicePixelRatio);
    host.appendChild(renderer.domElement);
    const { scene, camera, show, dispose } = createStage(renderer);
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    const highlight = new THREE.BoxHelper(new THREE.Object3D(), SELECTED);
    highlight.visible = false;
    scene.add(highlight);

    let model: THREE.Object3D | null = null;
    let disposed = false;
    loadModel(path, ext, images)
      .then((loaded) => {
        if (disposed) return;
        model = loaded;
        show(loaded);
        controls.target.set(0, 0, 0);
      })
      .catch((err) => callbacks.current.onError(err instanceof Error ? err.message : String(err)));

    const observer = new ResizeObserver(() => {
      const { clientWidth: w, clientHeight: h } = host;
      if (w === 0 || h === 0) return;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    });
    observer.observe(host);
    renderer.setAnimationLoop(() => {
      controls.update();
      if (highlight.visible) highlight.update();
      renderer.render(scene, camera);
    });

    // A click (not the end of an orbit drag) picks the part under the cursor.
    let downAt = { x: 0, y: 0 };
    const onDown = (e: PointerEvent) => {
      downAt = { x: e.clientX, y: e.clientY };
    };
    const onUp = (e: PointerEvent) => {
      if (!model || Math.hypot(e.clientX - downAt.x, e.clientY - downAt.y) > 4) return;
      const rect = renderer.domElement.getBoundingClientRect();
      const pointer = new THREE.Vector2(
        ((e.clientX - rect.left) / rect.width) * 2 - 1,
        -((e.clientY - rect.top) / rect.height) * 2 + 1,
      );
      const raycaster = new THREE.Raycaster();
      raycaster.setFromCamera(pointer, camera);
      const hit = raycaster.intersectObject(model, true)[0];
      if (!hit) {
        highlight.visible = false;
        callbacks.current.onSelect(null);
        return;
      }
      const picked = namedObject(hit.object);
      highlight.setFromObject(picked);
      highlight.visible = true;
      callbacks.current.onSelect(picked.name || "unnamed part");
    };
    renderer.domElement.addEventListener("pointerdown", onDown);
    renderer.domElement.addEventListener("pointerup", onUp);

    return () => {
      disposed = true;
      observer.disconnect();
      renderer.setAnimationLoop(null);
      controls.dispose();
      dispose();
      renderer.dispose();
      renderer.domElement.remove();
    };
  }, [path, ext, images]);

  return (
    <div ref={hostRef} className="h-full w-full cursor-crosshair overflow-hidden" style={{ background: BACKDROP }} />
  );
}
