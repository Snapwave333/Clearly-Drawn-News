use super::*;

#[derive(Clone)]
pub(crate) struct ProductionBeat {
    title: String,
    narration: String,
    scene: String,
    emotion: MascotEmotion,
    gesture: MascotGesture,
}

pub(crate) enum VideoBeat {
    Curated(ProductionBeat),
    Directed(StoryboardBeat),
}

pub(crate) struct PlannedVideo {
    pub(crate) beats: Vec<VideoBeat>,
    pub(crate) generated_plan: Option<StoryboardPlan>,
}

impl VideoBeat {
    pub(crate) fn title(&self) -> &str {
        match self {
            Self::Curated(beat) => &beat.title,
            Self::Directed(beat) => &beat.title,
        }
    }

    pub(crate) fn narration(&self) -> &str {
        match self {
            Self::Curated(beat) => &beat.narration,
            Self::Directed(beat) => &beat.narration,
        }
    }

    pub(crate) fn scene(&self) -> &str {
        match self {
            Self::Curated(beat) => &beat.scene,
            Self::Directed(beat) => match beat.scene_pattern {
                ScenePattern::Hook => "generic_hook",
                ScenePattern::Parts => "generic_parts",
                ScenePattern::Flow => "generic_flow",
                ScenePattern::SplitPath => "generic_split_path",
                ScenePattern::Comparison => "generic_comparison",
                ScenePattern::Scale => "generic_scale",
                ScenePattern::Timeline => "generic_timeline",
                ScenePattern::CauseEffect => "generic_cause_effect",
                ScenePattern::Risk => "generic_risk",
                ScenePattern::Cycle => "generic_cycle",
            },
        }
    }

    pub(crate) fn emotion(&self) -> MascotEmotion {
        match self {
            Self::Curated(beat) => beat.emotion.clone(),
            Self::Directed(beat) => match beat.character.emotion {
                DirectedEmotion::Curious => MascotEmotion::Curious,
                DirectedEmotion::Excited => MascotEmotion::Excited,
                DirectedEmotion::Explaining => MascotEmotion::Explaining,
                DirectedEmotion::Concerned => MascotEmotion::Concerned,
                DirectedEmotion::Confident => MascotEmotion::Confident,
            },
        }
    }

    pub(crate) fn gesture(&self) -> MascotGesture {
        match self {
            Self::Curated(beat) => beat.gesture.clone(),
            Self::Directed(beat) => match beat.character.gesture {
                DirectedGesture::PointLeft => MascotGesture::PointLeft,
                DirectedGesture::PointRight => MascotGesture::PointRight,
                DirectedGesture::Present => MascotGesture::Present,
                DirectedGesture::Think => MascotGesture::Think,
                DirectedGesture::Warn => MascotGesture::Warn,
                DirectedGesture::Celebrate => MascotGesture::Celebrate,
            },
        }
    }

    pub(crate) fn morph(&self) -> Option<MascotMorph> {
        match self {
            Self::Curated(beat) => morph_for_scene(&beat.scene),
            Self::Directed(beat) => beat.character.morph.as_ref().map(|morph| MascotMorph {
                label: morph.label.clone(),
                silhouette: morph.silhouette.clone(),
            }),
        }
    }

    pub(crate) fn broll_query(&self) -> &str {
        match self {
            Self::Curated(_) => "educational science technology close up",
            Self::Directed(beat) => &beat.broll_query,
        }
    }

    pub(crate) fn focal_terms(&self) -> Vec<String> {
        match self {
            Self::Curated(beat) => curated_visual_plan(&beat.scene).1,
            Self::Directed(beat) => beat.focal_terms.clone(),
        }
    }

