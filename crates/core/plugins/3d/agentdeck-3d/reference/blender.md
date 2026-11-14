# Blender for agents

## Contents
- Two ways to drive Blender
- Headless scripts
- Version differences that break scripts
- Look at the result
- Clean models, materials, UVs, baking
- Export for real time
- Script skeleton

## Two ways to drive Blender

Python, never descriptions of clicks.

- **Blender MCP connected** (tools like `execute_blender_code`, `get_scene_info`, `look`): call
  `get_scene_info` before editing, run small code steps (big ones time out), call `look` after every
  visual change, and save the .blend before risky code. Assets from Poly Haven / Sketchfab: low
  resolution first; keep only CC0 or credit CC-BY.
- **No MCP**: headless scripts (below). Find Blender: `blender` on PATH; Windows
  `C:/Program Files/Blender Foundation/Blender <ver>/blender.exe`; macOS
  `/Applications/Blender.app/Contents/MacOS/Blender`. Not installed: ask, never install silently.

## Headless scripts

- `blender -b file.blend -P script.py -- args` (`-b` = no window; args after `--` are in
  `sys.argv[sys.argv.index("--") + 1:]`).
- New scene: `blender -b --factory-startup -P script.py`.
- Quick check: `blender -b --python-expr "import bpy; print(bpy.app.version_string)"`.
- Scripts must fail loud: wrap in `try/except`, print the traceback, `sys.exit(1)`. Run with
  `--python-exit-code 1` so a Python error gives a non-zero exit code.
- Build with `bpy.data` and `bmesh` rather than `bpy.ops` (ops depend on context and selection).
  When an op needs context: `with bpy.context.temp_override(object=obj, active_object=obj): ...`.
- Save to a new file: `bpy.ops.wm.save_as_mainfile(filepath=..., copy=True)`.

## Version differences that break scripts

Check `bpy.app.version` and write code for that version.

| Version | Change |
|---|---|
| 4.0 | Principled BSDF inputs renamed: `Specular` → `Specular IOR Level`, `Emission` → `Emission Color`, `Subsurface Color` removed. Look sockets up by name and check they exist. Context override dicts removed: use `temp_override`. |
| 4.1 | `mesh.use_auto_smooth` and `auto_smooth_angle` removed: `bpy.ops.object.shade_auto_smooth()` / the "Smooth by Angle" modifier. |
| 4.2 | EEVEE Next; material `blend_method` replaced by `surface_render_method` (`DITHERED`, `BLENDED`); new shadow and light settings. |
| 5.0 | Legacy action API removed (`action.fcurves`, `action.groups`): use slotted actions and channelbags; `bgl` removed (use `gpu`); dict-style access to property data removed. |

Render engine id changed across versions: pick it from what exists instead of hard-coding it:

```python
engines = {e.identifier for e in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items}
scene.render.engine = next(e for e in ("BLENDER_EEVEE_NEXT", "BLENDER_EEVEE") if e in engines)
```

## Look at the result

Render a small preview to PNG and open it (Read tool) yourself, from several angles when shape
matters:

```python
scene.render.resolution_x, scene.render.resolution_y = 960, 540
scene.render.resolution_percentage = 100
scene.render.filepath = "/tmp/preview.png"   # absolute path
bpy.ops.render.render(write_still=True)
```

Fast previews: EEVEE, low samples. Final: Cycles, GPU if available, denoising on. A scene without a
camera or light renders black: add both for the preview.

## Clean models, materials, UVs, baking

- Apply scale and rotation (`transform_apply`), sensible origin, 1 unit = 1 m, merge by distance,
  normals outside, no n-gons on deforming parts. Print stats (verts, tris) and check face orientation.
- Hard-surface look: Bevel modifier (small width, 2-3 segments, harden normals) + Weighted Normal;
  Mirror for symmetry; Subdivision only with support loops; Solidify for thin parts. Keep the stack
  non-destructive until export.
- Materials for export: Principled BSDF with image textures plugged straight in; procedural nodes do
  not export: bake them. Base color sRGB; normal, roughness, metallic as Non-Color.
- UVs: proper seams or Smart UV Project, no overlap for baked maps, 4-16 px margin. Bake high-poly
  detail (normal, AO) onto the low-poly in Cycles.
- Geometry nodes and instances: realize or apply them before export.
- Lighting and look: HDRI world or 3-point light, AgX view transform, 35-85 mm camera, a ground
  plane to catch shadows.
- Rigs and animation: apply scale before rigging, Rigify for humans, one named action per clip,
  export deform bones only.

## Export for real time

`bpy.ops.export_scene.gltf(filepath=..., export_format="GLB", export_apply=True, use_selection=...)`.
Y up is converted for you: do not rotate by hand. Animations as actions or NLA tracks. Draco only if
the loader is set up for it (prefer meshopt via `gltf-transform` afterwards). Validate the GLB and
check scale and axes by importing it back (`bpy.ops.import_scene.gltf`) or in a viewer.

Imports: `bpy.ops.import_scene.gltf`, `bpy.ops.import_scene.fbx`, `bpy.ops.wm.obj_import`,
`bpy.ops.wm.stl_import`; check scale and axes right after.

## Script skeleton

```python
import bpy, sys, traceback

def find_input(node, *names):
    for name in names:
        if name in node.inputs:
            return node.inputs[name]
    raise KeyError(f"none of {names} on {node.name}")

try:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.mesh.primitive_cube_add(size=2)
    obj = bpy.context.active_object
    obj.name = "Box"
    mat = bpy.data.materials.new("BoxMat")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    find_input(bsdf, "Base Color").default_value = (0.8, 0.2, 0.1, 1)
    obj.data.materials.append(mat)
    bpy.ops.export_scene.gltf(filepath="/tmp/box.glb", export_format="GLB", export_apply=True)
except Exception:
    traceback.print_exc()
    sys.exit(1)
```
