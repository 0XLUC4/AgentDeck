---
name: agentdeck-3d
description: Build and debug 3D that looks right and runs smooth - Blender (modeling, materials, UVs, baking, rendering, export, Python bpy scripts run headless), Three.js, React Three Fiber, Babylon.js, WebGL/WebGPU, Bevy, Godot, Unity, Roblox - scenes, cameras, lights, materials, models (glTF), shaders, physics, and frame-rate work (draw calls, instancing, LOD, textures). Use for any Blender, 3D, game, shader or 3D viewer task, or when a scene or model is black, wrong or slow.
---

# Real-time 3D

Look at the result, measure the frame time, then change one thing. For web 3D use the AgentDeck
browser panel (`browser` MCP tools): screenshot, console, performance trace.

## Blender

Drive Blender with Python scripts, not by describing clicks. If a Blender MCP server is connected, use
it; otherwise run scripts headless and look at the result.

- **Find it**: `blender` on PATH; on Windows usually
  `C:\Program Files\Blender Foundation\Blender <ver>\blender.exe`; macOS
  `/Applications/Blender.app/Contents/MacOS/Blender`. Not installed: ask, do not install silently.
- **Run**: `blender -b file.blend -P script.py -- args` (`-b` = no window; args after `--` are in
  `sys.argv`). New scene: `blender -b --factory-startup -P script.py`. Inline:
  `--python-expr "import bpy; print(bpy.app.version_string)"`. Check the version before using an
  API: bpy changes between 3.x and 4.x (e.g. 4.0 Principled BSDF inputs renamed: `Specular` →
  `Specular IOR Level`, `Emission` → `Emission Color`; 4.2 Eevee Next). Make scripts fail loud:
  wrap in `try/except` that prints the traceback and `sys.exit(1)`, so headless errors are not missed.
- **Script well**: build with `bpy.data` and `bmesh`, not `bpy.ops` when avoidable (ops depend on
  context and selection). Name every object, mesh and material. Save with
  `bpy.ops.wm.save_as_mainfile(filepath=...)`. Never overwrite the user's .blend without a copy.
- **See the result**: render a preview to PNG and open the image (Read tool) to check it yourself.
  Fast preview: Eevee (`BLENDER_EEVEE_NEXT` in 4.2+, `BLENDER_EEVEE` before), low resolution and
  samples. Final: Cycles, GPU if available, denoise on.
- **Clean model**: apply scale and rotation (`transform_apply`), origin where it makes sense, real-world
  size (1 unit = 1 m), merge by distance, recalculate normals outside, no n-gons on deforming parts,
  modifiers applied before export. Check with a face-orientation and stats print (verts, tris).
- **Materials**: Principled BSDF only for anything exported (other nodes do not survive glTF). Image
  textures: base color sRGB, normal/roughness/metallic as Non-Color.
- **UVs and baking**: Smart UV Project or proper seams, no overlap for baked maps, margin 4-16 px.
  Bake high-poly detail (normal, AO) onto a low-poly in Cycles for game and web assets.
- **Export for real-time**: glTF 2.0 (`.glb`) with +Y up, apply modifiers, selected objects only,
  animations as NLA strips or actions. Target budgets: web/mobile prop under 10k tris and 1-2K
  textures; hero model under 100k tris. Decimate or remesh, then compress with `gltf-transform`.
- **Optimize a scene**: fewer, joined objects sharing materials; instances (`Alt+D` / linked
  duplicates, geometry nodes instancing) instead of copies; LOD by Decimate modifier copies.
- **Modeling that looks good**: start from blockout primitives, then Bevel modifier (small width,
  2-3 segments, harden normals) so edges catch light; Weighted Normal modifier + Auto Smooth (4.1+:
  "Smooth by Angle") for hard-surface; Mirror for symmetry; Subdivision Surface only with support
  loops; Solidify for thin parts. Keep the modifier stack non-destructive until export.
- **Procedural** (geometry nodes, procedural shader nodes): great for variations, but glTF and game
  engines do not read them: realize instances / apply, and bake procedural materials to image textures
  before export.
- **Lighting and render look**: 3-point light or an HDRI world, Filmic/AgX view transform, a
  camera with a real focal length (35-85 mm), ground plane or backdrop to catch shadows. Turntable:
  empty at center, camera parented, rotate 360° over N frames.