    pub(crate) fn apply_visuals(&self, spec: &mut atlas_blender::ShotSpec, portrait: bool) {
        if let Self::Curated(beat) = self {
            let (pattern, focal_terms) = curated_visual_plan(&beat.scene);
            let directed = StoryboardBeat {
                title: beat.title.clone(),
                narration: beat.narration.clone(),
                scene_pattern: pattern,
                focal_terms,
                broll_query: self.broll_query().into(),
                character: atlas_storyboard::CharacterDirection {
                    emotion: DirectedEmotion::Explaining,
                    gesture: DirectedGesture::Present,
                    acting_goal: "Make the mechanism visible".into(),
                    morph: None,
                },
            };
            Self::Directed(directed).apply_visuals(spec, portrait);
            return;
        }
        if let Self::Directed(beat) = self {
            spec.visual_elements.truncate(1);
            let count = beat.focal_terms.len().max(1);
            for (index, term) in beat.focal_terms.iter().enumerate() {
                let progress = if count == 1 {
                    0.5
                } else {
                    index as f32 / (count - 1) as f32
                };
                let x = if portrait { 0.0 } else { -2.4 + progress * 4.8 };
                let y = if portrait {
                    1.4 - index as f32 * 0.75
                } else {
                    0.2
                };
                spec.visual_elements.push(VisualElement {
                    element_id: format!("focus_{index}"),
                    element_type: VisualElementType::Shape,
                    reveal_frame: 20 + index as u32 * 14,
                    hold_frames: spec.duration_frames.saturating_sub(20),
                    properties: serde_json::json!({}),
                    transform: TransformSpec {
                        position: [x, y, 0.1],
                        rotation: [0.0, 0.0, 0.0],
                        scale: if portrait {
                            [1.45, 0.28, 0.05]
                        } else {
                            [0.95, 0.52, 0.05]
                        },
                    },
                });
                spec.visual_elements.push(VisualElement {
                    element_id: format!("label_{index}"),
                    element_type: VisualElementType::Label,
                    reveal_frame: 26 + index as u32 * 14,
                    hold_frames: spec.duration_frames.saturating_sub(26),
                    properties: serde_json::json!({"text": term, "font_size": 18.0}),
                    transform: TransformSpec {
                        position: [x, y, 0.25],
                        rotation: [0.0, 0.0, 0.0],
                        scale: [1.0, 1.0, 1.0],
                    },
                });
                if index > 0 && !matches!(beat.scene_pattern, ScenePattern::Parts) {
                    let previous_progress = if count == 1 {
                        0.5
                    } else {
                        (index - 1) as f32 / (count - 1) as f32
                    };
                    let previous_x = if portrait {
                        0.0
                    } else {
                        -2.4 + previous_progress * 4.8
                    };
                    let previous_y = if portrait {
                        1.4 - (index - 1) as f32 * 0.75
                    } else {
                        0.2
                    };
                    spec.visual_elements.push(VisualElement {
                        element_id: format!("connection_{index}"),
                        element_type: VisualElementType::Arrow,
                        reveal_frame: 32 + index as u32 * 14,
                        hold_frames: spec.duration_frames.saturating_sub(32),
                        properties: serde_json::json!({
                            "start": [previous_x, previous_y, 0.2],
                            "end": [x, y, 0.2]
                        }),
                        transform: TransformSpec {
                            position: [0.0, 0.0, 0.0],
                            rotation: [0.0, 0.0, 0.0],
                            scale: [1.0, 1.0, 1.0],
                        },
                    });
                }
            }
        }
    }
}

pub(crate) fn curated_visual_plan(scene: &str) -> (ScenePattern, Vec<String>) {
    let s = scene.to_ascii_lowercase();
    if s.contains("hook") {
        (
            ScenePattern::Hook,
            vec!["chemical imbalance".into(), "stored energy".into()],
        )
    } else if s.contains("anatomy") || s.contains("part") {
        (
            ScenePattern::Parts,
            vec!["anode".into(), "separator".into(), "cathode".into()],
        )
    } else if s.contains("electrolyte") || s.contains("split") {
        (
            ScenePattern::SplitPath,
            vec!["ions pass".into(), "electrons stop".into()],
        )
    } else if s.contains("flow") || s.contains("charg") {
        (
            ScenePattern::Flow,
            vec!["intake flow".into(), "transport channel".into(), "reaction core".into()],
        )
    } else if s.contains("electron") || s.contains("wire") {
        (
            ScenePattern::SplitPath,
            vec!["conduction path".into(), "external circuit".into()],
        )
    } else if s.contains("discharg") {
        (
            ScenePattern::Flow,
            vec!["driving potential".into(), "load circuit".into(), "lower state".into()],
        )
    } else if s.contains("voltage") || s.contains("scale") {
        (
            ScenePattern::Scale,
            vec![
                "high energy".into(),
                "voltage drop".into(),
                "useful work".into(),
            ],
        )
    } else if s.contains("aging") || s.contains("cause") {
        (
            ScenePattern::CauseEffect,
            vec![
                "stress".into(),
                "side reactions".into(),
                "capacity loss".into(),
            ],
        )
    } else if s.contains("safe") || s.contains("risk") || s.contains("limit") {
        (
            ScenePattern::Risk,
            vec!["barrier".into(), "thermal stress".into(), "protection".into()],
        )
    } else {
        (
            ScenePattern::Cycle,
            vec!["input work".into(), "storage state".into(), "useful power".into()],
        )
    }
}

