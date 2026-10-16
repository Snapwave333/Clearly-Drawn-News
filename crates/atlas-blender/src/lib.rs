use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{info, warn};

/// A single shot specification for the Blender renderer.
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
}

/// Visual element to render (stroke, shape, text, diagram).
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

/// Blender render pipeline - generates Python scripts and executes renders.
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

    /// Build a Blender Python script from a ShotSpec.
    pub fn build_script(&self, spec: &ShotSpec, quality: RenderQuality) -> AtlasResult<String> {
        let script = match spec.engine {
            RenderEngine::Blender => self.build_blender_script(spec, quality)?,
            RenderEngine::Unreal => return Err(AtlasError::Render("Unreal not implemented in BlenderPipeline".into())),
        };
        Ok(script)
    }

    fn build_blender_script(&self, spec: &ShotSpec, quality: RenderQuality) -> AtlasResult<String> {
        let (samples, resolution_pct) = match quality {
            RenderQuality::Preview => (16, 25),
            RenderQuality::Final => (128, 100),
        };

        let mut script = String::new();
        script.push_str(&self.blender_header(samples, resolution_pct));
        script.push_str(&self.setup_scene(spec));
        script.push_str(&self.create_grease_pencil_objects(spec));
        script.push_str(&self.animate_stroke_reveals(spec));
        script.push_str(&self.setup_camera_and_render(spec));
        script.push_str(&self.render_output(spec, quality));

        Ok(script)
    }

    fn blender_header(&self, samples: u32, resolution_pct: u32) -> String {
        format!(r#"
import bpy
import math
import json

# Render settings
scene = bpy.context.scene
scene.render.engine = 'BLENDER_EEVEE_NEXT'
scene.eevee.taa_render_samples = {samples}
scene.render.resolution_percentage = {resolution_pct}
scene.render.fps = 30
scene.frame_start = 1
scene.frame_end = {duration}

# World - clean white background
world = bpy.data.worlds.new("WhiteboardWorld")
world.use_nodes = True
bg = world.node_tree.nodes["Background"]
bg.inputs[0].default_value = (1.0, 1.0, 1.0, 1.0)
scene.world = world

# Clean up default objects
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete()

"#, duration = 300) // Will be overridden per shot
    }

    fn setup_scene(&self, spec: &ShotSpec) -> String {
        format!(r#"
# Scene setup
scene.frame_end = {duration}
scene.render.fps = {fps}

"#, duration = spec.duration_frames, fps = spec.fps)
    }

    fn create_grease_pencil_objects(&self, spec: &ShotSpec) -> String {
        let mut script = String::new();
        script.push_str("# Create Grease Pencil objects\n");

        for elem in &spec.visual_elements {
            match elem.element_type {
                VisualElementType::StrokeReveal => {
                    script.push_str(&self.create_stroke_reveal_gp(elem));
                }
                VisualElementType::Text => {
                    script.push_str(&self.create_text_gp(elem));
                }
                VisualElementType::Shape => {
                    script.push_str(&self.create_shape_gp(elem));
                }
                VisualElementType::Arrow => {
                    script.push_str(&self.create_arrow_gp(elem));
                }
                VisualElementType::Label => {
                    script.push_str(&self.create_label_gp(elem));
                }
                VisualElementType::Diagram => {
                    script.push_str(&self.create_diagram_gp(elem));
                }
                VisualElementType::CrossSection => {
                    script.push_str(&self.create_cross_section_gp(elem));
                }
                VisualElementType::Graph => {
                    script.push_str(&self.create_graph_gp(elem));
                }
            }
        }
        script
    }

    fn create_stroke_reveal_gp(&self, elem: &VisualElement) -> String {
        let element_id = &elem.element_id;
        let reveal = elem.reveal_frame;
        let hold = elem.hold_frames;
        let props = &elem.properties;

        // Extract stroke points from properties
        let _points_json = props.get("points").cloned().unwrap_or(serde_json::json!([]));
        let color = props.get("color").and_then(|c| c.as_str()).unwrap_or("#000000");
        let thickness = props.get("thickness").and_then(|t| t.as_f64()).unwrap_or(2.0) as f32;

        format!(r#"
# Stroke reveal: {element_id}
gp_data = bpy.data.grease_pencils.new("{element_id}")
gp_obj = bpy.data.objects.new("{element_id}", gp_data)
bpy.context.collection.objects.link(gp_obj)
gp_obj.location = {position}

layer = gp_data.layers.new("{element_id}_layer", set_active=True)
frame = layer.frames.new({reveal})

stroke = frame.strokes.new()
stroke.display_mode = '3DSPACE'
stroke.draw_mode = '3DSPACE'
stroke.points.add_count({point_count})
# Points will be set below
stroke.line_width = {thickness}
# Color: {color}

# Animate stroke reveal via build modifier
mod = gp_obj.modifiers.new("Build", 'GPENCIL_BUILD')
mod.frame_start = {reveal}
mod.frame_end = {reveal} + {hold}
mod.use_reverse = False

"#, 
            element_id = element_id,
            reveal = reveal,
            hold = hold,
            position = format!("[{}, {}, {}]", elem.transform.position[0], elem.transform.position[1], elem.transform.position[2]),
            point_count = 10, // placeholder
            thickness = thickness,
            color = color,
        )
    }

    fn create_text_gp(&self, elem: &VisualElement) -> String {
        let element_id = &elem.element_id;
        let reveal = elem.reveal_frame;
        let text = elem.properties.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let font_size = elem.properties.get("font_size").and_then(|f| f.as_f64()).unwrap_or(24.0);

        format!(r#"
# Text: {element_id}
txt_data = bpy.data.curves.new(type='FONT', name="{element_id}_curve")
txt_data.body = "{text}"
txt_data.size = {font_size}
txt_obj = bpy.data.objects.new("{element_id}", txt_data)
bpy.context.collection.objects.link(txt_obj)
txt_obj.location = {position}
txt_obj.hide_render = True
txt_obj.hide_viewport = True

# Reveal at frame {reveal}
txt_obj.keyframe_insert(data_path="hide_render", frame={reveal}-1)
txt_obj.hide_render = False
txt_obj.keyframe_insert(data_path="hide_render", frame={reveal})

"#,
            element_id = element_id,
            reveal = reveal,
            position = format!("[{}, {}, {}]", elem.transform.position[0], elem.transform.position[1], elem.transform.position[2]),
            font_size = font_size,
        )
    }

    fn create_shape_gp(&self, elem: &VisualElement) -> String {
        let element_id = &elem.element_id;
        let reveal = elem.reveal_frame;
        let shape_type = elem.properties.get("shape").and_then(|s| s.as_str()).unwrap_or("circle");

        format!(r#"
# Shape: {element_id} ({shape_type})
bpy.ops.mesh.primitive_{shape_type}_add(location={position})
shape_obj = bpy.context.active_object
shape_obj.name = "{element_id}"
shape_obj.hide_render = True
shape_obj.hide_viewport = True
shape_obj.keyframe_insert(data_path="hide_render", frame={reveal}-1)
shape_obj.hide_render = False
shape_obj.keyframe_insert(data_path="hide_render", frame={reveal})

"#,
            element_id = element_id,
            shape_type = shape_type,
            reveal = reveal,
            position = format!("[{}, {}, {}]", elem.transform.position[0], elem.transform.position[1], elem.transform.position[2]),
        )
    }

    fn create_arrow_gp(&self, elem: &VisualElement) -> String {
        let element_id = &elem.element_id;
        let reveal = elem.reveal_frame;
        let start_arr = elem.properties.get("start").and_then(|s| s.as_array()).cloned().unwrap_or_default();
        let end_arr = elem.properties.get("end").and_then(|e| e.as_array()).cloned().unwrap_or_default();
        let start = &start_arr;
        let end = &end_arr;

        format!(r#"
# Arrow: {element_id}
import mathutils
start = mathutils.Vector({start})
end = mathutils.Vector({end})
direction = (end - start).normalized()
length = (end - start).length

bpy.ops.mesh.primitive_cone_add(radius1=0.05, radius2=0, depth=length, location=(start + end) / 2)
arrow_obj = bpy.context.active_object
arrow_obj.name = "{element_id}"
arrow_obj.rotation_euler = direction.to_track_quat('Z', 'Y').to_euler()
arrow_obj.hide_render = True
arrow_obj.hide_viewport = True
arrow_obj.keyframe_insert(data_path="hide_render", frame={reveal}-1)
arrow_obj.hide_render = False
arrow_obj.keyframe_insert(data_path="hide_render", frame={reveal})

"#,
            element_id = element_id,
            reveal = reveal,
            start = format!("[{}, {}, {}]", start.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0), start.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0), start.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0)),
            end = format!("[{}, {}, {}]", end.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0), end.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0), end.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0)),
        )
    }

    fn create_label_gp(&self, elem: &VisualElement) -> String {
        self.create_text_gp(elem) // Labels are just text
    }

    fn create_diagram_gp(&self, elem: &VisualElement) -> String {
        // Diagram = composition of shapes + arrows + labels
        let element_id = &elem.element_id;
        let reveal = elem.reveal_frame;
        let components_arr = elem.properties.get("components").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        let components = &components_arr;

        let mut script = format!("# Diagram: {element_id}\n");
        for (i, comp) in components.iter().enumerate() {
            let comp_type = comp.get("type").and_then(|t| t.as_str()).unwrap_or("shape");
            let comp_id = format!("{}_{}", element_id, i);
            let comp_reveal = comp.get("reveal_frame").and_then(|r| r.as_u64()).unwrap_or(reveal as u64);
            
            script.push_str(&match comp_type {
                "shape" => self.create_shape_gp(&VisualElement {
                    element_id: comp_id,
                    element_type: VisualElementType::Shape,
                    reveal_frame: comp_reveal as u32,
                    hold_frames: elem.hold_frames,
                    properties: comp.clone(),
                    transform: elem.transform.clone(),
                }),
                "arrow" => self.create_arrow_gp(&VisualElement {
                    element_id: comp_id,
                    element_type: VisualElementType::Arrow,
                    reveal_frame: comp_reveal as u32,
                    hold_frames: elem.hold_frames,
                    properties: comp.clone(),
                    transform: elem.transform.clone(),
                }),
                "text" => self.create_text_gp(&VisualElement {
                    element_id: comp_id,
                    element_type: VisualElementType::Text,
                    reveal_frame: comp_reveal as u32,
                    hold_frames: elem.hold_frames,
                    properties: comp.clone(),
                    transform: elem.transform.clone(),
                }),
                _ => String::new(),
            });
        }
        script
    }

    fn create_cross_section_gp(&self, elem: &VisualElement) -> String {
        // Cross-section: outer shape peels away to reveal inner layers
        let element_id = &elem.element_id;
        let reveal = elem.reveal_frame;
        let layers_arr = elem.properties.get("layers").and_then(|l| l.as_array()).cloned().unwrap_or_default();
        let layers = &layers_arr;

        let mut script = format!("# Cross-section: {element_id}\n");
        for (i, layer) in layers.iter().enumerate() {
            let layer_id = format!("{}_layer{}", element_id, i);
            let layer_reveal = layer.get("reveal_frame").and_then(|r| r.as_u64()).unwrap_or(reveal as u64 + i as u64 * 5) as u32;
            
            script.push_str(&self.create_shape_gp(&VisualElement {
                element_id: layer_id,
                element_type: VisualElementType::Shape,
                reveal_frame: layer_reveal as u32,
                hold_frames: elem.hold_frames,
                properties: layer.clone(),
                transform: elem.transform.clone(),
            }));
        }
        script
    }

    fn create_graph_gp(&self, elem: &VisualElement) -> String {
        let element_id = &elem.element_id;
        let _reveal = elem.reveal_frame;
        let data_arr = elem.properties.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default();
        let data = &data_arr;
        let graph_type = elem.properties.get("type").and_then(|t| t.as_str()).unwrap_or("line");

        format!(r#"
# Graph: {element_id} ({graph_type})
# Data points: {point_count}
# This would generate axes, grid, and animated plot lines
# Placeholder for VectorCraft-generated SVG import

"#, 
            element_id = element_id,
            graph_type = graph_type,
            point_count = data.len(),
        )
    }

    fn animate_stroke_reveals(&self, spec: &ShotSpec) -> String {
        // Additional animation logic for coordinated reveals
        let mut script = String::new();
        script.push_str("# Coordinated stroke reveal timing\n");
        for elem in &spec.visual_elements {
            if matches!(elem.element_type, VisualElementType::StrokeReveal) {
                script.push_str(&format!(
                    "# {} reveals at frame {}, holds for {} frames\n",
                    elem.element_id, elem.reveal_frame, elem.hold_frames
                ));
            }
        }
        script
    }

    fn setup_camera_and_render(&self, spec: &ShotSpec) -> String {
        let cam = &spec.camera;
        format!(r#"
# Camera setup
bpy.ops.object.camera_add(location={location}, rotation={rotation})
camera = bpy.context.active_object
camera.data.lens = {lens}
scene.camera = camera

"#, 
            location = format!("[{}, {}, {}]", cam.location[0], cam.location[1], cam.location[2]),
            rotation = format!("[{}, {}, {}]", cam.rotation[0], cam.rotation[1], cam.rotation[2]),
            lens = cam.lens,
        )
    }

    fn render_output(&self, spec: &ShotSpec, quality: RenderQuality) -> String {
        let output_dir = self.workspace_dir.join("renders").join(&spec.project_id).join(&spec.shot_id);
        let format = match quality {
            RenderQuality::Preview => "PNG",
            RenderQuality::Final => "PNG", // PNG sequence for frame-accurate editing
        };
        format!(r#"
# Render output
scene.render.filepath = r"{}"
scene.render.image_settings.file_format = '{}'
scene.render.image_settings.color_mode = 'RGBA'
scene.render.image_settings.color_depth = '16'
bpy.ops.render.render(animation=True, write_still=False)

"#, output_dir.display(), format)
    }

    /// Execute a render job (preview or final).
    pub async fn render(&self, job: &RenderJob) -> AtlasResult<RenderOutput> {
        let script = self.build_script(&job.shot_spec, job.quality.clone())?;
        
        // Write script to temp file
        let script_path = self.workspace_dir.join("temp").join(format!("render_{}.py", job.job_id));
        tokio::fs::create_dir_all(script_path.parent().unwrap()).await?;
        tokio::fs::write(&script_path, &script).await?;

        info!("Starting Blender render: {} ({:?})", job.job_id, job.quality);

        // Execute Blender
        let output = Command::new(&self.blender_exe)
            .args(["--background", "--python", script_path.to_str().unwrap()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| AtlasError::Render(format!("Failed to start Blender: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Blender render failed: {}", stderr);
            return Err(AtlasError::Render(format!("Blender exited with {}: {}", output.status, stderr)));
        }

        // Collect output frames
        let frames = self.collect_frames(&job.shot_spec, job.quality.clone()).await?;
        
        Ok(RenderOutput {
            job_id: job.job_id.clone(),
            frames,
            duration_frames: job.shot_spec.duration_frames,
            fps: job.shot_spec.fps,
        })
    }

    async fn collect_frames(&self, spec: &ShotSpec, _quality: RenderQuality) -> AtlasResult<Vec<PathBuf>> {
        let output_dir = self.workspace_dir.join("renders").join(&spec.project_id).join(&spec.shot_id);
        let mut frames = Vec::new();
        
        for frame in 1..=spec.duration_frames {
            let frame_path = output_dir.join(format!("{:04}.png", frame));
            if frame_path.exists() {
                frames.push(frame_path);
            }
        }
        
        if frames.is_empty() {
            return Err(AtlasError::Render("No frames rendered".into()));
        }
        
        Ok(frames)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderOutput {
    pub job_id: String,
    pub frames: Vec<PathBuf>,
    pub duration_frames: u32,
    pub fps: u32,
}

/// Helper to create a simple whiteboard shot spec for testing.
pub fn create_test_shot_spec(project_id: &str, shot_id: &str) -> ShotSpec {
    ShotSpec {
        shot_id: shot_id.to_string(),
        project_id: project_id.to_string(),
        engine: RenderEngine::Blender,
        scene_template: "whiteboard_explain".to_string(),
        duration_frames: 90, // 3 seconds at 30fps
        fps: 30,
        claim_ids: vec!["claim_1".to_string()],
        narration_segment_ids: vec!["nar_1".to_string()],
        visual_elements: vec![
            VisualElement {
                element_id: "title".to_string(),
                element_type: VisualElementType::Text,
                reveal_frame: 1,
                hold_frames: 90,
                properties: serde_json::json!({
                    "text": "How Lithium-Ion Batteries Work",
                    "font_size": 48.0,
                }),
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
                properties: serde_json::json!({
                    "points": [[-1.0, -0.5], [1.0, -0.5], [1.0, 0.5], [-1.0, 0.5], [-1.0, -0.5]],
                    "color": "#000000",
                    "thickness": 3.0,
                }),
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
                properties: serde_json::json!({
                    "text": "Anode (Graphite)",
                    "font_size": 24.0,
                }),
                transform: TransformSpec {
                    position: [-1.5, -0.5, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
            VisualElement {
                element_id: "cathode_label".to_string(),
                element_type: VisualElementType::Label,
                reveal_frame: 45,
                hold_frames: 45,
                properties: serde_json::json!({
                    "text": "Cathode (LiCoO₂)",
                    "font_size": 24.0,
                }),
                transform: TransformSpec {
                    position: [1.5, 0.5, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
            VisualElement {
                element_id: "arrow_li".to_string(),
                element_type: VisualElementType::Arrow,
                reveal_frame: 60,
                hold_frames: 30,
                properties: serde_json::json!({
                    "start": [-0.5, 0.0],
                    "end": [0.5, 0.0],
                }),
                transform: TransformSpec {
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
        ],
        camera: CameraSpec {
            location: [0.0, 0.0, 10.0],
            rotation: [0.0, 0.0, 0.0],
            lens: 50.0,
            ortho_scale: Some(5.0),
        },
        transitions: vec![],
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
        let script = pipeline.build_script(&spec, RenderQuality::Preview).unwrap();
        
        assert!(script.contains("Grease Pencil"));
        assert!(script.contains("battery_outline"));
        assert!(script.contains("anode_label"));
        assert!(script.contains("stroke.line_width"));
    }

    #[test]
    fn test_shot_spec_serialization() {
        let spec = create_test_shot_spec("test", "shot1");
        let json = serde_json::to_string(&spec).unwrap();
        let parsed: ShotSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.visual_elements.len(), 5);
    }
}