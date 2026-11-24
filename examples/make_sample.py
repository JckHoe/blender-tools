import bpy
from pathlib import Path

SCENES = [("BG_Cycles", "CYCLES", 1, 2), ("CH_Eevee", "BLENDER_EEVEE", 3, 4)]
OUT = Path(__file__).resolve().with_name("sample.blend")

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.preferences.addon_enable(module="cycles")

first = bpy.data.scenes[0]
for i, (name, engine, start, end) in enumerate(SCENES):
    scene = first if i == 0 else bpy.data.scenes.new(name)
    scene.name = name
    scene.render.engine = engine
    scene.frame_start, scene.frame_end = start, end
    scene.render.resolution_x = scene.render.resolution_y = 64
    scene.render.filepath = f"//render/{name}_"

bpy.ops.wm.save_as_mainfile(filepath=str(OUT), compress=True)
print(f"Saved {OUT}")