pub(crate) async fn planned_video(
    topic: &str,
    format: VideoFormat,
    long_plan: Option<&StoryboardPlan>,
) -> anyhow::Result<PlannedVideo> {
    let expected = format.profile().shot_count as usize;
    if matches!(format, VideoFormat::Short) {
        if let Some(long_plan) = long_plan {
            if let Some(director) = DirectorClient::from_env() {
                match director.create_short_plan(long_plan, expected).await {
                    Ok(plan) => {
                        let beats = plan
                            .beats
                            .iter()
                            .cloned()
                            .map(VideoBeat::Directed)
                            .collect();
                        return Ok(PlannedVideo {
                            beats,
                            generated_plan: Some(plan),
                        });
                    }
                    Err(error) => eprintln!("Creative director fallback for short adaptation: {error}"),
                }
            }
        }
    }
    if let Some(director) = DirectorClient::from_env() {
        match director
            .create_plan(topic, format.profile().slug, expected)
            .await
        {
            Ok(plan) => {
                let beats = plan
                    .beats
                    .iter()
                    .cloned()
                    .map(VideoBeat::Directed)
                    .collect();
                return Ok(PlannedVideo {
                    beats,
                    generated_plan: Some(plan),
                });
            }
            Err(error) => eprintln!("Creative director fallback activated: {error}"),
        }
    }
    Ok(PlannedVideo {
        beats: production_beats(topic, format)
            .into_iter()
            .map(VideoBeat::Curated)
            .collect(),
        generated_plan: None,
    })
}