- **Animation and rigs**: one action per clip, named; apply scale before rigging; Rigify for humans;
  check weights by posing; export with only deform bones; 30 fps unless the target says otherwise.
- **Import**: `bpy.ops.import_scene.gltf`, `.fbx`, `wm.obj_import` (4.x), `wm.stl_import`; check
  scale and axes right after import.
- **Minimal script skeleton**:

  ```python
  import bpy, sys, traceback
  try:
      bpy.ops.wm.read_factory_settings(use_empty=True)
      bpy.ops.mesh.primitive_cube_add(size=2)
      obj = bpy.context.active_object; obj.name = "Box"
      mat = bpy.data.materials.new("BoxMat"); mat.use_nodes = True
      mat.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.8, 0.2, 0.1, 1)
      obj.data.materials.append(mat)
      bpy.ops.export_scene.gltf(filepath="//box.glb", export_format="GLB", export_apply=True)
  except Exception:
      traceback.print_exc(); sys.exit(1)
  ```

  Preview render: set `scene.render.filepath`, `resolution_percentage = 50`, then
  `bpy.ops.render.render(write_still=True)`.

## Get it on screen first

When nothing shows or the scene is black, check in this order:

1. Console errors (shader compile, failed model or texture load, CORS, wrong path).
2. Canvas size not 0, renderer attached, render loop running.
3. Camera: position not inside the object, looking at it, `near`/`far` covering the scene
   (near 0.1, far a few times the scene size, never near 0).
4. Lights: standard/PBR materials are black without light. Add ambient plus one directional light,
   or an environment map.
5. Scale and units: glTF is meters, Y up. A model 1000x too big or small looks empty.
6. Normals and face side: inverted model looks hollow; test with `side: DoubleSide`.

## Look right

- Color: textures for color in sRGB, data textures (normal, roughness, metal) linear; renderer output
  sRGB with tone mapping (ACES or AgX).
- Lighting: an HDR environment map gives most of the realism; one shadow-casting light, tight shadow
  camera bounds, shadow bias tuned against acne and peter-panning.
- Models: glTF/GLB. Compress with `gltf-transform optimize in.glb out.glb --texture-compress webp`
  (Draco or Meshopt geometry, KTX2/WebP textures). Inspect with `gltf-transform inspect`.
- React Three Fiber: `useGLTF` from drei (with `useGLTF.preload`), `npx gltfjsx model.glb -t` to get
  a typed component, `<Environment preset>` for lighting, `<Suspense>` with a fallback,
  `frameloop="demand"` for static scenes, never `setState` inside `useFrame` (mutate refs).
- Anti-aliasing on, device pixel ratio capped at 2.

## Run smooth (target 60 fps = 16.6 ms per frame, 120 fps = 8.3 ms)

Measure first: browser performance trace, `renderer.info` (calls, triangles, textures), stats panel,
or the engine's profiler (Bevy diagnostics, Godot monitors, Unity profiler, Roblox MicroProfiler).
Then fix the biggest cost, in this order:

1. **Draw calls** (aim under a few hundred): `InstancedMesh` for repeated objects, merge static
   geometry, share materials, texture atlases.
2. **Work per frame**: no allocation in the render loop (reuse vectors, matrices), no new materials
   or geometries per frame, update only what moved, render on demand for static viewers.
3. **Triangles**: LOD levels, decimate distant models, frustum culling on, hide what is off-screen.
4. **Pixels**: cap pixel ratio, fewer post-processing passes, smaller shadow maps, cheaper materials
   for far objects.
5. **Memory**: dispose geometries, materials, textures and render targets you remove; power-of-two
   compressed textures; mipmaps on.
6. **Physics and logic**: fixed time step, simple colliders (box, sphere, capsule), sleep inactive
   bodies, heavy work in a worker.

## Shaders

Start from a working shader and change one line at a time. Precision `highp` for positions. Avoid
branches and texture reads in loops. Print values as colors to debug (`gl_FragColor = vec4(v, 0, 0, 1)`).

## Report

Screenshot of the result, frame time and draw calls before and after, what you changed. Say what you
did not test (mobile GPU, other browsers, low-end hardware).
