---
name: agentdeck-3d
description: Build and debug 3D that looks right and runs smooth - Blender (bpy scripts headless or through the Blender MCP, modeling, materials, UVs, baking, rendering, glTF export), Three.js (WebGPU and WebGL), React Three Fiber, Babylon.js, Godot, Unity, Bevy, Roblox - scenes, cameras, lights, materials, models, shaders, physics, frame rate. Use for any Blender, 3D, game, shader or 3D viewer task, and when a scene or model is black, invisible, wrong or slow.
---

# 3D

**Iron law: look at the result before saying it works.** Render a preview and open the image, or
screenshot the page; measure frame time before and after any performance change.

Details by tool (read the one you need):

- Blender, bpy scripts, versions, export: [reference/blender.md](reference/blender.md)
- Three.js, React Three Fiber, Babylon.js, Godot, Unity, Bevy, Roblox: [reference/realtime.md](reference/realtime.md)

Web 3D runs in the AgentDeck browser panel (`browser` MCP tools): `take_screenshot`,
`list_console_messages`, performance traces.

## Workflow

1. Check versions first (`blender --version`, `three` in `package.json`, engine version): APIs
   change between major versions.
2. Small steps; after each visual change, look (render or screenshot).
3. Name every object, mesh, material; real-world scale (1 unit = 1 m, glTF is Y up).
4. Never overwrite the user's .blend, scene or assets without a copy.

## Nothing on screen or all black: check in this order

1. Console errors: shader compile, failed model or texture load, CORS, wrong path.
2. Canvas size not 0, renderer attached and initialized, render loop running.
3. Camera: not inside the object, looking at it, `near`/`far` covering the scene (near 0.1, far a
   few times the scene size, never near 0).
4. Light: PBR materials are black without light. Ambient plus one directional light, or an
   environment map.
5. Scale: a model 1000x too big or small looks empty. Print its bounding box.
6. Normals: an inverted model looks hollow; test with double-sided materials.

## Looks right

- Color management: color textures sRGB, data textures (normal, roughness, metal, AO) linear;
  output sRGB with a tone mapper (AgX or ACES).
- Lighting: an HDR environment does most of the realism; one shadow-casting light with tight
  shadow bounds and tuned bias (no acne, no peter-panning).
- Edges catch light: small bevels and weighted normals on hard-surface models.
- Anti-aliasing on; device pixel ratio capped at 2.

## Runs smooth (60 fps = 16.6 ms, 120 fps = 8.3 ms per frame)

Measure first (engine profiler, `renderer.info`, browser trace), then fix the biggest cost:

1. **Draw calls** (aim for a few hundred at most): instancing for copies, batching for many meshes
   sharing a material, merged static geometry, shared materials, texture atlases.
2. **Per-frame work**: no allocation in the loop (reuse vectors and matrices), no new materials or
   geometries per frame, update only what moved, render on demand for static viewers.
3. **Triangles**: LOD, decimate distant models, frustum culling, hide what is off screen.
4. **Pixels**: capped pixel ratio, fewer post-processing passes, smaller shadow maps.
5. **Memory**: dispose geometries, materials, textures and render targets you remove; compressed
   textures (KTX2/Basis) with mipmaps.
6. **Physics**: fixed time step, simple colliders, sleeping bodies, heavy work off the main thread.

## Assets

glTF/GLB for real-time. Inspect and compress with `gltf-transform`:
`npx @gltf-transform/cli inspect model.glb`, then
`npx @gltf-transform/cli optimize in.glb out.glb --compress meshopt --texture-compress ktx2`
(`webp` when KTX2 tooling is missing). Validate with the Khronos glTF validator. Budgets: web or
mobile prop under 10k triangles and 1-2K textures; hero model under 100k.

## Shaders

Start from a working shader, change one line at a time, print values as colors to debug. Avoid
branches and texture reads in loops; `highp` for positions.

## Report

Screenshot or render of the result, frame time and draw calls before and after, what changed, and
what was not tested (mobile GPU, other browsers, low-end hardware, other engine versions).