pub(crate) fn production_beats(topic: &str, format: VideoFormat) -> Vec<ProductionBeat> {
    if !topic.to_ascii_lowercase().contains("lithium") {
        return generic_beats(topic, format);
    }
    match format {
        VideoFormat::Short => vec![
            ProductionBeat {
                title: "YOUR BATTERY IS NOT A TANK".into(),
                narration: "Your battery does not store electricity like a bottle; it stores a chemical imbalance, and Luma will show you the trick.".into(),
                scene: "battery_hook".into(),
                emotion: MascotEmotion::Curious,
                gesture: MascotGesture::Present,
            },
            ProductionBeat {
                title: "THREE LAYERS CONTROL THE ACTION".into(),
                narration: "Two electrodes hold lithium, while an electrolyte carries ions and a separator prevents a dangerous short circuit.".into(),
                scene: "battery_anatomy".into(),
                emotion: MascotEmotion::Explaining,
                gesture: MascotGesture::PointLeft,
            },
            ProductionBeat {
                title: "CHARGING SEPARATES THE PLAYERS".into(),
                narration: "A charger pushes lithium ions into the graphite anode while electrons travel there through the outside wire.".into(),
                scene: "battery_charging".into(),
                emotion: MascotEmotion::Excited,
                gesture: MascotGesture::PointRight,
            },
            ProductionBeat {
                title: "DISCHARGING MAKES CURRENT".into(),
                narration: "When you unplug, ions return through the electrolyte and electrons power your device on their separate route.".into(),
                scene: "battery_discharging".into(),
                emotion: MascotEmotion::Confident,
                gesture: MascotGesture::Present,
            },
            ProductionBeat {
                title: "HEAT STEALS CAPACITY".into(),
                narration: "Heat and side reactions build barriers that trap active lithium, so an older battery cannot hold the same charge.".into(),
                scene: "battery_aging".into(),
                emotion: MascotEmotion::Concerned,
                gesture: MascotGesture::Warn,
            },
        ],
        VideoFormat::Long => {
            let core_beats = [
                ("A BATTERY STORES A CHEMICAL IMBALANCE", "A lithium ion battery is not a tank of electricity; it stores energy by separating charge into a chemical state that actively wants to relax toward equilibrium.", "battery_hook", MascotEmotion::Curious, MascotGesture::Present),
                ("THE CELL HAS FOUR CRITICAL PARTS", "The graphite anode and metal oxide cathode host lithium ions, the electrolyte carries those ions between them, and the porous separator prevents physical short circuits.", "battery_anatomy", MascotEmotion::Explaining, MascotGesture::PointLeft),
                ("IONS MAY CROSS; ELECTRONS MAY NOT", "The electrolyte conducts positively charged lithium ions smoothly, but it is an electrical insulator for electrons, forcing charge into external conductors.", "battery_electrolyte", MascotEmotion::Explaining, MascotGesture::PointRight),
                ("CHARGING DRIVES LITHIUM UPHILL", "When connected to a power supply, external electrical work pulls lithium atoms from the metal oxide cathode and drives them as ions into graphite interstitial sheets.", "battery_charging", MascotEmotion::Excited, MascotGesture::PointLeft),
                ("ELECTRONS TAKE THE OUTSIDE ROAD", "Simultaneously, electrons travel through the external circuit wire because the internal separator and electrolyte completely block electron transit.", "battery_electrons", MascotEmotion::Explaining, MascotGesture::PointRight),
                ("DISCHARGING POWERS THE OUTSIDE WORLD", "Upon connecting a load, spontaneous chemical potentials drive ions back through the electrolyte while electrons flow through your external device.", "battery_discharging", MascotEmotion::Confident, MascotGesture::Present),
                ("VOLTAGE MEASURES THE CHEMICAL POTENTIAL", "Terminal voltage is a direct measure of the Gibbs free energy difference between lithium intercalated in graphite versus the metal oxide crystal lattice.", "battery_voltage", MascotEmotion::Curious, MascotGesture::Think),
                ("THE SOLID ELECTROLYTE INTERPHASE FORMS", "During the very first charging cycles, electrolyte decomposition forms a passivating SEI film on the anode that protects the electrode from further continuous solvent breakdown.", "battery_aging", MascotEmotion::Explaining, MascotGesture::PointRight),
                ("PARASITIC SIDE REACTIONS CONSUME CAPACITY", "Elevated temperatures and prolonged exposure to high states of charge accelerate secondary parasitic chemical reactions that permanently trap active lithium ions.", "battery_aging", MascotEmotion::Concerned, MascotGesture::Warn),
                ("LITHIUM PLATING AND DENDRITE FORMATION", "Charging too rapidly at freezing temperatures forces lithium ions to deposit as metallic needles rather than intercalating into graphite, risking internal punctures.", "battery_safety", MascotEmotion::Concerned, MascotGesture::Warn),
                ("THE SEPARATOR AS A CRUCIAL SAFETY BARRIER", "A ceramic-coated polymer separator maintains a microscopic gap between opposing electrodes, with shutdown pores that seal up if the cell overheats.", "battery_safety", MascotEmotion::Explaining, MascotGesture::PointLeft),
                ("CATHODE CHEMISTRY GOVERNS ENERGY DENSITY", "Commercial cells choose between nickel-manganese-cobalt formulations for high specific energy density and lithium iron phosphate for superior thermal stability and cyclic longevity.", "battery_anatomy", MascotEmotion::Explaining, MascotGesture::Present),
                ("THE COMPLETE THERMODYNAMIC CYCLE", "Every battery cycle represents a controlled round-trip migration: electrical work organizes high-energy states, and controlled discharge extracts that energy to power modern life.", "battery_summary", MascotEmotion::Confident, MascotGesture::Celebrate),
            ];
            let mut beats = Vec::new();
            for i in 0..35 {
                let (title, narration, scene, emotion, gesture) = &core_beats[i % core_beats.len()];
                beats.push(ProductionBeat {
                    title: (*title).to_string(),
                    narration: (*narration).to_string(),
                    scene: (*scene).to_string(),
                    emotion: emotion.clone(),
                    gesture: gesture.clone(),
                });
            }
            beats
        }
    }
}

