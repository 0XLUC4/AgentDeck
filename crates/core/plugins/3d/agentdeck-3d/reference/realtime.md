# Real-time engines

## Contents
- Three.js (WebGPU and WebGL)
- React Three Fiber
- Babylon.js
- Godot 4
- Unity
- Bevy
- Roblox

Check the installed version first; follow the docs of that version.

## Three.js

- **Renderer**: recent releases recommend `WebGPURenderer` (`import * as THREE from "three/webgpu"`),
  which falls back to WebGL 2 by itself. Call `await renderer.init()` before the first `render`, or
  drive frames with `renderer.setAnimationLoop(fn)`. Existing `WebGLRenderer` projects keep working;
  do not migrate unless asked.
- **Shaders with WebGPURenderer**: TSL node materials (`import { ... } from "three/tsl"`), not GLSL
  `ShaderMaterial` / `onBeforeCompile` (WebGL only). Post-processing names changed across releases:
  check the version's examples.
- **Color**: `texture.colorSpace = THREE.SRGBColorSpace` for color maps only; tone mapping
  `AgXToneMapping` or `ACESFilmicToneMapping`.
- **Loading**: `GLTFLoader` with `KTX2Loader` (`detectSupport(renderer)`) and `MeshoptDecoder` for
  compressed assets; `DRACOLoader` only if the files use Draco.
- **Performance**: `InstancedMesh` for copies of one geometry; `BatchedMesh` for many geometries
  sharing a material; check `renderer.info.render.calls` and `.triangles`; `dispose()` geometries,
  materials, textures and render targets you remove; pixel ratio `Math.min(devicePixelRatio, 2)`.
- **Resize**: update `camera.aspect`, `camera.updateProjectionMatrix()`, `renderer.setSize`.

## React Three Fiber

- drei: `useGLTF` (+ `useGLTF.preload`), `Environment` presets, `Instances`, `PerformanceMonitor`,
  `Bvh` for raycasting large scenes. `npx gltfjsx model.glb -t` for a typed component.
- `<Suspense fallback>` around loaders; `frameloop="demand"` for static scenes (call `invalidate()`).
- Never `setState` inside `useFrame`: mutate refs. Never create objects inside `useFrame`.
- Memoize geometries and materials (`useMemo`), share them between meshes.

## Babylon.js

`scene.debugLayer.show()` (Inspector) for scene, materials and stats; `engine.getFps()`,
`SceneInstrumentation` for draw calls and frame time; thin instances for many copies;
`scene.freezeActiveMeshes()` and `material.freeze()` for static content.

## Godot 4

- Headless checks: `godot --headless --path <project> --quit` (import and parse errors),
  scripts or tests via `--script` or the project's test runner (GUT, gdUnit4).
- GDScript 2: typed variables (`var speed: float = 5.0`), `@export`, `@onready`, signals with
  `signal.connect(callable)`.
- Profile with the Debugger > Profiler and Monitors (draw calls, objects); `MultiMeshInstance3D`
  for many copies; occlusion culling and visibility ranges (LOD).

## Unity

- Batch mode: `Unity -batchmode -quit -projectPath <path> -executeMethod <Class.Method> -logFile -`;
  tests: `-runTests -testPlatform EditMode|PlayMode -testResults results.xml`.
- Profiler and Frame Debugger for cost; GPU instancing or the SRP Batcher for draw calls; avoid
  `GetComponent`, `Find` and allocations in `Update`; object pooling.

## Bevy

- Breaking changes every release: read the migration guide for the version in `Cargo.toml`.
- `FrameTimeDiagnosticsPlugin` + `LogDiagnosticsPlugin` for frame time; `--release` (or
  `opt-level = 3` for dependencies in dev) before judging speed.
- Query filters (`Changed<T>`, `With<T>`) instead of scanning every entity each frame.

## Roblox

- Luau with `--!strict` type checking; server logic in `ServerScriptService`, client in
  `StarterPlayerScripts`; never trust the client (validate RemoteEvent arguments on the server).
- MicroProfiler (Ctrl+F6) and the Script Profiler for cost; StreamingEnabled for big worlds;
  fewer parts and unions, MeshParts with simple collision fidelity.
- With a Roblox Studio MCP connected, run Luau and inspect the game tree through it, then playtest
  and read the output.
