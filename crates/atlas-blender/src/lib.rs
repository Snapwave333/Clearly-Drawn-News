use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{info, warn};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShotSpec {
    pub shot_id: String,
    pub project_id: String,
    pub engine: RenderEngine,
    pub scene_template: String,
    pub duration_frames: u32,
    pub fps: u32,
    pub claim_ids: Vec<String>,
    pub narration_segment_ids: Vec<String>,
    pub visual_elements: Vec<VisualElement>,
    pub camera: CameraSpec,
    pub transitions: Vec<TransitionSpec>,
    #[serde(default)]
    pub mascot: Option<MascotSpec>,
    #[serde(default)]
    pub background_media: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MascotSpec {
    pub name: String,
    pub emotion: MascotEmotion,
    pub gesture: MascotGesture,
    pub lip_sync: Vec<LipSyncCue>,
    pub morph: Option<MascotMorph>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MascotMorph {
    pub label: String,
    pub silhouette: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MascotEmotion {
    Curious,
    Excited,
    Explaining,
    Concerned,
    Confident,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MascotGesture {
    PointLeft,
    PointRight,
    Present,
    Think,
    Warn,
    Celebrate,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LipSyncCue {
    pub frame: u32,
    pub viseme: Viseme,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Viseme {
    Rest,
    Closed,
    Open,
    Wide,
    Round,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualElement {
    pub element_id: String,
    pub element_type: VisualElementType,
    pub reveal_frame: u32,
    pub hold_frames: u32,
    pub properties: serde_json::Value,
    pub transform: TransformSpec,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualElementType {
    StrokeReveal,
    Shape,
    Text,
    Diagram,
    Arrow,
    Label,
    CrossSection,
    Graph,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransformSpec {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CameraSpec {
    pub location: [f32; 3],
    pub rotation: [f32; 3],
    pub lens: f32,
    pub ortho_scale: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransitionSpec {
    pub from_frame: u32,
    pub to_frame: u32,
    pub transition_type: TransitionType,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionType {
    Morph,
    Fade,
    Cut,
    PanZoom,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderEngine {
    Blender,
    Unreal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderJob {
    pub job_id: String,
    pub shot_spec: ShotSpec,
    pub quality: RenderQuality,
    pub output_dir: PathBuf,
    pub status: RenderStatus,
    #[serde(default)]
    pub output_size: Option<[u32; 2]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderQuality {
    Preview,
    Final,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderStatus {
    Queued,
    Running,
    Completed,
    Failed { error: String },
}

pub struct BlenderPipeline {
    blender_exe: PathBuf,
    workspace_dir: PathBuf,
}

impl BlenderPipeline {
    pub fn new(blender_exe: impl Into<PathBuf>, workspace_dir: impl Into<PathBuf>) -> Self {
        Self {
            blender_exe: blender_exe.into(),
            workspace_dir: workspace_dir.into(),
        }
    }

    pub fn build_script(&self, spec: &ShotSpec, quality: RenderQuality) -> AtlasResult<String> {
        self.build_script_for_size(spec, quality, None)
    }

    fn build_script_for_size(
        &self,
        spec: &ShotSpec,
        quality: RenderQuality,
        output_size: Option<[u32; 2]>,
    ) -> AtlasResult<String> {
        if !matches!(spec.engine, RenderEngine::Blender) {
            return Err(AtlasError::Render(
                "The Blender pipeline cannot render an Unreal shot".into(),
            ));
        }
        if spec.duration_frames == 0 || spec.fps == 0 {
            return Err(AtlasError::Render(
                "Shot duration and frame rate must be greater than zero".into(),
            ));
        }

        let (width, height) = match output_size {
            Some([width, height]) if width > 0 && height > 0 => (width, height),
            _ => match quality {
                RenderQuality::Preview => (960, 540),
                RenderQuality::Final => (1920, 1080),
            },
        };
        let output_dir = self
            .workspace_dir
            .join("renders")
            .join(&spec.project_id)
            .join(&spec.shot_id);
        let mut script = render_header(spec, &output_dir, width, height);
        if let Some(path) = &spec.background_media {
            script.push_str(&background_media_script(
                path,
                spec.duration_frames,
                width,
                height,
            ));
        }
        script.push_str(&scene_script(spec, width < height));
        for element in &spec.visual_elements {
            script.push_str(&element_script(element)?);
        }
        if let Some(mascot) = &spec.mascot {
            script.push_str(&mascot_script(mascot, spec.duration_frames, width < height));
        }
        script.push_str(&camera_script(&spec.camera));
        script.push_str("bpy.ops.render.render(animation=True)\n");
        Ok(script)
    }

    pub async fn render(&self, job: &RenderJob) -> AtlasResult<RenderOutput> {
        let output_dir = self
            .workspace_dir
            .join("renders")
            .join(&job.shot_spec.project_id)
            .join(&job.shot_spec.shot_id);
        tokio::fs::create_dir_all(&output_dir).await?;
        let mut existing = tokio::fs::read_dir(&output_dir).await?;
        while let Some(entry) = existing.next_entry().await? {
            if entry.path().extension().is_some_and(|value| value == "png") {
                tokio::fs::remove_file(entry.path()).await?;
            }
        }

        let script =
            self.build_script_for_size(&job.shot_spec, job.quality.clone(), job.output_size)?;
        let script_path = self
            .workspace_dir
            .join("temp")
            .join(format!("render_{}.py", job.job_id));
        let parent = script_path.parent().ok_or_else(|| {
            AtlasError::Render(format!("Invalid script path: {}", script_path.display()))
        })?;
        tokio::fs::create_dir_all(parent).await?;
        tokio::fs::write(&script_path, script).await?;

        info!("Starting Blender render: {}", job.job_id);
        let output = Command::new(&self.blender_exe)
            .arg("--background")
            .arg("--python")
            .arg(&script_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| AtlasError::Render(format!("Failed to start Blender: {error}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success()
            || stdout.contains("Traceback (most recent call last)")
            || stderr.contains("Traceback (most recent call last)")
        {
            warn!("Blender render failed: {}", stderr);
            return Err(AtlasError::Render(format!(
                "Blender exited with {}\n{}\n{}",
                output.status, stdout, stderr
            )));
        }

        let frames = collect_frames(&output_dir, job.shot_spec.duration_frames).await?;
        Ok(RenderOutput {
            job_id: job.job_id.clone(),
            frames,
            duration_frames: job.shot_spec.duration_frames,
            fps: job.shot_spec.fps,
        })
    }
}

fn render_header(spec: &ShotSpec, output_dir: &Path, width: u32, height: u32) -> String {
    let output_dir = py_string(&output_dir.to_string_lossy());
    let safe_text_width = if width < height { 4.8 } else { 10.5 };
    format!(
        r#"import bpy
import os
from mathutils import Vector

bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
scene = bpy.context.scene
scene.render.engine = 'BLENDER_EEVEE'
scene.render.resolution_x = {width}
scene.render.resolution_y = {height}
scene.render.resolution_percentage = 100
scene.render.fps = {fps}
scene.frame_start = 1
scene.frame_end = {duration}
scene.render.image_settings.file_format = 'PNG'
scene.render.image_settings.color_mode = 'RGB'
scene.render.image_settings.color_depth = '8'
scene.render.film_transparent = False
scene.render.filepath = os.path.join({output_dir}, '')
scene.view_settings.look = 'AgX - Medium High Contrast'
scene.world.color = (0.955, 0.945, 0.91)

def material(name, color):
    value = bpy.data.materials.get(name)
    if value is None:
        value = bpy.data.materials.new(name)
        value.diffuse_color = (*color, 1.0)
        value.use_nodes = True
        nodes = value.node_tree.nodes
        nodes.clear()
        output = nodes.new('ShaderNodeOutputMaterial')
        emission = nodes.new('ShaderNodeEmission')
        emission.inputs['Color'].default_value = (*color, 1.0)
        emission.inputs['Strength'].default_value = 1.0
        value.node_tree.links.new(emission.outputs['Emission'], output.inputs['Surface'])
    return value

INK = material('Ink', (0.035, 0.05, 0.08))
BLUE = material('Blue', (0.04, 0.36, 0.72))
ORANGE = material('Orange', (0.95, 0.38, 0.08))
YELLOW = material('Yellow', (1.0, 0.72, 0.08))
WHITE = material('White', (1.0, 0.99, 0.95))
PAPER = material('Paper', (0.96, 0.94, 0.88))

def reveal(obj, frame):
    obj.hide_render = True
    obj.keyframe_insert(data_path='hide_render', frame=max(1, frame - 1))
    obj.hide_render = False
    obj.keyframe_insert(data_path='hide_render', frame=frame)
    original_scale = obj.scale.copy()
    obj.scale = original_scale * 0.86
    obj.keyframe_insert(data_path='scale', frame=frame)
    obj.scale = original_scale
    obj.keyframe_insert(data_path='scale', frame=min(scene.frame_end, frame + 8))

def add_text(name, body, location, size, frame, color=INK):
    data = bpy.data.curves.new(name + '_data', 'FONT')
    data.body = body
    data.align_x = 'CENTER'
    data.align_y = 'CENTER'
    data.size = size
    data.extrude = 0.008
    obj = bpy.data.objects.new(name, data)
    bpy.context.collection.objects.link(obj)
    obj.location = location
    obj.data.materials.append(color)
    bpy.context.view_layer.update()
    width = max(corner[0] for corner in obj.bound_box) - min(corner[0] for corner in obj.bound_box)
    if width > {safe_text_width}:
        fit = {safe_text_width} / width
        obj.scale = obj.scale * fit
    reveal(obj, frame)
    return obj

def add_box(name, location, scale, frame, color):
    bpy.ops.mesh.primitive_cube_add(location=location)
    obj = bpy.context.object
    obj.name = name
    obj.scale = scale
    obj.data.materials.append(color)
    bevel = obj.modifiers.new('Soft edges', 'BEVEL')
    bevel.width = 0.08
    bevel.segments = 4
    reveal(obj, frame)
    return obj

def add_circle(name, location, radius, frame, color):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=64, ring_count=32, location=location)
    obj = bpy.context.object
    obj.name = name
    obj.scale = (radius, radius, 0.08)
    obj.data.materials.append(color)
    reveal(obj, frame)
    return obj

def add_polygon(name, points, z, frame, color):
    mesh = bpy.data.meshes.new(name + '_mesh')
    vertices = [(point[0], point[1], z) for point in points]
    mesh.from_pydata(vertices, [], [list(range(len(vertices)))])
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    obj.data.materials.append(color)
    bevel = obj.modifiers.new('Cel bevel', 'BEVEL')
    bevel.width = 0.045
    bevel.segments = 3
    reveal(obj, frame)
    return obj

def add_line(name, points, frame, color=INK, width=0.035):
    data = bpy.data.curves.new(name + '_data', 'CURVE')
    data.dimensions = '3D'
    data.bevel_depth = width
    data.bevel_resolution = 4
    spline = data.splines.new('POLY')
    spline.points.add(len(points) - 1)
    for point, coordinate in zip(spline.points, points):
        point.co = (*coordinate, 1.0)
    obj = bpy.data.objects.new(name, data)
    bpy.context.collection.objects.link(obj)
    obj.data.materials.append(color)
    reveal(obj, frame)
    return obj

def add_arrow(name, start, end, frame, color=ORANGE):
    add_line(name + '_shaft', [start, end], frame, color, 0.045)
    direction = Vector(end) - Vector(start)
    bpy.ops.mesh.primitive_cone_add(vertices=32, radius1=0.16, radius2=0.0, depth=0.38, location=end)
    head = bpy.context.object
    head.name = name + '_head'
    head.rotation_euler = direction.to_track_quat('Z', 'Y').to_euler()
    head.data.materials.append(color)
    reveal(head, frame)
    return head

add_box('whiteboard_background', (0.0, 0.0, -1.0), (8.0, 5.0, 0.05), 1, PAPER)

"#,
        width = width,
        height = height,
        safe_text_width = safe_text_width,
        fps = spec.fps,
        duration = spec.duration_frames,
        output_dir = output_dir,
    )
}

fn background_media_script(path: &Path, duration_frames: u32, width: u32, height: u32) -> String {
    let source = py_string(&path.to_string_lossy());
    let scale_x = if width < height { 2.82 } else { 5.0 };
    let scale_y = 5.0;
    let is_movie = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "mp4" | "mov" | "webm"));
    let movie_setup = if is_movie {
        format!(
            "image.source = 'MOVIE'\ntexture.image_user.use_auto_refresh = True\ntexture.image_user.frame_duration = {duration_frames}\n"
        )
    } else {
        String::new()
    };
    format!(
        "image = bpy.data.images.load({source}, check_existing=True)\nmedia_material = bpy.data.materials.new('Background media')\nmedia_material.use_nodes = True\nnodes = media_material.node_tree.nodes\nnodes.clear()\nmedia_output = nodes.new('ShaderNodeOutputMaterial')\nmedia_emission = nodes.new('ShaderNodeEmission')\ntexture = nodes.new('ShaderNodeTexImage')\ntexture.image = image\n{movie_setup}mix = nodes.new('ShaderNodeMixRGB')\nmix.blend_type = 'MIX'\nmix.inputs[0].default_value = 0.58\nmix.inputs[2].default_value = (0.96, 0.94, 0.88, 1.0)\nmedia_material.node_tree.links.new(texture.outputs['Color'], mix.inputs[1])\nmedia_material.node_tree.links.new(mix.outputs['Color'], media_emission.inputs['Color'])\nmedia_material.node_tree.links.new(media_emission.outputs['Emission'], media_output.inputs['Surface'])\nadd_box('background_media', (0.0, 0.0, -0.8), ({scale_x}, {scale_y}, 0.03), 1, media_material)\n"
    )
}

fn scene_script(spec: &ShotSpec, portrait: bool) -> String {
    let left = if portrait { -0.9 } else { -2.3 };
    let right = if portrait { 0.9 } else { 2.3 };
    let top = if portrait { 2.0 } else { 1.2 };
    let bottom = if portrait { -1.3 } else { -1.0 };
    let end = spec.duration_frames;
    if portrait && spec.scene_template == "battery_hook" {
        return format!(
            "add_box('phone', (0.0, 0.65, 0.1), (0.68, 1.05, 0.08), 12, INK)\nadd_box('screen', (0.0, 0.65, 0.2), (0.54, 0.86, 0.04), 18, PAPER)\ncharge = add_box('charge', (0.0, 0.55, 0.3), (0.38, 0.5, 0.03), 24, ORANGE)\ncharge.scale.y = 0.5\ncharge.keyframe_insert(data_path='scale', frame=24)\ncharge.scale.y = 0.08\ncharge.keyframe_insert(data_path='scale', frame={})\nadd_text('hook_question', 'WHERE DID THE ENERGY GO?', (0.0, -0.65, 0.2), 0.36, 30, INK)\nadd_text('hook_answer', 'Ions changed where they prefer to be.', (0.0, -1.2, 0.2), 0.25, 70, BLUE)\n",
            end.saturating_sub(18)
        );
    }
    if portrait && spec.scene_template == "battery_aging" {
        return format!(
            "add_box('fresh_electrode', (0.0, 0.45, 0.1), (0.75, 1.0, 0.06), 18, BLUE)\nfilm = add_box('sei_layer', (0.88, 0.45, 0.2), (0.05, 1.0, 0.04), 45, ORANGE)\nfilm.scale.x = 0.05\nfilm.keyframe_insert(data_path='scale', frame=45)\nfilm.scale.x = 0.24\nfilm.keyframe_insert(data_path='scale', frame={})\nadd_text('aging_label', 'SIDE REACTIONS BUILD A BARRIER', (0.0, -0.9, 0.2), 0.3, 55, INK)\nadd_text('aging_result', 'Fewer ions move. Capacity falls.', (0.0, -1.35, 0.2), 0.25, 80, ORANGE)\n",
            end.saturating_sub(20)
        );
    }
    match spec.scene_template.as_str() {
        "battery_hook" => format!(
            "add_box('phone', ({left}, 0.0, 0.1), (0.75, 1.25, 0.08), 12, INK)\nadd_box('screen', ({left}, 0.0, 0.2), (0.6, 1.05, 0.04), 18, PAPER)\ncharge = add_box('charge', ({left}, -0.15, 0.3), (0.42, 0.62, 0.03), 24, ORANGE)\ncharge.scale.y = 0.62\ncharge.keyframe_insert(data_path='scale', frame=24)\ncharge.scale.y = 0.08\ncharge.keyframe_insert(data_path='scale', frame={})\nadd_text('hook_question', 'WHERE DID THE ENERGY GO?', ({right}, 0.35, 0.2), 0.48, 30, INK)\nadd_text('hook_answer', 'It changed where ions prefer to be.', ({right}, -0.45, 0.2), 0.3, 70, BLUE)\n",
            end.saturating_sub(18)
        ),
        "battery_anatomy" => format!(
            "add_box('anode', ({left}, 0.0, 0.1), (0.72, 1.15, 0.06), 18, BLUE)\nadd_box('separator', (0.0, 0.0, 0.15), (0.12, 1.25, 0.06), 28, ORANGE)\nadd_box('cathode', ({right}, 0.0, 0.1), (0.72, 1.15, 0.06), 38, INK)\nadd_text('anode_name', 'ANODE', ({left}, {bottom}, 0.2), 0.3, 45, BLUE)\nadd_text('separator_name', 'SEPARATOR', (0.0, {top}, 0.2), 0.25, 55, ORANGE)\nadd_text('cathode_name', 'CATHODE', ({right}, {bottom}, 0.2), 0.3, 65, INK)\n"
        ),
        "battery_electrolyte" => format!(
            "add_box('electrolyte_field', (0.0, 0.0, 0.05), (2.15, 1.2, 0.04), 16, BLUE)\nadd_box('separator_gate', (0.0, 0.0, 0.15), (0.12, 1.35, 0.05), 24, ORANGE)\nadd_text('ion_pass', 'IONS PASS', ({left}, {top}, 0.2), 0.3, 32, BLUE)\nadd_text('electron_stop', 'ELECTRONS STOP', ({right}, {top}, 0.2), 0.3, 42, ORANGE)\nion = add_circle('passing_ion', ({left}, -0.35, 0.3), 0.16, 48, ORANGE)\nion.location = ({left}, -0.35, 0.3)\nion.keyframe_insert(data_path='location', frame=48)\nion.location = ({right}, -0.35, 0.3)\nion.keyframe_insert(data_path='location', frame={})\nadd_arrow('blocked_electron', ({right}, 0.5, 0.3), (0.3, 0.5, 0.3), 56, INK)\n",
            end.saturating_sub(20)
        ),
        "battery_charging" => {
            particle_flow_script(left, right, true, end, "CHARGER PUSHES IONS INTO THE ANODE")
        }
        "battery_electrons" => format!(
            "add_box('cell_body', (0.0, -0.25, 0.1), (2.25, 0.9, 0.05), 16, BLUE)\nadd_line('external_wire', [({left}, 0.65, 0.2), ({left}, {top}, 0.2), ({right}, {top}, 0.2), ({right}, 0.65, 0.2)], 24, INK, 0.06)\nadd_text('wire_label', 'THE OUTSIDE ROAD', (0.0, {} , 0.2), 0.32, 32, INK)\nelectron = add_circle('moving_electron', ({left}, {top}, 0.3), 0.15, 40, ORANGE)\nelectron.location = ({left}, {top}, 0.3)\nelectron.keyframe_insert(data_path='location', frame=40)\nelectron.location = (0.0, {top}, 0.3)\nelectron.keyframe_insert(data_path='location', frame={})\nelectron.location = ({right}, {top}, 0.3)\nelectron.keyframe_insert(data_path='location', frame={})\n",
            top + 0.55,
            end / 2,
            end.saturating_sub(18)
        ),
        "battery_discharging" => {
            particle_flow_script(left, right, false, end, "ELECTRONS POWER YOUR DEVICE")
        }
        "battery_voltage" => format!(
            "add_line('energy_hill', [({left}, {bottom}, 0.1), (0.0, {top}, 0.1), ({right}, {bottom}, 0.1)], 20, BLUE, 0.06)\nball = add_circle('energy_ball', ({left}, {bottom}, 0.25), 0.22, 28, ORANGE)\nball.location = ({left}, {bottom}, 0.25)\nball.keyframe_insert(data_path='location', frame=28)\nball.location = (0.0, {top}, 0.25)\nball.keyframe_insert(data_path='location', frame={})\nadd_text('voltage_label', 'VOLTAGE = THE ENERGY DIFFERENCE', (0.0, {}, 0.2), 0.34, 45, INK)\n",
            end.saturating_sub(20),
            top + 0.65
        ),
        "battery_aging" => format!(
            "add_box('fresh_electrode', ({left}, 0.0, 0.1), (0.7, 1.2, 0.06), 18, BLUE)\nfilm = add_box('sei_layer', ({left} + 0.82, 0.0, 0.2), (0.05, 1.2, 0.04), 45, ORANGE)\nfilm.scale.x = 0.05\nfilm.keyframe_insert(data_path='scale', frame=45)\nfilm.scale.x = 0.24\nfilm.keyframe_insert(data_path='scale', frame={})\nadd_text('aging_label', 'SIDE REACTIONS BUILD A BARRIER', ({right}, 0.45, 0.2), 0.36, 55, INK)\nadd_text('aging_result', 'Fewer ions move. Capacity falls.', ({right}, -0.35, 0.2), 0.3, 80, ORANGE)\n",
            end.saturating_sub(20)
        ),
        "battery_safety" => format!(
            "add_box('separator_safe', (0.0, 0.0, 0.15), (0.14, 1.35, 0.06), 20, ORANGE)\nadd_text('keep_apart', 'THE SEPARATOR KEEPS ELECTRODES APART', (0.0, {top}, 0.2), 0.34, 30, INK)\nadd_arrow('left_force', ({left}, 0.0, 0.2), (-0.3, 0.0, 0.2), 50, BLUE)\nadd_arrow('right_force', ({right}, 0.0, 0.2), (0.3, 0.0, 0.2), 50, BLUE)\nadd_text('safety_result', 'Damage can create a short circuit and heat.', (0.0, {bottom}, 0.2), 0.3, 80, ORANGE)\n"
        ),
        "battery_summary" => format!(
            "add_circle('cycle_left', ({left}, 0.0, 0.15), 0.7, 20, BLUE)\nadd_circle('cycle_right', ({right}, 0.0, 0.15), 0.7, 25, INK)\nadd_arrow('cycle_top', ({left} + 0.7, 0.45, 0.2), ({right} - 0.7, 0.45, 0.2), 35, ORANGE)\nadd_arrow('cycle_bottom', ({right} - 0.7, -0.45, 0.2), ({left} + 0.7, -0.45, 0.2), 55, BLUE)\nadd_text('charge_word', 'CHARGE', (0.0, 0.9, 0.2), 0.3, 40, ORANGE)\nadd_text('use_word', 'USE', (0.0, -0.9, 0.2), 0.3, 60, BLUE)\n"
        ),
        _ => String::new(),
    }
}

fn particle_flow_script(left: f32, right: f32, charging: bool, end: u32, label: &str) -> String {
    let (start, finish, color) = if charging {
        (right, left, "ORANGE")
    } else {
        (left, right, "BLUE")
    };
    let mut script = format!(
        "add_box('left_electrode', ({left}, 0.0, 0.1), (0.65, 1.2, 0.06), 15, BLUE)\nadd_box('right_electrode', ({right}, 0.0, 0.1), (0.65, 1.2, 0.06), 15, INK)\nadd_text('flow_label', {}, (0.0, 1.65, 0.2), 0.34, 24, INK)\n",
        py_string(label)
    );
    for index in 0..5 {
        let y = -0.8 + index as f32 * 0.4;
        let begin = 38 + index * 9;
        let finish_frame = end.saturating_sub(22).max(begin + 12);
        script.push_str(&format!(
            "ion_{index} = add_circle('ion_{index}', ({start}, {y}, 0.3), 0.13, {begin}, {color})\nion_{index}.location = ({start}, {y}, 0.3)\nion_{index}.keyframe_insert(data_path='location', frame={begin})\nion_{index}.location = ({finish}, {y}, 0.3)\nion_{index}.keyframe_insert(data_path='location', frame={finish_frame})\n"
        ));
    }
    script
}

fn mascot_script(mascot: &MascotSpec, duration_frames: u32, portrait: bool) -> String {
    let x = if portrait { 0.0 } else { 4.3 };
    let y = if portrait { -3.0 } else { -0.45 };
    let side = match mascot.gesture {
        MascotGesture::PointLeft | MascotGesture::Warn => -1.0,
        _ => 1.0,
    };
    let eye_scale = match mascot.emotion {
        MascotEmotion::Excited => 0.14,
        MascotEmotion::Concerned => 0.09,
        _ => 0.12,
    };
    let mut cues = String::new();
    for cue in &mascot.lip_sync {
        let (width, height) = match cue.viseme {
            Viseme::Rest => (0.22, 0.035),
            Viseme::Closed => (0.2, 0.025),
            Viseme::Open => (0.18, 0.19),
            Viseme::Wide => (0.3, 0.09),
            Viseme::Round => (0.14, 0.16),
        };
        cues.push_str(&format!(
            "mouth.scale = ({width}, {height}, 0.03)\nmouth.keyframe_insert(data_path='scale', frame={})\n",
            cue.frame.max(1).min(duration_frames)
        ));
    }
    let pose_frame = duration_frames / 4;
    let settle_frame = duration_frames / 2;
    let blink_frame = duration_frames / 3;
    let blink_open_frame = blink_frame + 4;
    let morph_start = duration_frames * 5 / 10;
    let morph_peak = duration_frames * 6 / 10;
    let morph_return = duration_frames * 8 / 10;
    let morph_end = duration_frames * 9 / 10;
    let morph_script = mascot.morph.as_ref().map_or_else(String::new, |morph| {
        if morph.silhouette.len() != 11 {
            return String::new();
        }
        let points = morph
            .silhouette
            .iter()
            .map(|[px, py]| {
                let target_x = x + px.clamp(-1.0, 1.0) * 0.68;
                let target_y = y + 0.68 + py.clamp(-1.0, 1.0) * 0.62;
                format!("({target_x}, {target_y}, 0.56)")
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "body.shape_key_add(name='Basis')\nmorph_key = body.shape_key_add(name='IdeaMorph')\nmorph_points = [{points}]\nfor vertex, target in zip(morph_key.data, morph_points):\n    vertex.co = target\nmorph_key.value = 0.0\nmorph_key.keyframe_insert(data_path='value', frame={morph_start})\nmorph_key.value = 1.0\nmorph_key.keyframe_insert(data_path='value', frame={morph_peak})\nmorph_key.keyframe_insert(data_path='value', frame={morph_return})\nmorph_key.value = 0.0\nmorph_key.keyframe_insert(data_path='value', frame={morph_end})\n"
        )
    });
    let rig_script = format!(
        "root = bpy.data.objects.new('luma_rig', None)\nbpy.context.collection.objects.link(root)\nglow = add_circle('luma_glow', ({x}, {y} + 0.72, 0.48), 0.9, 1, YELLOW)\nbody = add_polygon('luma_glass', [({x} - 0.38, {y} + 0.08), ({x} - 0.55, {y} + 0.3), ({x} - 0.67, {y} + 0.62), ({x} - 0.61, {y} + 0.93), ({x} - 0.38, {y} + 1.18), ({x}, {y} + 1.3), ({x} + 0.38, {y} + 1.18), ({x} + 0.61, {y} + 0.93), ({x} + 0.67, {y} + 0.62), ({x} + 0.55, {y} + 0.3), ({x} + 0.38, {y} + 0.08)], 0.56, 1, YELLOW)\nfilament = add_line('luma_filament', [({x} - 0.22, {y} + 0.2, 0.62), ({x} - 0.1, {y} + 0.42, 0.62), ({x}, {y} + 0.3, 0.62), ({x} + 0.1, {y} + 0.42, 0.62), ({x} + 0.22, {y} + 0.2, 0.62)], 2, ORANGE, 0.035)\nbase_top = add_box('luma_base_top', ({x}, {y} - 0.02, 0.58), (0.4, 0.09, 0.05), 2, INK)\nbase_mid = add_box('luma_base_mid', ({x}, {y} - 0.2, 0.58), (0.34, 0.08, 0.05), 3, INK)\nbase_tip = add_box('luma_base_tip', ({x}, {y} - 0.36, 0.58), (0.25, 0.07, 0.05), 4, INK)\nleft_white = add_circle('luma_left_eye_white', ({x} - 0.22, {y} + 0.78, 0.7), 0.19, 4, WHITE)\nleft_white.scale.x = 0.78\nright_white = add_circle('luma_right_eye_white', ({x} + 0.22, {y} + 0.78, 0.7), 0.19, 4, WHITE)\nright_white.scale.x = 0.78\nleft_eye = add_circle('luma_left_pupil', ({x} - 0.19, {y} + 0.76, 0.75), {eye_scale}, 5, INK)\nright_eye = add_circle('luma_right_pupil', ({x} + 0.25, {y} + 0.76, 0.75), {eye_scale}, 5, INK)\nleft_brow = add_line('luma_left_brow', [({x} - 0.36, {y} + 1.02, 0.76), ({x} - 0.09, {y} + 1.06, 0.76)], 6, INK, 0.035)\nright_brow = add_line('luma_right_brow', [({x} + 0.09, {y} + 1.06, 0.76), ({x} + 0.36, {y} + 1.02, 0.76)], 6, INK, 0.035)\nmouth = add_circle('luma_mouth', ({x}, {y} + 0.49, 0.75), 0.2, 5, INK)\nmouth.scale = (0.22, 0.035, 0.03)\nactive_arm = add_line('luma_arm_active', [({x} + ({side} * 0.5), {y} + 0.28, 0.6), ({x} + ({side} * 0.78), {y} - 0.05, 0.6)], 8, INK, 0.055)\nrest_arm = add_line('luma_arm_rest', [({x} - ({side} * 0.5), {y} + 0.28, 0.6), ({x} - ({side} * 0.82), {y} - 0.12, 0.6)], 8, INK, 0.055)\nactive_glove = add_circle('luma_active_glove', ({x} + ({side} * 0.8), {y} - 0.08, 0.66), 0.18, 8, WHITE)\nrest_glove = add_circle('luma_rest_glove', ({x} - ({side} * 0.84), {y} - 0.15, 0.66), 0.18, 8, WHITE)\nleft_leg = add_line('luma_leg_left', [({x} - 0.16, {y} - 0.43, 0.6), ({x} - 0.34, {y} - 0.78, 0.6)], 8, INK, 0.06)\nright_leg = add_line('luma_leg_right', [({x} + 0.16, {y} - 0.43, 0.6), ({x} + 0.34, {y} - 0.78, 0.6)], 8, INK, 0.06)\nleft_shoe = add_circle('luma_left_shoe', ({x} - 0.4, {y} - 0.84, 0.65), 0.2, 9, INK)\nleft_shoe.scale.y = 0.52\nright_shoe = add_circle('luma_right_shoe', ({x} + 0.4, {y} - 0.84, 0.65), 0.2, 9, INK)\nright_shoe.scale.y = 0.52\nrig_parts = [glow, body, filament, base_top, base_mid, base_tip, left_white, right_white, left_eye, right_eye, left_brow, right_brow, mouth, active_arm, rest_arm, active_glove, rest_glove, left_leg, right_leg, left_shoe, right_shoe]\nfor part in rig_parts:\n    part.parent = root\nactive_hand = active_arm.data.splines[0].points[1]\nactive_hand.co = ({x} + ({side} * 0.78), {y} - 0.05, 0.6, 1.0)\nactive_hand.keyframe_insert(data_path='co', frame=8)\nactive_hand.co = ({x} + ({side} * 1.2), {y} + 0.88, 0.6, 1.0)\nactive_hand.keyframe_insert(data_path='co', frame={pose_frame})\nactive_hand.co = ({x} + ({side} * 1.02), {y} + 0.66, 0.6, 1.0)\nactive_hand.keyframe_insert(data_path='co', frame={settle_frame})\nactive_glove.location = ({x} + ({side} * 0.8), {y} - 0.08, 0.66)\nactive_glove.keyframe_insert(data_path='location', frame=8)\nactive_glove.location = ({x} + ({side} * 1.22), {y} + 0.85, 0.66)\nactive_glove.keyframe_insert(data_path='location', frame={pose_frame})\nactive_glove.location = ({x} + ({side} * 1.04), {y} + 0.63, 0.66)\nactive_glove.keyframe_insert(data_path='location', frame={settle_frame})\nroot.location = (0.0, 0.0, 0.0)\nroot.rotation_euler.z = -0.04 * {side}\nroot.keyframe_insert(data_path='location', frame=1)\nroot.keyframe_insert(data_path='rotation_euler', frame=1)\nroot.location.y = 0.1\nroot.rotation_euler.z = 0.035 * {side}\nroot.keyframe_insert(data_path='location', frame={pose_frame})\nroot.keyframe_insert(data_path='rotation_euler', frame={pose_frame})\nroot.location.y = 0.0\nroot.rotation_euler.z = 0.0\nroot.keyframe_insert(data_path='location', frame={duration_frames})\nroot.keyframe_insert(data_path='rotation_euler', frame={duration_frames})\nleft_white.scale.y = 1.0\nleft_white.keyframe_insert(data_path='scale', frame=1)\nleft_white.scale.y = 0.06\nleft_white.keyframe_insert(data_path='scale', frame={blink_frame})\nleft_white.scale.y = 1.0\nleft_white.keyframe_insert(data_path='scale', frame={blink_open_frame})\nright_white.scale.y = 1.0\nright_white.keyframe_insert(data_path='scale', frame=1)\nright_white.scale.y = 0.06\nright_white.keyframe_insert(data_path='scale', frame={blink_frame})\nright_white.scale.y = 1.0\nright_white.keyframe_insert(data_path='scale', frame={blink_open_frame})\n{cues}"
    );
    format!("{rig_script}{morph_script}")
}

fn element_script(element: &VisualElement) -> AtlasResult<String> {
    let name = py_string(&element.element_id);
    let [x, y, z] = element.transform.position;
    let frame = element.reveal_frame.max(1);
    match element.element_type {
        VisualElementType::Text | VisualElementType::Label => {
            let body = element
                .properties
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&element.element_id);
            let size = element
                .properties
                .get("font_size")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(42.0)
                / 84.0;
            Ok(format!(
                "add_text({name}, {}, ({x}, {y}, {z}), {size}, {frame})\n",
                py_string(body)
            ))
        }
        VisualElementType::StrokeReveal => {
            let points = element
                .properties
                .get("points")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| {
                    AtlasError::Render(format!("{} has no points", element.element_id))
                })?;
            let coordinates = points
                .iter()
                .filter_map(serde_json::Value::as_array)
                .map(|point| {
                    let px = point
                        .first()
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0)
                        + x as f64;
                    let py = point
                        .get(1)
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0)
                        + y as f64;
                    format!("({px}, {py}, {z})")
                })
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "add_line({name}, [{coordinates}], {frame}, INK, 0.045)\n"
            ))
        }
        VisualElementType::Arrow => {
            let start = vector_property(&element.properties, "start", [x - 1.0, y, z]);
            let end = vector_property(&element.properties, "end", [x + 1.0, y, z]);
            Ok(format!(
                "add_arrow({name}, ({}, {}, {}), ({}, {}, {}), {frame})\n",
                start[0], start[1], start[2], end[0], end[1], end[2]
            ))
        }
        VisualElementType::Shape
        | VisualElementType::Diagram
        | VisualElementType::CrossSection
        | VisualElementType::Graph => {
            let [sx, sy, sz] = element.transform.scale;
            Ok(format!(
                "add_box({name}, ({x}, {y}, {z}), ({}, {}, {}), {frame}, BLUE)\n",
                sx.max(0.1),
                sy.max(0.1),
                sz.max(0.05)
            ))
        }
    }
}

fn vector_property(properties: &serde_json::Value, key: &str, fallback: [f32; 3]) -> [f64; 3] {
    let Some(values) = properties.get(key).and_then(serde_json::Value::as_array) else {
        return fallback.map(f64::from);
    };
    [
        values
            .first()
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(fallback[0] as f64),
        values
            .get(1)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(fallback[1] as f64),
        values
            .get(2)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(fallback[2] as f64),
    ]
}

fn camera_script(camera: &CameraSpec) -> String {
    let ortho_scale = camera.ortho_scale.unwrap_or(7.0);
    format!(
        "bpy.ops.object.camera_add(location=({}, {}, {}), rotation=({}, {}, {}))\ncamera = bpy.context.object\ncamera.data.type = 'ORTHO'\ncamera.data.ortho_scale = {}\nscene.camera = camera\n",
        camera.location[0],
        camera.location[1],
        camera.location[2],
        camera.rotation[0],
        camera.rotation[1],
        camera.rotation[2],
        ortho_scale
    )
}

fn py_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

async fn collect_frames(output_dir: &Path, duration_frames: u32) -> AtlasResult<Vec<PathBuf>> {
    let mut frames = Vec::with_capacity(duration_frames as usize);
    for frame in 1..=duration_frames {
        let frame_path = output_dir.join(format!("{frame:04}.png"));
        if !frame_path.is_file() {
            return Err(AtlasError::Render(format!(
                "Blender did not create expected frame {}",
                frame_path.display()
            )));
        }
        frames.push(frame_path);
    }
    Ok(frames)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderOutput {
    pub job_id: String,
    pub frames: Vec<PathBuf>,
    pub duration_frames: u32,
    pub fps: u32,
}

pub fn create_test_shot_spec(project_id: &str, shot_id: &str) -> ShotSpec {
    ShotSpec {
        shot_id: shot_id.to_string(),
        project_id: project_id.to_string(),
        engine: RenderEngine::Blender,
        scene_template: "whiteboard_explain".to_string(),
        duration_frames: 90,
        fps: 30,
        claim_ids: vec!["claim_1".to_string()],
        narration_segment_ids: vec!["nar_1".to_string()],
        visual_elements: vec![
            VisualElement {
                element_id: "title".to_string(),
                element_type: VisualElementType::Text,
                reveal_frame: 1,
                hold_frames: 90,
                properties: serde_json::json!({"text": "How Lithium-Ion Batteries Work", "font_size": 48.0}),
                transform: TransformSpec {
                    position: [0.0, 2.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
            VisualElement {
                element_id: "battery_outline".to_string(),
                element_type: VisualElementType::StrokeReveal,
                reveal_frame: 15,
                hold_frames: 75,
                properties: serde_json::json!({"points": [[-2.4, -0.8], [2.4, -0.8], [2.4, 0.8], [-2.4, 0.8], [-2.4, -0.8]]}),
                transform: TransformSpec {
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
            VisualElement {
                element_id: "anode_label".to_string(),
                element_type: VisualElementType::Label,
                reveal_frame: 30,
                hold_frames: 60,
                properties: serde_json::json!({"text": "ANODE", "font_size": 22.0}),
                transform: TransformSpec {
                    position: [-1.45, 0.0, 0.1],
                    rotation: [0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
            VisualElement {
                element_id: "cathode_label".to_string(),
                element_type: VisualElementType::Label,
                reveal_frame: 45,
                hold_frames: 45,
                properties: serde_json::json!({"text": "CATHODE", "font_size": 22.0}),
                transform: TransformSpec {
                    position: [1.45, 0.0, 0.1],
                    rotation: [0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
            VisualElement {
                element_id: "lithium_flow".to_string(),
                element_type: VisualElementType::Arrow,
                reveal_frame: 60,
                hold_frames: 30,
                properties: serde_json::json!({"start": [-0.4, 0.0, 0.2], "end": [0.4, 0.0, 0.2]}),
                transform: TransformSpec {
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
        ],
        camera: CameraSpec {
            location: [0.0, 0.0, 20.0],
            rotation: [0.0, 0.0, 0.0],
            lens: 50.0,
            ortho_scale: Some(7.5),
        },
        transitions: vec![],
        mascot: None,
        background_media: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn builds_blender_script() {
        let dir = tempdir().unwrap();
        let pipeline = BlenderPipeline::new("blender", dir.path());
        let spec = create_test_shot_spec("test_proj", "shot_001");
        let script = pipeline
            .build_script(&spec, RenderQuality::Preview)
            .unwrap();
        assert!(script.contains("battery_outline"));
        assert!(script.contains("anode_label"));
        assert!(script.contains("bpy.ops.render.render(animation=True)"));
    }

    #[test]
    fn test_shot_spec_serialization() {
        let spec = create_test_shot_spec("test", "shot1");
        let json = serde_json::to_string(&spec).unwrap();
        let parsed: ShotSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.visual_elements.len(), 5);
    }
}