pub(crate) fn generic_beats(topic: &str, format: VideoFormat) -> Vec<ProductionBeat> {
    let count = format.profile().shot_count as usize;
    let clean_topic = topic.trim();
    let upper = clean_topic.to_ascii_uppercase();
    let lower = clean_topic.to_ascii_lowercase();

    let is_fluid_wave = lower.contains("vort")
        || lower.contains("wave")
        || lower.contains("vibrat")
        || lower.contains("flow")
        || lower.contains("aero")
        || lower.contains("hydro");
    let is_bio_chem = lower.contains("gene")
        || lower.contains("crispr")
        || lower.contains("cell")
        || lower.contains("dna")
        || lower.contains("protein")
        || lower.contains("enzyme")
        || lower.contains("organ");
    let is_compute = lower.contains("comput")
        || lower.contains("neural")
        || lower.contains("algorithm")
        || lower.contains("code")
        || lower.contains("quantum")
        || lower.contains("silicon")
        || lower.contains("chip");

    let archetypes = if is_fluid_wave {
        vec![
            (
                format!("THE PUZZLE OF {upper}"),
                format!("When examining {clean_topic}, subtle fluid boundary layer interactions trigger unexpected macroscopic motion that defies simple intuition."),
                "wave_hook",
                MascotEmotion::Curious,
                MascotGesture::Present,
            ),
            (
                "FLUID BOUNDARIES AND SHEDDING".to_string(),
                format!("As moving flow impinges upon {clean_topic}, boundary layer separation produces asymmetric pressure gradients."),
                "vort_anatomy",
                MascotEmotion::Explaining,
                MascotGesture::PointLeft,
            ),
            (
                "HYDRODYNAMIC DISPLACEMENT".to_string(),
                format!("Alternating vortex shedding forces the physical structure of {clean_topic} into cyclical transverse oscillation."),
                "flow_transport",
                MascotEmotion::Explaining,
                MascotGesture::PointRight,
            ),
            (
                "RESONANCE AND LOCK-IN ENVELOPE".to_string(),
                format!("When vortex shedding frequency synchronizes with the natural resonance of {clean_topic}, lock-in amplifies displacement into rapid fatigue."),
                "vort_resonance",
                MascotEmotion::Concerned,
                MascotGesture::Warn,
            ),
            (
                "ENGINEERING SUPPRESSION AND MITIGATION".to_string(),
                format!("Helical strakes, tuned mass dampers, and hydrodynamic fairings disrupt coherent shedding to protect {clean_topic}."),
                "risk_mitigation",
                MascotEmotion::Confident,
                MascotGesture::Celebrate,
            ),
        ]
    } else if is_bio_chem {
        vec![
            (
                format!("THE CODE OF {upper}"),
                format!("In the molecular machinery of {clean_topic}, precise biochemical interactions orchestrate biological function with atomic fidelity."),
                "bio_hook",
                MascotEmotion::Curious,
                MascotGesture::Present,
            ),
            (
                "MOLECULAR STRUCTURE AND DOMAINS".to_string(),
                format!("The three-dimensional architecture of {clean_topic} relies on specific active sites and complementary binding domains."),
                "cell_anatomy",
                MascotEmotion::Explaining,
                MascotGesture::PointLeft,
            ),
            (
                "CATALYTIC AND TRANSPORT KINETICS".to_string(),
                format!("Substrates migrate through molecular pathways in {clean_topic}, lowering activation barriers for spontaneous reactions."),
                "transport_flow",
                MascotEmotion::Explaining,
                MascotGesture::PointRight,
            ),
            (
                "REGULATION AND CELLULAR EQUILIBRIUM".to_string(),
                format!("Feedback mechanisms monitor reaction thresholds in {clean_topic}, preventing metabolic instability or toxic build-up."),
                "risk_boundary",
                MascotEmotion::Concerned,
                MascotGesture::Warn,
            ),
            (
                "THE REGENERATIVE LIFE CYCLE".to_string(),
                format!("By coupling thermodynamic driving forces with cellular repair, {clean_topic} maintains homeostatic continuity."),
                "cycle_summary",
                MascotEmotion::Confident,
                MascotGesture::Celebrate,
            ),
        ]
    } else if is_compute {
        vec![
            (
                format!("HOW {upper} COMPUTES"),
                format!("At the foundation of {clean_topic}, microscopic state transitions coordinate to solve complex computational problems."),
                "circuit_hook",
                MascotEmotion::Curious,
                MascotGesture::Present,
            ),
            (
                "LOGIC GATES AND SYSTEM ARCHITECTURE".to_string(),
                format!("Information in {clean_topic} flows across interconnected logic gates and memory registers structured in deterministic hierarchies."),
                "chip_anatomy",
                MascotEmotion::Explaining,
                MascotGesture::PointLeft,
            ),
            (
                "SIGNAL PROPAGATION AND CONCURRENCY".to_string(),
                format!("Synchronous clock pulses propagate high-frequency data streams across the routing buses of {clean_topic}."),
                "signal_flow",
                MascotEmotion::Explaining,
                MascotGesture::PointRight,
            ),
            (
                "LATENCY, BOTTLENECKS, AND THERMAL LIMITS".to_string(),
                format!("Parasitic capacitance and thermal dissipation create fundamental scaling limits for modern {clean_topic}."),
                "thermal_risk",
                MascotEmotion::Concerned,
                MascotGesture::Warn,
            ),
            (
                "THE UNIFIED COMPUTATIONAL MODEL".to_string(),
                format!("From basic bit operations to global distributed throughput, {clean_topic} transforms simple rules into immense capability."),
                "system_summary",
                MascotEmotion::Confident,
                MascotGesture::Celebrate,
            ),
        ]
    } else {
        vec![
            (
                format!("INVESTIGATING {upper}"),
                format!("Every physical principle in {clean_topic} operates under strict laws governing conservation of energy and equilibrium."),
                "universal_hook",
                MascotEmotion::Curious,
                MascotGesture::Present,
            ),
            (
                "SYSTEM BOUNDARIES AND ARCHITECTURE".to_string(),
                format!("By separating {clean_topic} into core functional modules, we can observe how each component behaves under operational load."),
                "universal_anatomy",
                MascotEmotion::Explaining,
                MascotGesture::PointLeft,
            ),
            (
                "DYNAMIC EQUILIBRIUM AND TRANSPORT".to_string(),
                format!("Continuous mass and energy transfer through {clean_topic} establishes steady-state operation."),
                "universal_flow",
                MascotEmotion::Explaining,
                MascotGesture::PointRight,
            ),
            (
                "OPERATIONAL THRESHOLDS AND STRESS".to_string(),
                format!("Subjecting {clean_topic} to excessive operating conditions exposes boundary failure points and safety limits."),
                "universal_risk",
                MascotEmotion::Concerned,
                MascotGesture::Warn,
            ),
            (
                "SYNTHESIS OF THE COMPLETE MODEL".to_string(),
                format!("Understanding the governing relationships of {clean_topic} provides a deterministic mental model of cause and effect."),
                "universal_cycle",
                MascotEmotion::Confident,
                MascotGesture::Celebrate,
            ),
        ]
    };

    let mut beats = Vec::with_capacity(count);
    for i in 0..count {
        let (title, narration, scene, emotion, gesture) = &archetypes[i % archetypes.len()];
        let beat_title = if count > archetypes.len() {
            format!("{title} [PART {}]", (i / archetypes.len()) + 1)
        } else {
            title.clone()
        };
        beats.push(ProductionBeat {
            title: beat_title,
            narration: narration.clone(),
            scene: scene.to_string(),
            emotion: emotion.clone(),
            gesture: gesture.clone(),
        });
    }
    beats
}

pub(crate) fn lip_sync_cues(audio: &AudioSegment) -> Vec<LipSyncCue> {
    let mut cues = vec![LipSyncCue {
        frame: 1,
        viseme: Viseme::Rest,
    }];
    for word in &audio.word_timestamps {
        let sounds = word
            .word
            .chars()
            .filter(|value| value.is_ascii_alphabetic())
            .collect::<Vec<_>>();
        let sound_count = sounds.len().clamp(1, 5);
        let duration = (word.end_seconds - word.start_seconds).max(0.03);
        for (index, sound) in sounds.into_iter().take(sound_count).enumerate() {
            let progress = index as f32 / sound_count as f32;
            let frame = ((word.start_seconds + duration * progress) * 30.0).round() as u32 + 1;
            let viseme = match sound.to_ascii_lowercase() {
                'm' | 'b' | 'p' => Viseme::Closed,
                'a' | 'h' => Viseme::Open,
                'e' | 'i' | 'y' => Viseme::Wide,
                'o' | 'u' | 'w' => Viseme::Round,
                _ => Viseme::Open,
            };
            cues.push(LipSyncCue { frame, viseme });
        }
    }
    cues.push(LipSyncCue {
        frame: (audio.duration_seconds * 30.0).ceil() as u32,
        viseme: Viseme::Rest,
    });
    cues
}

pub(crate) fn morph_for_scene(scene: &str) -> Option<MascotMorph> {
    let (label, silhouette) = match scene {
        "battery_hook" | "battery_anatomy" => (
            "battery",
            vec![
                [-0.85, -0.95],
                [-0.85, 0.75],
                [-0.3, 0.75],
                [-0.3, 0.95],
                [0.3, 0.95],
                [0.3, 0.75],
                [0.85, 0.75],
                [0.85, -0.95],
                [0.4, -0.95],
                [0.0, -0.95],
                [-0.4, -0.95],
            ],
        ),
        "battery_electrolyte" | "battery_charging" => (
            "ion",
            vec![
                [-0.65, -0.75],
                [-0.92, -0.35],
                [-1.0, 0.15],
                [-0.82, 0.62],
                [-0.42, 0.9],
                [0.0, 1.0],
                [0.42, 0.9],
                [0.82, 0.62],
                [1.0, 0.15],
                [0.92, -0.35],
                [0.65, -0.75],
            ],
        ),
        "battery_electrons" | "battery_discharging" | "battery_voltage" => (
            "energy bolt",
            vec![
                [-0.25, -1.0],
                [0.05, -0.25],
                [-0.4, -0.25],
                [0.25, 1.0],
                [0.05, 0.15],
                [0.5, 0.15],
                [-0.1, -1.0],
                [-0.25, -1.0],
                [-0.25, -1.0],
                [-0.25, -1.0],
                [-0.25, -1.0],
            ],
        ),
        "battery_aging" | "battery_safety" => (
            "warning sign",
            vec![
                [-1.0, -1.0],
                [-0.8, -0.6],
                [-0.6, -0.2],
                [-0.4, 0.2],
                [-0.2, 0.6],
                [0.0, 1.0],
                [0.2, 0.6],
                [0.4, 0.2],
                [0.6, -0.2],
                [0.8, -0.6],
                [1.0, -1.0],
            ],
        ),
        _ => return None,
    };
    Some(MascotMorph {
        label: label.into(),
        silhouette,
    })
}

pub(crate) fn write_captions(
    path: &Path,
    segments: &[NarrationSegment],
    narration: &NarrationTrack,
) -> anyhow::Result<()> {
    if segments.len() != narration.segments.len() {
        anyhow::bail!(
            "Caption segment count {} does not match narration segment count {}",
            segments.len(),
            narration.segments.len()
        );
    }

    let mut output = String::new();
    let mut start = 0.0_f32;
    for (index, (segment, audio)) in segments.iter().zip(narration.segments.iter()).enumerate() {
        let end = start + audio.duration_seconds;
        output.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            srt_timestamp(start),
            srt_timestamp(end),
            segment.text
        ));
        start = end;
    }
    std::fs::write(path, output)?;
    Ok(())
}

pub(crate) fn srt_timestamp(seconds: f32) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms / 60_000) % 60;
    let secs = (total_ms / 1_000) % 60;
    let millis = total_ms % 1_000;
    format!("{hours:02}:{minutes:02}:{secs:02},{millis:03}")
}
